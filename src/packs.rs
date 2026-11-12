//! Modpack exchange and content downloads. Imports are staged and never overwrite an instance.
use crate::{
    downloads,
    model::{self, Profile, SortOrder, Source},
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Digest;
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[default]
    Modrinth,
    CurseForge,
    ATLauncher,
    Ftb,
    Technic,
}
impl Provider {
    pub fn supports_sort(self, sort: SortOrder) -> bool {
        match self {
            Self::ATLauncher => sort != SortOrder::Downloads,
            Self::Technic => matches!(sort, SortOrder::Downloads | SortOrder::Relevance),
            _ => true,
        }
    }
    pub const ALL: [Self; 5] = [
        Self::Modrinth,
        Self::CurseForge,
        Self::ATLauncher,
        Self::Ftb,
        Self::Technic,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Modrinth => "Modrinth",
            Self::CurseForge => "CurseForge",
            Self::ATLauncher => "ATLauncher",
            Self::Ftb => "FTB",
            Self::Technic => "Technic",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub provider: Provider,
    pub website: String,
    pub icon_url: String,
    pub author: String,
    pub downloads: u64,
    pub updated: String,
    pub published: String,
}
#[derive(Clone, Debug)]
pub struct Version {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ResourceRecord {
    pub project: String,
    pub provider: Provider,
    pub version: String,
    pub name: String,
    pub path: String,
}
pub fn resources(instance: &Path) -> Vec<ResourceRecord> {
    fs::read(instance.join("launcher/resources.json"))
        .ok()
        .filter(|b| b.len() < 1024 * 1024)
        .and_then(|b| serde_json::from_slice::<Vec<ResourceRecord>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|r| game_path(&r.path).is_ok() && instance.join(&r.path).is_file())
        .collect()
}
pub fn cached_entries(path: &Path) -> Vec<Entry> {
    fs::read(path)
        .ok()
        .filter(|b| b.len() <= 512 * 1024)
        .and_then(|b| serde_json::from_slice::<Vec<Entry>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .take(20)
        .collect()
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FileSpec {
    pub path: String,
    pub url: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub sha512: String,
    #[serde(default)]
    pub md5: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub curseforge: Option<(String, String)>,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    format: u32,
    name: String,
    minecraft: String,
    loader: String,
    #[serde(default)]
    loader_version: String,
    #[serde(default)]
    files: Vec<FileSpec>,
}
pub struct Service {
    client: reqwest::blocking::Client,
    key: zeroize::Zeroizing<String>,
}
pub fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}
fn date_key(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|n| format!("{n:020}")))
        .unwrap_or_default()
}
fn sort_entries(entries: &mut [Entry], sort: SortOrder) {
    match sort {
        SortOrder::Downloads => entries.sort_by_key(|a| std::cmp::Reverse(a.downloads)),
        SortOrder::Updated => entries.sort_by(|a, b| b.updated.cmp(&a.updated)),
        SortOrder::Newest => entries.sort_by(|a, b| b.published.cmp(&a.published)),
        SortOrder::Relevance => {}
    }
}
fn catalog_name(project: &str, manifest: &str) -> String {
    if project.trim().is_empty() {
        manifest.to_owned()
    } else {
        project.trim().to_owned()
    }
}
fn array(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().context("Некорректный ответ каталога")
}
fn segment(s: &str) -> Result<&str> {
    anyhow::ensure!(
        !s.is_empty()
            && s.len() < 200
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c)),
        "Некорректный ID"
    );
    Ok(s)
}
fn https(url: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url)?;
    anyhow::ensure!(
        parsed.scheme() == "https" && parsed.username().is_empty() && parsed.password().is_none(),
        "Разрешён только HTTPS без пароля в URL"
    );
    let host = parsed.host_str().context("URL без сервера")?;
    anyhow::ensure!(
        host != "localhost" && !host.ends_with(".local") && !host.ends_with(".localhost"),
        "Локальные URL запрещены"
    );
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        let blocked = match ip {
            std::net::IpAddr::V4(a) => {
                a.is_private() || a.is_loopback() || a.is_link_local() || a.is_unspecified()
            }
            std::net::IpAddr::V6(a) => {
                a.is_loopback()
                    || a.is_unspecified()
                    || a.is_unique_local()
                    || a.is_unicast_link_local()
            }
        };
        anyhow::ensure!(!blocked, "Локальные URL запрещены");
    }
    Ok(parsed)
}

/// Portable path checks, including Windows aliases and NTFS alternate streams.
pub fn safe_path(name: &str) -> Result<PathBuf> {
    let relative = downloads::relative(name)?;
    for part in name.split('/') {
        anyhow::ensure!(
            !part.is_empty()
                && !part
                    .chars()
                    .any(|c| c.is_control() || "<>\"|?*".contains(c)),
            "Небезопасное имя файла"
        );
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        anyhow::ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.ends_with(['1', '2', '3', '4', '5', '6', '7', '8', '9'])),
            "Зарезервированное имя Windows"
        );
    }
    Ok(relative)
}
fn game_path(name: &str) -> Result<PathBuf> {
    let path = safe_path(name)?;
    let first = name.split('/').next().unwrap_or("").to_ascii_lowercase();
    anyhow::ensure!(
        !matches!(
            first.as_str(),
            "launcher"
                | "libraries"
                | "versions"
                | "assets"
                | "runtime"
                | "natives"
                | "bin"
                | "sessions.dat"
                | "api-keys.dat"
                | "config.json"
                | "accounts.json"
                | "launcher_accounts.json"
                | "launcher_profiles.json"
                | ".env"
                | "mlauncher-pack.json"
                | "mods.json"
        ),
        "Защищённый путь в сборке: {name}"
    );
    anyhow::ensure!(
        ![".exe", ".dll", ".bat", ".cmd", ".ps1", ".lnk", ".com", ".scr"]
            .iter()
            .any(|ext| name.to_ascii_lowercase().ends_with(ext)),
        "Исполняемый файл ОС в сборке запрещён"
    );
    Ok(path)
}

impl Service {
    pub fn new(key: String) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent("MLauncher/0.2.0")
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(180))
            .redirect(reqwest::redirect::Policy::custom(|a| {
                let key_request = a
                    .previous()
                    .first()
                    .is_some_and(|u| u.host_str() == Some("api.curseforge.com"));
                if https(a.url().as_str()).is_err()
                    || (key_request && a.url().host_str() != Some("api.curseforge.com"))
                    || a.previous().len() > 6
                {
                    a.error("Unsafe redirect")
                } else {
                    a.follow()
                }
            }))
            .build()?;
        Ok(Self {
            client,
            key: zeroize::Zeroizing::new(key),
        })
    }
    pub fn get(&self, url: &str, params: &[(&str, String)]) -> Result<Value> {
        self.get_with_timeout(url, params, std::time::Duration::from_secs(180))
    }
    fn get_with_timeout(
        &self,
        url: &str,
        params: &[(&str, String)],
        timeout: std::time::Duration,
    ) -> Result<Value> {
        let url = https(url)?;
        let mut request = self.client.get(url.clone()).query(params).timeout(timeout);
        if url.host_str() == Some("api.curseforge.com") {
            anyhow::ensure!(
                !self.key.is_empty(),
                "Нужен API-ключ CurseForge в настройках"
            );
            request = request.header("x-api-key", self.key.as_str());
        }
        let mut bytes = vec![];
        request
            .send()?
            .error_for_status()?
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() <= 32 * 1024 * 1024,
            "Слишком большие метаданные"
        );
        Ok(serde_json::from_slice(&bytes)?)
    }
    pub fn search(
        &self,
        provider: Provider,
        query: &str,
        resources: bool,
        mc: &str,
    ) -> Result<Vec<Entry>> {
        self.search_sorted(provider, query, resources, mc, SortOrder::Downloads)
    }
    pub fn search_sorted(
        &self,
        provider: Provider,
        query: &str,
        resources: bool,
        mc: &str,
        sort: SortOrder,
    ) -> Result<Vec<Entry>> {
        self.search_page(provider, query, resources, mc, sort, 0)
    }
    pub fn search_page(
        &self,
        provider: Provider,
        query: &str,
        resources: bool,
        mc: &str,
        sort: SortOrder,
        offset: usize,
    ) -> Result<Vec<Entry>> {
        self.search_catalog(provider, query, resources, false, mc, sort, offset)
    }

    pub fn search_maps(
        &self,
        query: &str,
        mc: &str,
        sort: SortOrder,
        offset: usize,
    ) -> Result<Vec<Entry>> {
        self.search_catalog(Provider::CurseForge, query, false, true, mc, sort, offset)
    }

    fn search_catalog(
        &self,
        provider: Provider,
        query: &str,
        resources: bool,
        maps: bool,
        mc: &str,
        sort: SortOrder,
        offset: usize,
    ) -> Result<Vec<Entry>> {
        // The other providers expose bounded catalogs rather than cursor searches.
        if offset > 0 && !matches!(provider, Provider::Modrinth | Provider::CurseForge) {
            return Ok(vec![]);
        }
        let mut entries = vec![];
        match provider {
            Provider::Modrinth => {
                let mut facets = vec![vec![format!(
                    "project_type:{}",
                    if resources { "resourcepack" } else { "modpack" }
                )]];
                if resources && query.is_empty() {
                    facets.push(vec![format!("versions:{mc}")]);
                }
                let v = self.get(
                    "https://api.modrinth.com/v2/search",
                    &[
                        ("query", query.into()),
                        ("limit", "20".into()),
                        ("offset", offset.to_string()),
                        ("index", sort.modrinth().into()),
                        ("facets", json!(facets).to_string()),
                    ],
                )?;
                for p in array(&v["hits"])? {
                    entries.push(Entry {
                        id: text(p, "project_id"),
                        name: text(p, "title"),
                        description: text(p, "description"),
                        icon_url: text(p, "icon_url"),
                        author: text(p, "author"),
                        downloads: p["downloads"].as_u64().unwrap_or(0),
                        updated: text(p, "date_modified"),
                        published: text(p, "date_created"),
                        provider,
                        website: format!(
                            "https://modrinth.com/{}/{}",
                            if resources { "resourcepack" } else { "modpack" },
                            text(p, "slug")
                        ),
                    });
                }
            }
            Provider::CurseForge => {
                let mut params = vec![
                    ("gameId", "432".into()),
                    (
                        "classId",
                        if maps {
                            "17"
                        } else if resources {
                            "12"
                        } else {
                            "4471"
                        }
                        .into(),
                    ),
                    ("searchFilter", query.into()),
                    ("pageSize", "20".into()),
                    ("index", offset.to_string()),
                    ("sortField", sort.curseforge().into()),
                    ("sortOrder", "desc".into()),
                ];
                if maps || resources && query.is_empty() {
                    params.push(("gameVersion", mc.into()));
                }
                let v = self.get("https://api.curseforge.com/v1/mods/search", &params)?;
                for p in array(&v["data"])? {
                    entries.push(Entry {
                        id: p["id"].to_string(),
                        name: text(p, "name"),
                        description: text(p, "summary"),
                        icon_url: text(&p["logo"], "thumbnailUrl"),
                        author: text(&p["authors"][0], "name"),
                        downloads: p["downloadCount"].as_f64().unwrap_or(0.) as u64,
                        updated: text(p, "dateModified"),
                        published: text(p, "dateCreated"),
                        provider,
                        website: text(&p["links"], "websiteUrl"),
                    });
                }
            }
            Provider::ATLauncher => {
                anyhow::ensure!(!resources, "У этого каталога нет ресурспаков");
                let v = self.get("https://api.atlauncher.com/v1/packs/simple", &[])?;
                for p in array(&v["data"])? {
                    if p["type"] != "public" {
                        continue;
                    }
                    let name = text(p, "name");
                    if !name.to_lowercase().contains(&query.to_lowercase()) {
                        continue;
                    }
                    entries.push(Entry {
                        id: text(p, "safeName"),
                        name,
                        description: "Публичная сборка ATLauncher".into(),
                        icon_url: format!(
                            "https://cdn.atlcdn.net/images/packs/{}.png",
                            text(p, "safeName").to_lowercase()
                        ),
                        author: String::new(),
                        downloads: 0,
                        updated: String::new(),
                        published: String::new(),
                        provider,
                        website: format!("https://atlauncher.com/pack/{}", text(p, "safeName")),
                    });
                    if entries.len() >= 20 {
                        break;
                    }
                }
            }
            Provider::Ftb => {
                anyhow::ensure!(!resources, "У этого каталога нет ресурспаков");
                let v = self.get(
                    "https://api.feed-the-beast.com/v1/modpacks/public/modpack/search/10",
                    &[("term", query.into())],
                )?;
                for id in array(&v["packs"])? {
                    let id = id.to_string();
                    let p = self.get(
                        &format!(
                            "https://api.feed-the-beast.com/v1/modpacks/public/modpack/{}",
                            segment(&id)?
                        ),
                        &[],
                    )?;
                    entries.push(Entry {
                        id,
                        name: text(&p, "name"),
                        description: text(&p, "synopsis"),
                        icon_url: p["art"]
                            .as_array()
                            .and_then(|art| {
                                art.iter()
                                    .find(|a| a["type"] == "square")
                                    .or_else(|| art.first())
                            })
                            .map(|a| text(a, "url"))
                            .unwrap_or_default(),
                        author: text(&p["authors"][0], "name"),
                        downloads: p["installs"].as_u64().unwrap_or(0),
                        updated: date_key(&p["updated"]),
                        published: date_key(&p["released"]),
                        provider,
                        website: format!(
                            "https://www.feed-the-beast.com/modpacks/{}",
                            text(&p, "slug")
                        ),
                    });
                }
            }
            Provider::Technic => {
                anyhow::ensure!(!resources, "У этого каталога нет ресурспаков");
                let v = self.get(
                    "https://api.technicpack.net/search",
                    &[("build", "999".into()), ("q", query.into())],
                )?;
                for p in array(&v["modpacks"])? {
                    entries.push(Entry {
                        id: text(p, "slug"),
                        name: text(p, "name"),
                        description: "Сборка Technic".into(),
                        icon_url: text(p, "iconUrl"),
                        author: String::new(),
                        downloads: 0,
                        updated: String::new(),
                        published: String::new(),
                        provider,
                        website: text(p, "url"),
                    });
                }
            }
        }
        if matches!(provider, Provider::ATLauncher | Provider::Technic) {
            // These search APIs omit descriptions. Enrich a bounded page in parallel.
            std::thread::scope(|scope| {
                for chunk in entries.chunks_mut(5) {
                    scope.spawn(move || {
                        for entry in chunk {
                            if segment(&entry.id).is_err() {
                                continue;
                            }
                            let url = if provider == Provider::ATLauncher {
                                format!("https://api.atlauncher.com/v1/pack/{}", entry.id)
                            } else {
                                format!("https://api.technicpack.net/modpack/{}", entry.id)
                            };
                            let params = if provider == Provider::Technic {
                                vec![("build", "999".into())]
                            } else {
                                vec![]
                            };
                            if let Ok(value) = self.get_with_timeout(
                                &url,
                                &params,
                                std::time::Duration::from_secs(10),
                            ) {
                                let value = if provider == Provider::ATLauncher {
                                    &value["data"]
                                } else {
                                    &value
                                };
                                let description = text(value, "description");
                                if !description.trim().is_empty() {
                                    entry.description = description;
                                }
                                if provider == Provider::Technic {
                                    entry.author = text(value, "user");
                                    entry.downloads = value["installs"].as_u64().unwrap_or(0);
                                } else if let Some(versions) = value["versions"].as_array() {
                                    entry.updated = versions
                                        .iter()
                                        .map(|v| date_key(&v["published"]))
                                        .max()
                                        .unwrap_or_default();
                                    entry.published = versions
                                        .iter()
                                        .map(|v| date_key(&v["published"]))
                                        .filter(|d| !d.is_empty())
                                        .min()
                                        .unwrap_or_default();
                                }
                            }
                        }
                    });
                }
            });
        }
        if !matches!(provider, Provider::Modrinth | Provider::CurseForge) {
            sort_entries(&mut entries, sort);
        }
        Ok(entries)
    }
    pub fn versions(&self, e: &Entry, resources: bool, mc: &str) -> Result<Vec<Version>> {
        segment(&e.id)?;
        let v = match e.provider {
            Provider::Modrinth => self.get(
                &format!("https://api.modrinth.com/v2/project/{}/version", e.id),
                &if resources {
                    vec![("game_versions", json!([mc]).to_string())]
                } else {
                    vec![]
                },
            )?,
            Provider::CurseForge => self.get(
                &format!("https://api.curseforge.com/v1/mods/{}/files", e.id),
                &if resources {
                    vec![("gameVersion", mc.into()), ("pageSize", "50".into())]
                } else {
                    vec![("pageSize", "50".into())]
                },
            )?,
            Provider::ATLauncher => {
                self.get(&format!("https://api.atlauncher.com/v1/pack/{}", e.id), &[])?
            }
            Provider::Ftb => self.get(
                &format!(
                    "https://api.feed-the-beast.com/v1/modpacks/public/modpack/{}",
                    e.id
                ),
                &[],
            )?,
            Provider::Technic => self.get(
                &format!("https://api.technicpack.net/modpack/{}", e.id),
                &[("build", "999".into())],
            )?,
        };
        let mut result = vec![];
        match e.provider {
            Provider::Modrinth => {
                for p in array(&v)? {
                    result.push(Version {
                        id: text(p, "id"),
                        name: format!(
                            "{} · {}",
                            text(p, "version_number"),
                            p["game_versions"]
                                .as_array()
                                .map(|a| a
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", "))
                                .unwrap_or_default()
                        ),
                    });
                }
            }
            Provider::CurseForge => {
                for p in array(&v["data"])? {
                    if p["isAvailable"] == false {
                        continue;
                    }
                    result.push(Version {
                        id: p["id"].to_string(),
                        name: text(p, "displayName"),
                    });
                }
            }
            Provider::ATLauncher => {
                for p in array(&v["data"]["versions"])? {
                    result.push(Version {
                        id: text(p, "version"),
                        name: format!("{} · {}", text(p, "version"), text(p, "minecraft")),
                    });
                }
            }
            Provider::Ftb => {
                for p in array(&v["versions"])?.iter().rev() {
                    if p["private"] == true || p["type"] == "archived" {
                        continue;
                    }
                    result.push(Version {
                        id: p["id"].to_string(),
                        name: text(p, "name"),
                    });
                }
            }
            Provider::Technic => {
                let solder = text(&v, "solder");
                if solder.is_empty() {
                    result.push(Version {
                        id: text(&v, "version"),
                        name: text(&v, "version"),
                    });
                } else {
                    let s = self.get(
                        &format!(
                            "{}modpack/{}",
                            solder.trim_end_matches('/').to_owned() + "/",
                            e.id
                        ),
                        &[],
                    )?;
                    for build in array(&s["builds"])?.iter().rev() {
                        if let Some(b) = build.as_str() {
                            result.push(Version {
                                id: b.into(),
                                name: b.into(),
                            });
                        }
                    }
                }
            }
        }
        Ok(result)
    }
    fn mr_file(&self, version: &str) -> Result<FileSpec> {
        let v = self.get(
            &format!("https://api.modrinth.com/v2/version/{}", segment(version)?),
            &[],
        )?;
        let files = array(&v["files"])?;
        let f = files
            .iter()
            .find(|f| f["primary"] == true)
            .or_else(|| files.first())
            .context("Нет файла")?;
        Ok(FileSpec {
            path: text(f, "filename"),
            url: text(f, "url"),
            sha1: text(&f["hashes"], "sha1"),
            sha512: text(&f["hashes"], "sha512"),
            size: f["size"].as_u64(),
            ..Default::default()
        })
    }
    fn cf_file(&self, project: &str, version: &str) -> Result<FileSpec> {
        let v = self.get(
            &format!(
                "https://api.curseforge.com/v1/mods/{}/files/{}",
                segment(project)?,
                segment(version)?
            ),
            &[],
        )?;
        let f = &v["data"];
        let url = text(f, "downloadUrl");
        anyhow::ensure!(!url.is_empty(),"Автор запретил стороннее скачивание CurseForge: проект {project}, файл {version}. Скачайте разрешённый архив вручную.");
        let hashes = array(&f["hashes"])?;
        Ok(FileSpec {
            path: text(f, "fileName"),
            url,
            sha1: hashes
                .iter()
                .find(|h| h["algo"] == 1)
                .map(|h| text(h, "value"))
                .unwrap_or_default(),
            md5: hashes
                .iter()
                .find(|h| h["algo"] == 2)
                .map(|h| text(h, "value"))
                .unwrap_or_default(),
            size: f["fileLength"].as_u64(),
            curseforge: Some((project.into(), version.into())),
            ..Default::default()
        })
    }
    pub fn fetch(
        &self,
        spec: &FileSpec,
        path: &Path,
        cancel: &AtomicBool,
        allow_unhashed: bool,
    ) -> Result<()> {
        downloads::check_cancel(cancel)?;
        anyhow::ensure!(
            allow_unhashed
                || ![&spec.sha1, &spec.sha256, &spec.sha512, &spec.md5]
                    .iter()
                    .all(|s| s.is_empty()),
            "Нет контрольной суммы для {}",
            spec.path
        );
        if path.exists() {
            anyhow::ensure!(
                existing_matches(path, spec)?,
                "Файл уже существует, но отличается от скачиваемого: {}. Переименуйте его вручную.",
                spec.path
            );
            return Ok(());
        }
        let mut response = self
            .client
            .get(https(&spec.url)?)
            .send()?
            .error_for_status()?;
        let parent = path.parent().context("Нет папки")?;
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        let (mut sha1, mut sha256, mut sha512, mut md5) = (
            sha1::Sha1::new(),
            sha2::Sha256::new(),
            sha2::Sha512::new(),
            md5::Md5::new(),
        );
        let mut size = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            downloads::check_cancel(cancel)?;
            let n = response.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            size += n as u64;
            anyhow::ensure!(size <= 1024 * 1024 * 1024, "Файл больше 1 ГиБ");
            sha1.update(&buffer[..n]);
            sha256.update(&buffer[..n]);
            sha512.update(&buffer[..n]);
            md5.update(&buffer[..n]);
            file.write_all(&buffer[..n])?;
        }
        for (expected, actual) in [
            (&spec.sha1, format!("{:x}", sha1.finalize())),
            (&spec.sha256, format!("{:x}", sha256.finalize())),
            (&spec.sha512, format!("{:x}", sha512.finalize())),
            (&spec.md5, format!("{:x}", md5.finalize())),
        ] {
            anyhow::ensure!(
                expected.is_empty() || expected.eq_ignore_ascii_case(&actual),
                "Хеш не совпал: {}",
                spec.path
            );
        }
        anyhow::ensure!(
            spec.size.is_none_or(|n| n == size),
            "Размер не совпал: {}",
            spec.path
        );
        file.as_file().sync_all()?;
        downloads::check_cancel(cancel)?;
        file.persist_noclobber(path)?;
        Ok(())
    }
    pub fn download_map(
        &self,
        entry: &Entry,
        mc: &str,
        instance: &Path,
        cancel: &AtomicBool,
    ) -> Result<()> {
        anyhow::ensure!(
            entry.provider == Provider::CurseForge,
            "Карты доступны в CurseForge"
        );
        segment(&entry.id)?;
        let saves = instance.join("saves");
        fs::create_dir_all(&saves)?;
        anyhow::ensure!(
            saves.canonicalize()?.parent() == Some(instance.canonicalize()?.as_path()),
            "Папка миров находится вне сборки"
        );
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(saves.join(".mlauncher-maps.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock).context("Уже скачивается другая карта")?;
        let destination = map_path(instance, entry);
        anyhow::ensure!(!destination.exists(), "Карта уже есть в папке миров");
        let versions = self.versions(entry, true, mc)?;
        let version = versions.first().context("Нет совместимой версии карты")?;
        let spec = self.cf_file(&entry.id, &version.id)?;
        anyhow::ensure!(
            spec.path.to_ascii_lowercase().ends_with(".zip"),
            "Поддерживаются карты в ZIP"
        );
        let staged = tempfile::tempdir_in(&saves)?;
        let archive = staged.path().join("map.zip");
        self.fetch(&spec, &archive, cancel, false)?;
        install_map_archive(&archive, staged.path(), &destination, cancel)
    }

    pub fn resource(
        &self,
        e: &Entry,
        version: &str,
        instance: &Path,
        cancel: &AtomicBool,
    ) -> Result<()> {
        fs::create_dir_all(instance.join("launcher"))?;
        let resource_lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(instance.join("launcher/.resources.lock"))?;
        fs2::FileExt::try_lock_exclusive(&resource_lock)
            .context("В этой сборке уже скачивается другой ресурспак. Дождитесь завершения.")?;
        let mut spec = match e.provider {
            Provider::Modrinth => self.mr_file(version)?,
            Provider::CurseForge => self.cf_file(&e.id, version)?,
            _ => bail!("Нет ресурспаков"),
        };
        anyhow::ensure!(
            spec.path.to_lowercase().ends_with(".zip"),
            "Ресурспак должен быть ZIP"
        );
        safe_path(&spec.path)?;
        anyhow::ensure!(!spec.path.contains('/'), "Имя ресурспака");
        let path = instance.join("resourcepacks").join(&spec.path);
        self.fetch(&spec, &path, cancel, false)?;
        spec.path = format!("resourcepacks/{}", spec.path);
        let mut refs = read_refs(instance);
        refs.retain(|f| f.path != spec.path);
        refs.push(spec);
        downloads::atomic_write(
            &instance.join("launcher/pack-files.json"),
            &serde_json::to_vec_pretty(&refs)?,
        )?;
        let mut records = resources(instance);
        records.retain(|r| r.project != e.id || r.provider != e.provider);
        records.push(ResourceRecord {
            project: e.id.clone(),
            provider: e.provider,
            version: version.into(),
            name: e.name.clone(),
            path: format!(
                "resourcepacks/{}",
                path.file_name().context("Нет имени")?.to_string_lossy()
            ),
        });
        downloads::atomic_write(
            &instance.join("launcher/resources.json"),
            &serde_json::to_vec_pretty(&records)?,
        )?;
        Ok(())
    }
    pub fn install(
        &self,
        e: &Entry,
        version: &str,
        instances: &Path,
        cancel: &AtomicBool,
        progress: &impl Fn(String),
    ) -> Result<Profile> {
        fs::create_dir_all(instances)?;
        let stage = tempfile::tempdir_in(instances)?;
        let destination = stage.path().join("game");
        fs::create_dir(&destination)?;
        progress("Чтение манифеста сборки…".into());
        let mut manifest = match e.provider {
            Provider::Modrinth | Provider::CurseForge => {
                let spec = if e.provider == Provider::Modrinth {
                    self.mr_file(version)?
                } else {
                    self.cf_file(&e.id, version)?
                };
                let archive = stage.path().join("pack.zip");
                self.fetch(&spec, &archive, cancel, false)?;
                self.unpack(&archive, &destination, cancel, progress)?
            }
            Provider::Ftb => self.ftb(e, version, &destination, cancel, progress)?,
            Provider::ATLauncher => self.atlauncher(e, version, &destination, cancel, progress)?,
            Provider::Technic => self.technic(e, version, &destination, cancel, progress)?,
        };
        manifest.name = catalog_name(&e.name, &manifest.name);
        downloads::atomic_write(
            &destination.join("launcher/pack-info.json"),
            &serde_json::to_vec_pretty(
                &json!({"name":manifest.name,"project":e.id,"source":e.provider.label(),"version":version}),
            )?,
        )?;
        finish(manifest, &destination, instances, cancel)
    }
    pub fn import(
        &self,
        archive: &Path,
        instances: &Path,
        cancel: &AtomicBool,
        progress: &impl Fn(String),
    ) -> Result<Profile> {
        fs::create_dir_all(instances)?;
        let stage = tempfile::tempdir_in(instances)?;
        let destination = stage.path().join("game");
        fs::create_dir(&destination)?;
        let manifest = self.unpack(archive, &destination, cancel, progress)?;
        finish(manifest, &destination, instances, cancel)
    }
    fn download_plan(
        &self,
        files: &[FileSpec],
        destination: &Path,
        cancel: &AtomicBool,
        progress: &impl Fn(String),
    ) -> Result<()> {
        anyhow::ensure!(files.len() <= 30000, "Слишком много файлов");
        let mut names = HashSet::new();
        let mut total = 0u64;
        for (i, file) in files.iter().enumerate() {
            let path = game_path(&file.path)?;
            anyhow::ensure!(
                names.insert(file.path.to_lowercase()),
                "Повтор пути {}",
                file.path
            );
            progress(format!("Файл {}/{}: {}", i + 1, files.len(), file.path));
            let mut resolved = file.clone();
            if let Some((project, version)) = &file.curseforge {
                let current = self.cf_file(project, version)?;
                resolved.url = current.url;
            }
            self.fetch(&resolved, &destination.join(path), cancel, false)?;
            total += destination.join(game_path(&file.path)?).metadata()?.len();
            anyhow::ensure!(total <= 12 * 1024 * 1024 * 1024, "Сборка превышает 12 ГиБ");
        }
        Ok(())
    }
    fn unpack(
        &self,
        archive: &Path,
        destination: &Path,
        cancel: &AtomicBool,
        progress: &impl Fn(String),
    ) -> Result<Manifest> {
        let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
        anyhow::ensure!(zip.len() <= 100000, "Слишком много файлов ZIP");
        // Validate all paths before making any requests from an untrusted index.
        let mut archive_paths = HashSet::new();
        let mut archive_size = 0u64;
        for i in 0..zip.len() {
            downloads::check_cancel(cancel)?;
            let entry = zip.by_index(i)?;
            safe_path(entry.name().trim_end_matches('/'))?;
            anyhow::ensure!(
                !entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000),
                "Ссылки ZIP запрещены"
            );
            anyhow::ensure!(
                archive_paths.insert(entry.name().to_lowercase()),
                "Повтор пути в ZIP"
            );
            archive_size += entry.size();
            anyhow::ensure!(
                archive_size <= 12 * 1024 * 1024 * 1024 && entry.size() <= 1024 * 1024 * 1024,
                "Превышен лимит ZIP"
            );
        }
        let has_own = zip.file_names().any(|s| s == "mlauncher-pack.json");
        let has_mr = zip.file_names().any(|s| s == "modrinth.index.json");
        let (mut manifest, prefix) = if has_own {
            let m: Manifest = serde_json::from_value(zip_json(&mut zip, "mlauncher-pack.json")?)?;
            anyhow::ensure!(m.format == 1, "Неизвестный формат сборки");
            (m, "files/".to_owned())
        } else if has_mr {
            let v = zip_json(&mut zip, "modrinth.index.json")?;
            anyhow::ensure!(
                v["game"] == "minecraft" && v["formatVersion"] == 1,
                "Неподдерживаемый mrpack"
            );
            let (loader, loader_version) = dependency_loader(&v["dependencies"]);
            let mut files = vec![];
            for f in array(&v["files"])? {
                if f["env"]["client"] == "unsupported" {
                    continue;
                }
                files.push(FileSpec {
                    path: text(f, "path"),
                    url: f["downloads"][0]
                        .as_str()
                        .context("Нет URL файла mrpack")?
                        .into(),
                    sha1: text(&f["hashes"], "sha1"),
                    sha512: text(&f["hashes"], "sha512"),
                    size: f["fileSize"].as_u64(),
                    ..Default::default()
                });
            }
            (
                Manifest {
                    format: 1,
                    name: text(&v, "name"),
                    minecraft: text(&v["dependencies"], "minecraft"),
                    loader,
                    loader_version,
                    files,
                },
                "overrides/".into(),
            )
        } else {
            let v = zip_json(&mut zip, "manifest.json")?;
            anyhow::ensure!(
                v["manifestType"] == "minecraftModpack" && v["manifestVersion"] == 1,
                "Неизвестный формат ZIP: нужен MLauncher, mrpack или CurseForge"
            );
            let loaders = array(&v["minecraft"]["modLoaders"])?;
            let id = loaders
                .iter()
                .find(|l| l["primary"] == true)
                .or_else(|| loaders.first())
                .map(|l| text(l, "id"))
                .unwrap_or_default();
            let (loader, loader_version) = parse_loader(&id);
            let mut files = vec![];
            for f in array(&v["files"])? {
                if f["required"] == false {
                    continue;
                }
                let spec = self.cf_file(&f["projectID"].to_string(), &f["fileID"].to_string())?;
                let mut spec = spec;
                spec.path = format!("mods/{}", spec.path);
                files.push(spec);
            }
            let prefix = format!("{}/", v["overrides"].as_str().unwrap_or("overrides"));
            safe_path(prefix.trim_end_matches('/'))?;
            (
                Manifest {
                    format: 1,
                    name: text(&v, "name"),
                    minecraft: text(&v["minecraft"], "version"),
                    loader,
                    loader_version,
                    files,
                },
                prefix,
            )
        };
        validate_manifest(&manifest)?;
        self.download_plan(&manifest.files, destination, cancel, progress)?;
        let mut total = 0u64;
        let mut names = HashSet::new();
        // Client overrides intentionally replace common overrides, but not downloaded mods.
        for root in [
            &prefix[..],
            if has_mr {
                "client-overrides/"
            } else {
                "__none__/"
            },
        ] {
            for i in 0..zip.len() {
                downloads::check_cancel(cancel)?;
                let mut entry = zip.by_index(i)?;
                let raw = entry.name().to_owned();
                safe_path(raw.trim_end_matches('/'))?;
                anyhow::ensure!(
                    !entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000),
                    "Ссылки ZIP запрещены"
                );
                if entry.is_dir() {
                    continue;
                }
                let Some(name) = raw.strip_prefix(root) else {
                    continue;
                };
                let relative = game_path(name)?;
                total += entry.size();
                anyhow::ensure!(
                    entry.size() <= 1024 * 1024 * 1024 && total <= 12 * 1024 * 1024 * 1024,
                    "Превышен лимит распаковки"
                );
                anyhow::ensure!(
                    names.insert(format!("{root}{}", name.to_lowercase())),
                    "Дублирующийся путь в ZIP"
                );
                let path = destination.join(relative);
                if root == prefix && path.exists() {
                    bail!("Конфликт файла из индекса и ZIP: {name}");
                }
                fs::create_dir_all(path.parent().context("Нет папки")?)?;
                let mut file = File::create(&path)?;
                let mut buffer = [0u8; 65536];
                loop {
                    downloads::check_cancel(cancel)?;
                    let n = entry.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    file.write_all(&buffer[..n])?;
                }
            }
        }
        if manifest.name.trim().is_empty() {
            manifest.name = "Импортированная сборка".into();
        }
        Ok(manifest)
    }
    fn ftb(
        &self,
        e: &Entry,
        version: &str,
        destination: &Path,
        cancel: &AtomicBool,
        progress: &impl Fn(String),
    ) -> Result<Manifest> {
        let v = self.get(
            &format!(
                "https://api.feed-the-beast.com/v1/modpacks/public/modpack/{}/{}",
                segment(&e.id)?,
                segment(version)?
            ),
            &[],
        )?;
        anyhow::ensure!(v["status"] == "success", "FTB не вернул манифест");
        let targets = array(&v["targets"])?;
        let mc = targets
            .iter()
            .find(|t| t["name"] == "minecraft")
            .context("FTB: нет Minecraft")?;
        let loader = targets.iter().find(|t| t["type"] == "modloader");
        let mut files = vec![];
        for f in array(&v["files"])? {
            if f["serveronly"] == true || f["optional"] == true {
                continue;
            }
            let path = text(f, "path");
            let path = path.strip_prefix("./").unwrap_or(&path).trim_matches('/');
            let name = text(f, "name");
            let relative = if path.is_empty() || path == "." {
                name
            } else {
                format!("{path}/{name}")
            };
            files.push(FileSpec {
                path: relative,
                url: text(f, "url"),
                sha1: text(f, "sha1"),
                sha256: text(&f["hashes"], "sha256"),
                size: f["size"].as_u64(),
                ..Default::default()
            });
        }
        self.download_plan(&files, destination, cancel, progress)?;
        Ok(Manifest {
            format: 1,
            name: e.name.clone(),
            minecraft: text(mc, "version"),
            loader: loader
                .map(|l| text(l, "name"))
                .unwrap_or_else(|| "vanilla".into()),
            loader_version: loader.map(|l| text(l, "version")).unwrap_or_default(),
            files,
        })
    }
    fn atlauncher(
        &self,
        e: &Entry,
        version: &str,
        destination: &Path,
        cancel: &AtomicBool,
        progress: &impl Fn(String),
    ) -> Result<Manifest> {
        let base = format!(
            "https://download.nodecdn.net/containers/atl/packs/{}/versions/{}",
            segment(&e.id)?,
            segment(version)?
        );
        let v = self.get(&format!("{base}/Configs.json"), &[])?;
        let mc = text(&v, "minecraft");
        let mc = if mc.is_empty() {
            text(&v, "minecraftVersion")
        } else {
            mc
        };
        let loader_name = text(&v["loader"], "type").to_lowercase();
        let loader_version = v["loader"]["metadata"]["loader"]
            .as_str()
            .or_else(|| v["loader"]["metadata"]["version"].as_str())
            .or_else(|| v["loader"]["version"].as_str())
            .unwrap_or_default()
            .to_owned();
        let mut files = vec![];
        for f in array(&v["mods"])? {
            if f["client"] == false
                || f["optional"] == true && f["selected"] != true && f["recommended"] != true
            {
                continue;
            }
            let kind = text(f, "type").to_lowercase();
            let method = text(f, "download").to_lowercase();
            anyhow::ensure!(
                !matches!(method.as_str(), "browser" | "manual"),
                "ATLauncher: {} требует ручного скачивания; автоматический обход запрещён",
                text(f, "name")
            );
            anyhow::ensure!(matches!(kind.as_str(),"mods"|"mod"|"resourcepack"|"resourcepacks"),"ATLauncher: неподдерживаемый тип {kind}; установка остановлена без изменения сборок");
            let folder = if kind.starts_with("resource") {
                "resourcepacks"
            } else {
                "mods"
            };
            let filename = text(f, "file");
            let url = text(f, "url");
            let url = if method == "server" && !url.starts_with("https://") {
                safe_path(&url)?;
                format!("https://download.nodecdn.net/containers/atl/{url}")
            } else {
                url
            };
            files.push(FileSpec {
                path: format!("{folder}/{filename}"),
                url,
                md5: text(f, "md5"),
                sha1: text(f, "sha1"),
                size: f["filesize"].as_u64(),
                ..Default::default()
            });
        }
        self.download_plan(&files, destination, cancel, progress)?;
        let archive = destination
            .parent()
            .context("Нет staging")?
            .join("configs.zip");
        self.fetch(
            &FileSpec {
                path: "Configs.zip".into(),
                url: format!("{base}/Configs.zip"),
                sha1: text(&v["configs"], "sha1"),
                size: v["configs"]["filesize"].as_u64(),
                ..Default::default()
            },
            &archive,
            cancel,
            false,
        )?;
        extract_game_zip(&archive, destination, cancel)?;
        Ok(Manifest {
            format: 1,
            name: e.name.clone(),
            minecraft: mc,
            loader: if loader_name.is_empty() {
                "vanilla".into()
            } else {
                loader_name
            },
            loader_version,
            files,
        })
    }
    fn technic(
        &self,
        e: &Entry,
        version: &str,
        destination: &Path,
        cancel: &AtomicBool,
        progress: &impl Fn(String),
    ) -> Result<Manifest> {
        let p = self.get(
            &format!("https://api.technicpack.net/modpack/{}", segment(&e.id)?),
            &[("build", "999".into())],
        )?;
        let solder = text(&p, "solder");
        anyhow::ensure!(!solder.is_empty(),"Technic ZIP без Solder не содержит надёжных метаданных загрузчика. Импортируйте экспорт mrpack/CurseForge; сборка не добавлена.");
        let base = solder.trim_end_matches('/');
        let v = self.get(
            &format!("{base}/modpack/{}/{}", segment(&e.id)?, segment(version)?),
            &[],
        )?;
        let mods = array(&v["mods"])?;
        let loader = mods
            .iter()
            .find(|m| m["name"] == "forge")
            .map(|m| text(m, "version"))
            .or_else(|| v["forge"].as_str().map(str::to_owned))
            .context("Technic: нет версии Forge. Старые патченные сборки пока не поддерживаются")?;
        segment(&loader)?;
        for (i, m) in array(&v["mods"])?.iter().enumerate() {
            // Install the declared Forge version via its official installer, not bin/modpack.jar.
            if m["name"] == "forge" {
                continue;
            }
            downloads::check_cancel(cancel)?;
            progress(format!("Technic: {}", text(m, "name")));
            let zip = destination
                .parent()
                .context("Нет staging")?
                .join(format!("technic-{i}.zip"));
            self.fetch(
                &FileSpec {
                    path: format!("technic-{i}.zip"),
                    url: text(m, "url"),
                    md5: text(m, "md5"),
                    ..Default::default()
                },
                &zip,
                cancel,
                false,
            )?;
            extract_game_zip(&zip, destination, cancel)?;
        }
        // Old Solder packs embed patched Minecraft/Forge jars. Do not silently install as Vanilla.
        anyhow::ensure!(
            !destination.join("bin").exists(),
            "Technic: старый modpack.jar пока не поддерживается"
        );
        Ok(Manifest {
            format: 1,
            name: e.name.clone(),
            minecraft: text(&v, "minecraft"),
            loader: "forge".into(),
            loader_version: loader,
            files: vec![],
        })
    }
    pub fn export(
        &self,
        profile: &Profile,
        instance: &Path,
        target: &Path,
        indexed: bool,
        cancel: &AtomicBool,
        progress: &impl Fn(String),
    ) -> Result<()> {
        let mut paths = vec![];
        collect_export(instance, instance, &mut paths)?;
        let refs = read_refs(instance);
        let tracked = model::installed(instance)?;
        let mut manifest = Manifest {
            format: 1,
            name: profile.name.clone(),
            minecraft: profile.game_version.clone(),
            loader: profile.loader.clone(),
            loader_version: read_loader(instance),
            files: vec![],
        };
        let mut embedded = vec![];
        for (relative, path) in paths {
            downloads::check_cancel(cancel)?;
            if indexed
                && (relative.starts_with("mods/")
                    || refs.iter().any(|f| f.path == relative)
                        && (relative.starts_with("resourcepacks/")
                            || relative.starts_with("shaderpacks/")))
            {
                progress(format!("Индексирование: {relative}"));
                let sha256 = file_sha256(&path)?;
                let mut spec = refs.iter().find(|f| f.path == relative).cloned();
                if spec.is_none() {
                    if let Some(item) = tracked
                        .iter()
                        .find(|m| relative == format!("mods/{}", m.filename))
                    {
                        spec = Some(match item.source {
                            Source::Modrinth => self.mr_file(&item.version_id)?,
                            Source::CurseForge => {
                                self.cf_file(&item.project_id, &item.version_id)?
                            }
                        });
                    } else {
                        let sha512 = file_sha512(&path)?;
                        let v=self.get(&format!("https://api.modrinth.com/v2/version_file/{sha512}"),&[("algorithm","sha512".into())]).with_context(||format!("Для {relative} нет источника скачивания. Используйте полный экспорт; файл не будет потерян."))?;
                        let f = array(&v["files"])?
                            .iter()
                            .find(|f| f["hashes"]["sha512"] == sha512)
                            .context("Нет точного файла")?;
                        spec = Some(FileSpec {
                            url: text(f, "url"),
                            sha512,
                            ..Default::default()
                        });
                    }
                }
                let mut spec = spec.context("Нет источника")?;
                // Verify origin against current local bytes, including user-modified mods.
                let known = if !spec.sha256.is_empty() {
                    spec.sha256.eq_ignore_ascii_case(&sha256)
                } else if !spec.sha512.is_empty() {
                    spec.sha512.eq_ignore_ascii_case(&file_sha512(&path)?)
                } else if !spec.sha1.is_empty() {
                    spec.sha1.eq_ignore_ascii_case(&file_sha1(&path)?)
                } else if !spec.md5.is_empty() {
                    spec.md5
                        .eq_ignore_ascii_case(&digest_file::<md5::Md5>(&path)?)
                } else {
                    false
                };
                anyhow::ensure!(
                    known,
                    "{} изменён или не имеет проверяемого источника. Используйте полный экспорт.",
                    relative
                );
                spec.path = relative;
                spec.sha256 = sha256;
                spec.size = Some(path.metadata()?.len());
                manifest.files.push(spec);
            } else {
                embedded.push((relative, path));
            }
        }
        let parent = target.parent().context("Нет папки экспорта")?;
        let temp = tempfile::NamedTempFile::new_in(parent)?;
        let mut zip = zip::ZipWriter::new(temp.reopen()?);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("mlauncher-pack.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        for (relative, path) in embedded {
            downloads::check_cancel(cancel)?;
            progress(format!("В архив: {relative}"));
            zip.start_file(format!("files/{relative}"), options)?;
            let mut file = File::open(path)?;
            let mut buffer = [0u8; 65536];
            loop {
                downloads::check_cancel(cancel)?;
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                zip.write_all(&buffer[..n])?;
            }
        }
        zip.finish()?.sync_all()?;
        downloads::check_cancel(cancel)?;
        temp.persist(target)?;
        Ok(())
    }
}

fn zip_json(zip: &mut zip::ZipArchive<File>, name: &str) -> Result<Value> {
    let entry = zip.by_name(name)?;
    anyhow::ensure!(entry.size() <= 16 * 1024 * 1024, "Манифест слишком большой");
    let mut bytes = vec![];
    entry.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
fn dependency_loader(v: &Value) -> (String, String) {
    for (key, loader) in [
        ("fabric-loader", "fabric"),
        ("quilt-loader", "quilt"),
        ("forge", "forge"),
        ("neoforge", "neoforge"),
    ] {
        if let Some(version) = v[key].as_str() {
            return (loader.into(), version.into());
        }
    }
    ("vanilla".into(), String::new())
}
fn parse_loader(id: &str) -> (String, String) {
    for loader in ["neoforge", "forge", "fabric", "quilt"] {
        if let Some(version) = id.strip_prefix(&format!("{loader}-")) {
            return (loader.into(), version.into());
        }
    }
    ("vanilla".into(), String::new())
}
fn validate_manifest(m: &Manifest) -> Result<()> {
    segment(&m.minecraft)?;
    anyhow::ensure!(
        matches!(
            m.loader.as_str(),
            "vanilla" | "fabric" | "quilt" | "forge" | "neoforge"
        ),
        "Неизвестный загрузчик {}",
        m.loader
    );
    if !m.loader_version.is_empty() {
        segment(&m.loader_version)?;
    }
    Ok(())
}
fn finish(m: Manifest, staged: &Path, instances: &Path, cancel: &AtomicBool) -> Result<Profile> {
    validate_manifest(&m)?;
    downloads::check_cancel(cancel)?;
    downloads::atomic_write(
        &staged.join("launcher/pack-files.json"),
        &serde_json::to_vec_pretty(&m.files)?,
    )?;
    downloads::atomic_write(
        &staged.join("launcher/pack-loader.json"),
        &serde_json::to_vec(&json!({"loader":m.loader,"version":m.loader_version}))?,
    )?;
    let used = fs::read_dir(instances)?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Vec<_>>();
    let directory = model::unique_directory(&m.name, used.iter().map(String::as_str));
    let target = instances.join(&directory);
    anyhow::ensure!(!target.exists(), "Папка сборки уже существует");
    fs::rename(staged, &target)?;
    Ok(Profile {
        privacy: Default::default(),
        java: None,
        icon: Default::default(),
        name: m.name,
        game_version: m.minecraft,
        loader: m.loader,
        directory,
    })
}
pub fn map_path(instance: &Path, entry: &Entry) -> PathBuf {
    let name = model::directory_from_name(&entry.name);
    instance.join("saves").join(format!(
        "{name} [CF-{}]",
        model::directory_from_name(&entry.id)
    ))
}

fn install_map_archive(
    archive: &Path,
    stage: &Path,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<()> {
    let extracted = stage.join("world");
    fs::create_dir(&extracted)?;
    extract_game_zip(archive, &extracted, cancel)?;
    let mut pending = vec![extracted];
    let mut world = None;
    while let Some(dir) = pending.pop() {
        downloads::check_cancel(cancel)?;
        if dir.join("level.dat").is_file() {
            anyhow::ensure!(world.is_none(), "В архиве несколько миров");
            world = Some(dir);
        } else {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() && entry.file_name() != "__MACOSX" {
                    pending.push(entry.path());
                }
            }
        }
    }
    let world = world.context("В архиве нет мира Minecraft (level.dat)")?;
    downloads::check_cancel(cancel)?;
    anyhow::ensure!(!destination.exists(), "Карта уже есть в папке миров");
    fs::rename(world, destination)?;
    Ok(())
}

fn extract_game_zip(archive: &Path, destination: &Path, cancel: &AtomicBool) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    let mut total = 0u64;
    let mut names = HashSet::new();
    anyhow::ensure!(zip.len() < 100000, "Слишком много файлов");
    for i in 0..zip.len() {
        downloads::check_cancel(cancel)?;
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let relative = game_path(entry.name())?;
        anyhow::ensure!(
            !entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000),
            "ZIP содержит ссылку"
        );
        anyhow::ensure!(names.insert(entry.name().to_lowercase()), "Повтор пути ZIP");
        total += entry.size();
        anyhow::ensure!(
            total <= 12 * 1024 * 1024 * 1024 && entry.size() <= 1024 * 1024 * 1024,
            "Превышен лимит ZIP"
        );
        let path = destination.join(relative);
        anyhow::ensure!(!path.exists(), "Конфликт файлов ZIP: {}", entry.name());
        fs::create_dir_all(path.parent().context("Нет папки")?)?;
        let mut output = File::create(path)?;
        let mut buffer = [0u8; 65536];
        loop {
            downloads::check_cancel(cancel)?;
            let n = entry.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n])?;
        }
    }
    Ok(())
}
fn read_refs(instance: &Path) -> Vec<FileSpec> {
    fs::read(instance.join("launcher/pack-files.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn existing_matches(path: &Path, spec: &FileSpec) -> Result<bool> {
    let mut file = File::open(path)?;
    if spec
        .size
        .is_some_and(|s| file.metadata().is_ok_and(|m| m.len() != s))
    {
        return Ok(false);
    }
    let (mut a, mut b, mut c, mut d) = (
        sha1::Sha1::new(),
        sha2::Sha256::new(),
        sha2::Sha512::new(),
        md5::Md5::new(),
    );
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        a.update(&buffer[..n]);
        b.update(&buffer[..n]);
        c.update(&buffer[..n]);
        d.update(&buffer[..n]);
    }
    let hashes = [
        (&spec.sha1, format!("{:x}", a.finalize())),
        (&spec.sha256, format!("{:x}", b.finalize())),
        (&spec.sha512, format!("{:x}", c.finalize())),
        (&spec.md5, format!("{:x}", d.finalize())),
    ];
    Ok(hashes.iter().any(|(h, _)| !h.is_empty())
        && hashes
            .iter()
            .all(|(h, v)| h.is_empty() || h.eq_ignore_ascii_case(v)))
}
fn read_loader(instance: &Path) -> String {
    fs::read(instance.join("launcher/pack-loader.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .map(|v| text(&v, "version"))
        .unwrap_or_default()
}
fn collect_export(root: &Path, current: &Path, files: &mut Vec<(String, PathBuf)>) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let first = relative.split('/').next().unwrap_or("");
        if !matches!(
            first,
            "mods"
                | "resourcepacks"
                | "shaderpacks"
                | "config"
                | "defaultconfigs"
                | "kubejs"
                | "scripts"
                | "patchouli_books"
                | "global_packs"
                | "options.txt"
                | "optionsof.txt"
        ) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            anyhow::ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "Ссылки/junction в экспорте запрещены"
            );
        }
        anyhow::ensure!(
            !metadata.file_type().is_symlink() && (metadata.is_file() || metadata.is_dir()),
            "Ссылки в экспорте запрещены"
        );
        game_path(&relative)?;
        if metadata.is_dir() {
            collect_export(root, &path, files)?;
        } else {
            files.push((relative, path));
        }
        anyhow::ensure!(files.len() <= 100000, "Слишком много файлов экспорта");
    }
    Ok(())
}
fn digest_file<D: Digest + Default>(path: &Path) -> Result<String>
where
    D::OutputSize: sha2::digest::generic_array::ArrayLength<u8>,
{
    let mut digest = D::default();
    let mut file = File::open(path)?;
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn file_sha256(p: &Path) -> Result<String> {
    digest_file::<sha2::Sha256>(p)
}
fn file_sha512(p: &Path) -> Result<String> {
    digest_file::<sha2::Sha512>(p)
}
fn file_sha1(p: &Path) -> Result<String> {
    digest_file::<sha1::Sha1>(p)
}

/// Explicit, network-dependent integration check; always uses isolated temporary instances.
pub fn catalog_smoke_test() -> Result<()> {
    let service = Service::new(String::new())?;
    for (provider, query) in [
        (Provider::Modrinth, "Fabulously Optimized"),
        (Provider::ATLauncher, "All The Fabric 5"),
        (Provider::Ftb, "stoneblock"),
        (Provider::Technic, "tekkit"),
    ] {
        let entries = service.search(provider, query, false, "1.20.1")?;
        let entry = entries.first().context("Каталог пуст")?;
        anyhow::ensure!(!entry.description.trim().is_empty(), "Нет описания");
        let size = crate::media::check_catalog_icon(&entry.icon_url)?;
        println!(
            "{}: {} · description + {}x{} avatar OK",
            provider.label(),
            entry.name,
            size[0],
            size[1]
        );
    }
    let resources = service.search(Provider::Modrinth, "Default Dark Mode", true, "1.20.1")?;
    let entry = resources.first().context("Нет ресурспака")?;
    crate::media::check_catalog_icon(&entry.icon_url)?;
    anyhow::ensure!(
        !entry.description.trim().is_empty(),
        "Нет описания ресурспака"
    );
    println!("Modrinth resourcepack description + avatar OK");
    Ok(())
}

pub fn smoke_test() -> Result<()> {
    let service = Service::new(std::env::var("CURSEFORGE_API_KEY").unwrap_or_default())?;
    for (provider, query) in [
        (Provider::Modrinth, "Fabulously Optimized"),
        (Provider::ATLauncher, "All The Fabric 5"),
        (Provider::Ftb, "stoneblock"),
        (Provider::Technic, "tekkit"),
    ] {
        let entries = service.search(provider, query, false, "1.20.1")?;
        let entry = entries.first().context("Каталог пуст")?;
        let versions = service.versions(entry, false, "1.20.1")?;
        anyhow::ensure!(!versions.is_empty(), "Нет версий {}", entry.name);
        println!(
            "{}: catalog + versions OK ({}, {} versions)",
            provider.label(),
            entry.name,
            versions.len()
        );
    }
    if !service.key.is_empty() {
        let entries = service.search(Provider::CurseForge, "faithful", true, "1.20.1")?;
        anyhow::ensure!(!entries.is_empty(), "CurseForge catalog empty");
        println!("CurseForge catalog OK");
    } else {
        println!("CurseForge: SKIPPED (API key required)");
    }
    let root = tempfile::tempdir()?;
    let instances = root.path().join("instances");
    let cancel = AtomicBool::new(false);
    let entries = service.search(Provider::Modrinth, "Fabulously Optimized", false, "1.20.1")?;
    let entry = entries
        .iter()
        .find(|e| e.name == "Fabulously Optimized")
        .context("Fabulously Optimized missing")?;
    let versions = service.get(
        &format!("https://api.modrinth.com/v2/project/{}/version", entry.id),
        &[("game_versions", json!(["1.20.1"]).to_string())],
    )?;
    let version = array(&versions)?.first().context("Нет тестовой версии")?;
    let profile = service.install(entry, &text(version, "id"), &instances, &cancel, &|_| {})?;
    let instance = instances.join(&profile.directory);
    anyhow::ensure!(
        instance.join("mods").read_dir()?.count() > 0,
        "Нет установленных модов"
    );
    println!("Modrinth mrpack installation + pinned loader OK");
    let resources = service.search(Provider::Modrinth, "Default Dark Mode", true, "1.20.1")?;
    let resource = resources.first().context("Нет ресурспака")?;
    let versions = service.versions(resource, true, "1.20.1")?;
    service.resource(
        resource,
        &versions.first().context("Нет версии ресурспака")?.id,
        &instance,
        &cancel,
    )?;
    println!("Resourcepack search + verified download OK");
    service.resource(resource, &versions[0].id, &instance, &cancel)?;
    let installed = crate::packs::resources(&instance);
    anyhow::ensure!(
        installed.len() == 1 && installed[0].project == resource.id,
        "Реестр ресурспаков не сохранился"
    );
    println!("Resourcepack duplicate download + persistent registry OK");
    for indexed in [false, true] {
        let archive = root
            .path()
            .join(if indexed { "indexed.zip" } else { "full.zip" });
        service.export(&profile, &instance, &archive, indexed, &cancel, &|_| {})?;
        let imported = service.import(&archive, &instances, &cancel, &|_| {})?;
        let target = instances.join(imported.directory);
        for (relative, path) in {
            let mut paths = vec![];
            collect_export(&instance, &instance, &mut paths)?;
            paths
        } {
            anyhow::ensure!(
                file_sha256(&path)? == file_sha256(&target.join(relative))?,
                "Roundtrip hash mismatch"
            );
        }
        println!(
            "{} export/import byte-for-byte OK",
            if indexed { "Indexed" } else { "Full" }
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn map_archive_installs_nested_world_without_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let zip_path = root.path().join("map.zip");
        let mut zip = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        for (name, data) in [
            ("package/world/level.dat", b"world".as_slice()),
            ("package/world/region/r.0.0.mca", b"region".as_slice()),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        let destination = root.path().join("installed");
        let stage = tempfile::tempdir_in(root.path()).unwrap();
        install_map_archive(
            &zip_path,
            stage.path(),
            &destination,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(fs::read(destination.join("level.dat")).unwrap(), b"world");
        assert!(destination.join("region/r.0.0.mca").is_file());
        let stage = tempfile::tempdir_in(root.path()).unwrap();
        assert!(install_map_archive(
            &zip_path,
            stage.path(),
            &destination,
            &AtomicBool::new(false)
        )
        .is_err());
        assert_eq!(fs::read(destination.join("level.dat")).unwrap(), b"world");
    }

    #[test]
    #[ignore = "Downloads a small world from CurseForge using the configured key"]
    fn live_map_download() {
        let root = directories::ProjectDirs::from("dev", "MLauncher", "MLauncher").unwrap();
        let (_, stored) = crate::secrets::load_api_keys(root.data_local_dir());
        let key = std::env::var("CURSEFORGE_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or(stored);
        assert!(!key.trim().is_empty(), "CurseForge API key required");
        let service = Service::new(key.clone()).unwrap();
        let mc = "1.21.1";
        let maps = service
            .search_maps("skyblock", mc, SortOrder::Relevance, 0)
            .unwrap();
        assert!(!maps.is_empty());
        let instance = tempfile::tempdir().unwrap();
        let profile = crate::model::Config::default().profiles.remove(0);
        let bridge =
            crate::overlay::Bridge::start(&profile, instance.path().to_owned(), key).unwrap();
        let client = reqwest::blocking::Client::new();
        let request = |route: &str, body: Value| -> Value {
            client
                .post(format!("http://127.0.0.1:{}{route}", bridge.port))
                .header("X-MLauncher-Token", bridge.secret())
                .json(&body)
                .send()
                .unwrap()
                .json()
                .unwrap()
        };
        let catalog = request(
            "/maps",
            json!({"provider":"curseforge","query":"skyblock","sort":"relevance"}),
        );
        assert!(
            catalog["entries"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty()),
            "{catalog}"
        );
        for entry in maps {
            let versions = service.versions(&entry, true, mc).unwrap();
            let Some(version) = versions.first() else {
                continue;
            };
            let Ok(spec) = service.cf_file(&entry.id, &version.id) else {
                continue;
            };
            if !spec.path.to_ascii_lowercase().ends_with(".zip")
                || !spec.size.is_some_and(|size| size <= 8 * 1024 * 1024)
            {
                continue;
            }
            let result = request(
                "/download-map",
                json!({"provider":"curseforge", "id":entry.id}),
            );
            assert!(result["message"].is_string(), "{result}");
            assert!(map_path(instance.path(), &entry)
                .join("level.dat")
                .is_file());
            println!("Installed and verified world: {}", entry.name);
            return;
        }
        panic!("No compatible downloadable small ZIP world found");
    }

    #[test]
    fn overlay_and_launcher_cannot_race_resource_manifests() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("launcher")).unwrap();
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.path().join("launcher/.resources.lock"))
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&lock).unwrap();
        let e = entry("Test", 0, "", "");
        let error = Service::new(String::new())
            .unwrap()
            .resource(&e, "", root.path(), &AtomicBool::new(false))
            .unwrap_err();
        assert!(error.to_string().contains("уже скачивается"));
    }
    #[test]
    fn verified_existing_download_is_idempotent_and_conflicts_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pack.zip");
        fs::write(&path, b"resource").unwrap();
        let spec = FileSpec {
            path: "pack.zip".into(),
            url: "https://invalid.invalid/pack.zip".into(),
            sha256: file_sha256(&path).unwrap(),
            ..Default::default()
        };
        let service = Service::new(String::new()).unwrap();
        let cancel = AtomicBool::new(false);
        service.fetch(&spec, &path, &cancel, false).unwrap();
        service.fetch(&spec, &path, &cancel, false).unwrap();
        fs::write(&path, b"user-edited").unwrap();
        assert!(service.fetch(&spec, &path, &cancel, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"user-edited");
    }
    fn entry(name: &str, downloads: u64, updated: &str, published: &str) -> Entry {
        Entry {
            id: name.to_lowercase(),
            name: name.into(),
            description: String::new(),
            provider: Provider::ATLauncher,
            website: String::new(),
            icon_url: String::new(),
            author: String::new(),
            downloads,
            updated: updated.into(),
            published: published.into(),
        }
    }
    #[test]
    fn catalog_cache_and_resource_registry_survive_restart() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cards.json");
        let entries: Vec<_> = (0..25)
            .map(|i| entry(&format!("Pack {i}"), i, "2026-01-01", "2025-01-01"))
            .collect();
        fs::write(&cache, serde_json::to_vec(&entries).unwrap()).unwrap();
        assert_eq!(cached_entries(&cache).len(), 20);
        fs::create_dir_all(root.path().join("resourcepacks")).unwrap();
        fs::write(root.path().join("resourcepacks/test.zip"), b"test").unwrap();
        let record = ResourceRecord {
            project: "project".into(),
            provider: Provider::Modrinth,
            version: "v1".into(),
            name: "Test".into(),
            path: "resourcepacks/test.zip".into(),
        };
        downloads::atomic_write(
            &root.path().join("launcher/resources.json"),
            &serde_json::to_vec(&vec![record]).unwrap(),
        )
        .unwrap();
        assert_eq!(resources(root.path())[0].version, "v1");
        fs::remove_file(root.path().join("resourcepacks/test.zip")).unwrap();
        assert!(resources(root.path()).is_empty());
    }
    #[test]
    fn catalog_sorting_uses_project_names_and_dates() {
        let mut entries = vec![
            entry("Older", 3, "2024-02-01", "2023-01-01"),
            entry("Popular", 900, "2023-05-01", "2022-01-01"),
            entry("Fresh", 12, "2026-01-01", "2025-12-31"),
        ];
        sort_entries(&mut entries, SortOrder::Downloads);
        assert_eq!(entries[0].name, "Popular");
        sort_entries(&mut entries, SortOrder::Updated);
        assert_eq!(entries[0].name, "Fresh");
        sort_entries(&mut entries, SortOrder::Newest);
        assert_eq!(entries[0].name, "Fresh");
        assert_eq!(catalog_name("Project Name", "1.20.1 Pack"), "Project Name");
        assert_eq!(catalog_name("  ", "Manifest"), "Manifest");
        assert!(Provider::ATLauncher.supports_sort(SortOrder::Updated));
        assert!(!Provider::ATLauncher.supports_sort(SortOrder::Downloads));
        assert!(!Provider::Technic.supports_sort(SortOrder::Newest));
    }
    #[test]
    fn indexed_export_contains_hashes_not_mod_bytes() {
        let root = tempfile::tempdir().unwrap();
        let instance = root.path().join("source");
        fs::create_dir_all(instance.join("mods")).unwrap();
        fs::write(instance.join("mods/test.jar"), b"testmod").unwrap();
        let spec = FileSpec {
            path: "mods/test.jar".into(),
            url: "https://example.com/test.jar".into(),
            sha256: file_sha256(&instance.join("mods/test.jar")).unwrap(),
            ..Default::default()
        };
        downloads::atomic_write(
            &instance.join("launcher/pack-files.json"),
            &serde_json::to_vec(&vec![spec]).unwrap(),
        )
        .unwrap();
        let p = Profile {
            privacy: Default::default(),
            java: None,
            icon: Default::default(),
            name: "Test".into(),
            game_version: "1.20.1".into(),
            loader: "fabric".into(),
            directory: "Test".into(),
        };
        let service = Service::new(String::new()).unwrap();
        let archive = root.path().join("indexed.zip");
        service
            .export(
                &p,
                &instance,
                &archive,
                true,
                &AtomicBool::new(false),
                &|_| {},
            )
            .unwrap();
        let mut zip = zip::ZipArchive::new(File::open(&archive).unwrap()).unwrap();
        assert_eq!(zip.len(), 1);
        let manifest = zip_json(&mut zip, "mlauncher-pack.json").unwrap();
        assert_eq!(manifest["files"][0]["path"], "mods/test.jar");
        assert_eq!(manifest["files"][0]["sha256"].as_str().unwrap().len(), 64);
        fs::write(instance.join("mods/test.jar"), b"changed").unwrap();
        assert!(service
            .export(
                &p,
                &instance,
                &root.path().join("bad.zip"),
                true,
                &AtomicBool::new(false),
                &|_| {}
            )
            .is_err());
        assert!(!root.path().join("bad.zip").exists());
    }
    #[test]
    fn malicious_zip_does_not_create_an_instance() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("bad.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file("../outside.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"bad").unwrap();
        zip.finish().unwrap();
        let instances = root.path().join("instances");
        assert!(Service::new(String::new())
            .unwrap()
            .import(&archive, &instances, &AtomicBool::new(false), &|_| {})
            .is_err());
        assert_eq!(fs::read_dir(instances).unwrap().count(), 0);
        assert!(!root.path().join("outside.txt").exists());
    }
    #[test]
    fn rejects_portable_traversal() {
        for p in [
            "../x",
            "C:/x",
            "mods/CON.jar",
            "mods/aux.txt",
            "mods/a.jar:ads",
            "mods/a./x",
            "mods/a\\x",
            "mods/../x",
            "/x",
            "mods//x",
        ] {
            assert!(safe_path(p).is_err(), "{p}");
        }
        assert!(safe_path("mods/sodium.jar").is_ok());
        assert!(game_path("launcher/pack-loader.json").is_err());
    }
    #[test]
    fn full_roundtrip_and_no_secrets() {
        let root = tempfile::tempdir().unwrap();
        let instance = root.path().join("source");
        fs::create_dir_all(instance.join("mods")).unwrap();
        fs::create_dir_all(instance.join("config")).unwrap();
        fs::write(instance.join("mods/test.jar"), b"mod").unwrap();
        fs::write(instance.join("config/a.toml"), b"test=true").unwrap();
        fs::write(instance.join("sessions.dat"), b"secret").unwrap();
        let p = Profile {
            privacy: Default::default(),
            java: None,
            icon: Default::default(),
            name: "Test".into(),
            game_version: "1.20.1".into(),
            loader: "fabric".into(),
            directory: "Test".into(),
        };
        let service = Service::new(String::new()).unwrap();
        let archive = root.path().join("full.zip");
        let cancel = AtomicBool::new(false);
        service
            .export(&p, &instance, &archive, false, &cancel, &|_| {})
            .unwrap();
        let imported = service
            .import(&archive, &root.path().join("instances"), &cancel, &|_| {})
            .unwrap();
        let path = root.path().join("instances").join(imported.directory);
        assert_eq!(fs::read(path.join("mods/test.jar")).unwrap(), b"mod");
        assert!(!path.join("sessions.dat").exists());
    }
}
