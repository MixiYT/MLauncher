use crate::model::{self, Category, InstalledMod, Profile, Project, SearchOptions, Source};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha512};
use std::{
    collections::HashSet,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const MR: &str = "https://api.modrinth.com/v2";
const CF: &str = "https://api.curseforge.com/v1";

pub struct Api {
    client: Client,
    key: String,
}

#[derive(Clone, Debug)]
struct Artifact {
    project: String,
    version: String,
    name: String,
    filename: String,
    url: String,
    hash: String,
    sha512: bool,
}

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}
fn list(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().context("Некорректный ответ API")
}
fn loader_id(loader: &str) -> Result<&'static str> {
    match loader {
        "forge" => Ok("1"),
        "fabric" => Ok("4"),
        "quilt" => Ok("5"),
        "neoforge" => Ok("6"),
        _ => bail!("Неизвестный загрузчик"),
    }
}

impl Api {
    pub fn new(key: String) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    let api_request = attempt.previous().first().is_some_and(|url| {
                        matches!(
                            url.host_str(),
                            Some("api.curseforge.com" | "api.modrinth.com")
                        )
                    });
                    let changed_host = attempt
                        .previous()
                        .first()
                        .is_some_and(|url| url.host_str() != attempt.url().host_str());
                    if attempt.url().scheme() != "https" || (api_request && changed_host) {
                        attempt.error("Unsafe redirect rejected")
                    } else if attempt.previous().len() >= 10 {
                        attempt.error("Too many redirects")
                    } else {
                        attempt.follow()
                    }
                }))
                .user_agent("MLauncher/0.1.0 (Rust desktop mod manager)")
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(180))
                .build()?,
            key,
        })
    }

    fn get(&self, url: &str, params: &[(&str, String)], source: Source) -> Result<Value> {
        let mut req = self.client.get(url).query(params);
        if source == Source::CurseForge {
            if self.key.trim().is_empty() {
                bail!("Для CurseForge нужен API-ключ. Укажите его в настройках или CURSEFORGE_API_KEY.");
            }
            req = req.header("x-api-key", self.key.trim());
        }
        let response = req.send()?;
        match response.status().as_u16() {
            429 => bail!("Лимит запросов API. Подождите и повторите запрос."),
            401 | 403 => bail!("API отклонил доступ. Проверьте ключ и разрешения."),
            _ => {}
        }
        Ok(response.error_for_status()?.json()?)
    }

    pub fn search(
        &self,
        source: Source,
        query: &str,
        profile: &Profile,
        offset: usize,
    ) -> Result<Vec<Project>> {
        self.search_filtered(source, query, profile, offset, &SearchOptions::default())
    }

    pub fn categories(&self, source: Source) -> Result<Vec<Category>> {
        let response = match source {
            Source::Modrinth => self.get(&format!("{MR}/tag/category"), &[], source)?,
            Source::CurseForge => self.get(
                &format!("{CF}/categories"),
                &[("gameId", "432".into()), ("classId", "6".into())],
                source,
            )?,
        };
        let values = if source == Source::Modrinth {
            &response
        } else {
            &response["data"]
        };
        let mut categories: Vec<_> = list(values)?
            .iter()
            .filter(|c| source != Source::Modrinth || c["project_type"] == "mod")
            .map(|c| Category {
                id: if source == Source::Modrinth {
                    text(c, "name")
                } else {
                    c["id"].to_string()
                },
                name: text(c, "name"),
            })
            .collect();
        categories.sort_by_key(|c| c.name.to_lowercase());
        Ok(categories)
    }

    pub fn search_filtered(
        &self,
        source: Source,
        query: &str,
        profile: &Profile,
        offset: usize,
        options: &SearchOptions,
    ) -> Result<Vec<Project>> {
        let compatible_only = query.is_empty() && profile.loader != "vanilla";
        let mut facets = vec![vec!["project_type:mod".to_owned()]];
        if compatible_only {
            facets.push(vec![format!("versions:{}", profile.game_version)]);
            facets.push(vec![format!("categories:{}", profile.loader)]);
        }
        if let Some(category) = &options.category {
            facets.push(vec![format!("categories:{category}")]);
        }
        let mut v = match source {
            Source::Modrinth => self.get(
                &format!("{MR}/search"),
                &[
                    ("query", query.into()),
                    ("limit", "20".into()),
                    ("offset", offset.to_string()),
                    ("index", options.sort.modrinth().into()),
                    ("facets", json!(facets).to_string()),
                ],
                source,
            )?,
            Source::CurseForge => {
                let mut params = vec![
                    ("gameId", "432".into()),
                    ("classId", "6".into()),
                    ("searchFilter", query.into()),
                    ("pageSize", "20".into()),
                    ("index", offset.to_string()),
                    ("sortField", options.sort.curseforge().into()),
                    ("sortOrder", "desc".into()),
                ];
                if let Some(category) = &options.category {
                    params.push(("categoryId", category.clone()));
                }
                self.get(&format!("{CF}/mods/search"), &params, source)?
            }
        };
        if source == Source::Modrinth
            && offset == 0
            && options.category.is_none()
            && (1..=100).contains(&query.len())
            && query
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
            && list(&v["hits"])?.is_empty()
        {
            match self.get(&format!("{MR}/project/{query}"), &[], source) {
                Ok(project) if project["project_type"] == "mod" => v["hits"] = json!([project]),
                Ok(_) => {}
                Err(error)
                    if error
                        .downcast_ref::<reqwest::Error>()
                        .is_some_and(|e| e.status() == Some(reqwest::StatusCode::NOT_FOUND)) => {}
                Err(error) => return Err(error),
            }
        }
        list(
            &v[if source == Source::Modrinth {
                "hits"
            } else {
                "data"
            }],
        )?
        .iter()
        .map(|p| {
            Ok(match source {
                Source::Modrinth => Project {
                    id: p["project_id"]
                        .as_str()
                        .unwrap_or_else(|| p["id"].as_str().unwrap_or_default())
                        .to_owned(),
                    name: text(p, "title"),
                    description: text(p, "description"),
                    author: text(p, "author"),
                    downloads: p["downloads"].as_u64().unwrap_or(0),
                    website: format!("https://modrinth.com/mod/{}", text(p, "slug")),
                    source,
                    icon_url: text(p, "icon_url"),
                    categories: p["categories"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default(),
                },
                Source::CurseForge => Project {
                    id: p["id"]
                        .as_u64()
                        .context("Некорректный ID проекта")?
                        .to_string(),
                    name: text(p, "name"),
                    description: text(p, "summary"),
                    author: text(&p["authors"][0], "name"),
                    downloads: p["downloadCount"].as_f64().unwrap_or(0.) as u64,
                    website: text(&p["links"], "websiteUrl"),
                    source,
                    icon_url: text(&p["logo"], "thumbnailUrl"),
                    categories: p["categories"]
                        .as_array()
                        .map(|a| a.iter().map(|v| text(v, "name")).collect())
                        .unwrap_or_default(),
                },
            })
        })
        .collect()
    }

    // Resolve the full required dependency graph before touching the mods directory.
    fn resolve(
        &self,
        source: Source,
        id: &str,
        version: Option<&str>,
        profile: &Profile,
        seen: &mut HashSet<String>,
        plan: &mut Vec<Artifact>,
    ) -> Result<()> {
        if plan.len() > 200 || seen.len() > 200 {
            bail!("Слишком много зависимостей");
        }
        if seen.contains(id) {
            if let Some(required) = version {
                if let Some(previous) = plan.iter().find(|a| a.project == id) {
                    if previous.version != required {
                        bail!("Конфликт версий зависимости {id}");
                    }
                }
            }
            return Ok(());
        }
        seen.insert(id.into());
        match source {
            Source::Modrinth => {
                let v = if let Some(version) = version {
                    self.get(&format!("{MR}/version/{version}"), &[], source)?
                } else {
                    let versions = self.get(
                        &format!("{MR}/project/{id}/version"),
                        &[
                            ("game_versions", json!([profile.game_version]).to_string()),
                            ("loaders", json!([profile.loader]).to_string()),
                        ],
                        source,
                    )?;
                    list(&versions)?
                        .first()
                        .cloned()
                        .context("Нет совместимой версии мода")?
                };
                if !list(&v["game_versions"])?.contains(&json!(profile.game_version))
                    || !list(&v["loaders"])?.contains(&json!(profile.loader))
                {
                    bail!("Зависимость {id} несовместима с профилем");
                }
                let files = list(&v["files"])?;
                let f = files
                    .iter()
                    .find(|f| f["primary"] == true)
                    .or_else(|| files.first())
                    .context("Нет файла мода")?;
                plan.push(Artifact {
                    project: id.into(),
                    version: text(&v, "id"),
                    name: text(&v, "name"),
                    filename: text(f, "filename"),
                    url: text(f, "url"),
                    hash: text(&f["hashes"], "sha512"),
                    sha512: true,
                });
                for dep in list(&v["dependencies"])? {
                    if dep["dependency_type"] != "required" {
                        continue;
                    }
                    let version = dep["version_id"].as_str();
                    let project = if let Some(p) = dep["project_id"].as_str() {
                        p.to_owned()
                    } else if let Some(v) = version {
                        text(
                            &self.get(&format!("{MR}/version/{v}"), &[], source)?,
                            "project_id",
                        )
                    } else {
                        bail!("Невозможно разрешить зависимость {id}");
                    };
                    self.resolve(source, &project, version, profile, seen, plan)?;
                }
            }
            Source::CurseForge => {
                let response = self.get(
                    &format!("{CF}/mods/{id}/files"),
                    &[
                        ("gameVersion", profile.game_version.clone()),
                        ("modLoaderType", loader_id(&profile.loader)?.into()),
                        ("pageSize", "50".into()),
                    ],
                    source,
                )?;
                let f = list(&response["data"])?
                    .iter()
                    .filter(|f| f["isAvailable"].as_bool().unwrap_or(false))
                    .max_by_key(|f| text(f, "fileDate"))
                    .context("Нет доступного совместимого файла")?;
                let url = text(f, "downloadUrl");
                if url.is_empty() {
                    bail!("Автор запретил скачивание через сторонние клиенты. Откройте страницу CurseForge и скачайте файл вручную.");
                }
                let hash = list(&f["hashes"])?
                    .iter()
                    .find(|h| h["algo"] == 1)
                    .context("Нет SHA-1 файла")?;
                plan.push(Artifact {
                    project: id.into(),
                    version: f["id"].to_string(),
                    name: text(f, "displayName"),
                    filename: text(f, "fileName"),
                    url,
                    hash: text(hash, "value"),
                    sha512: false,
                });
                for dep in list(&f["dependencies"])? {
                    if dep["relationType"] == 3 {
                        let id = dep["modId"]
                            .as_u64()
                            .context("Некорректный ID зависимости")?
                            .to_string();
                        self.resolve(source, &id, None, profile, seen, plan)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn install(
        &self,
        project: &Project,
        profile: &Profile,
        instance: &Path,
        cancel: &AtomicBool,
        progress: impl Fn(String),
    ) -> Result<usize> {
        if profile.loader == "vanilla" {
            bail!("Установка модов в Vanilla-инстанс запрещена");
        }
        let mut plan = vec![];
        progress("Проверка совместимости и зависимостей…".into());
        self.resolve(
            project.source,
            &project.id,
            None,
            profile,
            &mut HashSet::new(),
            &mut plan,
        )?;
        let mut manifest = model::installed(instance)?;
        // Updating existing projects is deliberately explicit, never silently accumulate old JARs.
        for artifact in &plan {
            model::safe_filename(&artifact.filename)?;
            if manifest.iter().any(|m| {
                m.source == project.source
                    && m.project_id == artifact.project
                    && m.version_id != artifact.version
            }) {
                bail!(
                    "Другая версия {} уже установлена. Удалите её во вкладке «Установлено».",
                    artifact.name
                );
            }
        }
        std::fs::create_dir_all(instance)?;
        let staging = tempfile::tempdir_in(instance)?;
        let mods = instance.join("mods");
        std::fs::create_dir_all(&mods)?;
        let mut pending = vec![];
        let mut names = HashSet::new();
        for (index, a) in plan.iter().enumerate() {
            if !names.insert(a.filename.to_ascii_lowercase()) {
                bail!("Совпадение имён файлов зависимостей");
            }
            if manifest.iter().any(|m| {
                m.source == project.source && m.project_id == a.project && m.version_id == a.version
            }) && model::mod_path(instance, &a.filename)?.is_file()
            {
                continue;
            }
            if mods.join(&a.filename).exists() {
                bail!("Файл {} уже существует; перезапись запрещена", a.filename);
            }
            progress(format!(
                "Скачивание {}/{}: {}",
                index + 1,
                plan.len(),
                a.filename
            ));
            self.download_artifact(a, staging.path(), cancel)?;
            pending.push(a.clone());
        }
        // Hard links are same-volume, atomic and refuse to overwrite existing files.
        let mut committed = vec![];
        for a in &pending {
            let destination = mods.join(&a.filename);
            if let Err(error) = std::fs::hard_link(staging.path().join(&a.filename), &destination) {
                for path in committed {
                    let _ = std::fs::remove_file(path);
                }
                return Err(error.into());
            }
            committed.push(destination);
            manifest.retain(|m| !(m.source == project.source && m.project_id == a.project));
            manifest.push(InstalledMod {
                source: project.source,
                project_id: a.project.clone(),
                version_id: a.version.clone(),
                filename: a.filename.clone(),
                name: if a.project == project.id {
                    project.name.clone()
                } else {
                    a.name.clone()
                },
                icon_url: if a.project == project.id {
                    project.icon_url.clone()
                } else {
                    String::new()
                },
                description: if a.project == project.id {
                    project.description.clone()
                } else {
                    String::new()
                },
                categories: if a.project == project.id {
                    project.categories.clone()
                } else {
                    vec![]
                },
            });
        }
        if let Err(error) = save_manifest(instance, &manifest) {
            for path in committed {
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }
        Ok(pending.len())
    }

    fn check_cancel(cancel: &AtomicBool) -> Result<()> {
        if cancel.load(Ordering::Relaxed) {
            bail!("Установка отменена");
        }
        Ok(())
    }

    fn download_artifact(&self, a: &Artifact, dir: &Path, cancel: &AtomicBool) -> Result<()> {
        Self::check_cancel(cancel)?;
        let url = reqwest::Url::parse(&a.url)?;
        if url.scheme() != "https" {
            bail!("Разрешены только HTTPS-загрузки");
        }
        let mut response = self.client.get(url).send()?.error_for_status()?;
        let mut file = std::fs::File::create(dir.join(&a.filename))?;
        let mut sha512 = Sha512::new();
        let mut sha1 = sha1::Sha1::new();
        let mut size = 0usize;
        let mut buffer = [0u8; 65536];
        loop {
            Self::check_cancel(cancel)?;
            let n = response.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            size += n;
            if size > 512 * 1024 * 1024 {
                bail!("Файл превышает лимит 512 МиБ");
            }
            sha512.update(&buffer[..n]);
            sha1.update(&buffer[..n]);
            file.write_all(&buffer[..n])?;
        }
        file.sync_all()?;
        let actual = if a.sha512 {
            format!("{:x}", sha512.finalize())
        } else {
            format!("{:x}", sha1.finalize())
        };
        if a.hash.is_empty() || !actual.eq_ignore_ascii_case(&a.hash) {
            bail!("Контрольная сумма {} не совпала", a.filename);
        }
        Ok(())
    }

    fn fetch_latest(
        &self,
        source: Source,
        id: &str,
        profile: &Profile,
    ) -> Result<Option<Artifact>> {
        match source {
            Source::Modrinth => {
                let versions = self.get(
                    &format!("{MR}/project/{id}/version"),
                    &[
                        ("game_versions", json!([profile.game_version]).to_string()),
                        ("loaders", json!([profile.loader]).to_string()),
                    ],
                    source,
                )?;
                let Some(v) = list(&versions)?.first() else {
                    return Ok(None);
                };
                let files = list(&v["files"])?;
                let Some(f) = files
                    .iter()
                    .find(|f| f["primary"] == true)
                    .or_else(|| files.first())
                else {
                    return Ok(None);
                };
                Ok(Some(Artifact {
                    project: id.into(),
                    version: text(v, "id"),
                    name: text(v, "name"),
                    filename: text(f, "filename"),
                    url: text(f, "url"),
                    hash: text(&f["hashes"], "sha512"),
                    sha512: true,
                }))
            }
            Source::CurseForge => {
                let response = self.get(
                    &format!("{CF}/mods/{id}/files"),
                    &[
                        ("gameVersion", profile.game_version.clone()),
                        ("modLoaderType", loader_id(&profile.loader)?.into()),
                        ("pageSize", "50".into()),
                    ],
                    source,
                )?;
                let Some(f) = list(&response["data"])?
                    .iter()
                    .filter(|f| f["isAvailable"].as_bool().unwrap_or(false))
                    .max_by_key(|f| text(f, "fileDate"))
                else {
                    return Ok(None);
                };
                let url = text(f, "downloadUrl");
                if url.is_empty() {
                    bail!("Автор запретил скачивание через сторонние клиенты");
                }
                let hash = list(&f["hashes"])?
                    .iter()
                    .find(|h| h["algo"] == 1)
                    .context("Нет SHA-1 файла")?;
                Ok(Some(Artifact {
                    project: id.into(),
                    version: f["id"].to_string(),
                    name: text(f, "displayName"),
                    filename: text(f, "fileName"),
                    url,
                    hash: text(hash, "value"),
                    sha512: false,
                }))
            }
        }
    }

    /// Возвращает новую версию мода, если она отличается от установленной.
    pub fn latest_version(
        &self,
        source: Source,
        id: &str,
        profile: &Profile,
    ) -> Result<Option<String>> {
        Ok(self.fetch_latest(source, id, profile)?.map(|a| a.version))
    }

    /// Обновляет один мод, сохраняя его метаданные. Зависимости не обновляются автоматически.
    pub fn update(
        &self,
        item: &InstalledMod,
        profile: &Profile,
        instance: &Path,
        cancel: &AtomicBool,
        progress: impl Fn(String),
    ) -> Result<bool> {
        if profile.loader == "vanilla" {
            bail!("В Vanilla-инстанс нельзя устанавливать моды");
        }
        model::safe_filename(&item.filename)?;
        let Some(latest) = self.fetch_latest(item.source, &item.project_id, profile)? else {
            return Ok(false);
        };
        if latest.version == item.version_id {
            return Ok(false);
        }
        model::safe_filename(&latest.filename)?;
        let mut manifest = model::installed(instance)?;
        if manifest.iter().any(|entry| {
            entry.filename.eq_ignore_ascii_case(&latest.filename)
                && !(entry.source == item.source && entry.project_id == item.project_id)
        }) {
            bail!("Файл {} уже занят другим модом", latest.filename);
        }
        let mods = instance.join("mods");
        std::fs::create_dir_all(&mods)?;
        let staging = tempfile::tempdir_in(instance)?;
        progress(format!("Обновление {}: {}", item.name, latest.filename));
        self.download_artifact(&latest, staging.path(), cancel)?;
        let old_path = model::mod_path(instance, &item.filename)?;
        let disabled = old_path.extension().is_some_and(|ext| ext == "disabled");
        let backup = staging.path().join("previous.jar");
        let had_old = old_path.exists();
        if had_old {
            std::fs::rename(&old_path, &backup)?;
        }
        let destination = mods.join(if disabled {
            format!("{}.disabled", latest.filename)
        } else {
            latest.filename.clone()
        });
        if let Err(error) = std::fs::hard_link(staging.path().join(&latest.filename), &destination)
        {
            if had_old {
                let _ = std::fs::rename(&backup, &old_path);
            }
            return Err(error.into());
        }
        for entry in &mut manifest {
            if entry.source == item.source && entry.project_id == item.project_id {
                entry.version_id = latest.version.clone();
                entry.filename = latest.filename.clone();
                entry.name = latest.name.clone();
            }
        }
        if let Err(error) = save_manifest(instance, &manifest) {
            let _ = std::fs::remove_file(&destination);
            if had_old {
                let _ = std::fs::rename(&backup, &old_path);
            }
            return Err(error);
        }
        Ok(true)
    }
}

fn save_manifest(instance: &Path, manifest: &[InstalledMod]) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(instance)?;
    file.write_all(&serde_json::to_vec_pretty(manifest)?)?;
    file.as_file().sync_all()?;
    file.persist(instance.join("mods.json"))
        .map_err(|e| anyhow!(e.error))?;
    Ok(())
}

pub fn remove(instance: &Path, item: &InstalledMod) -> Result<()> {
    model::safe_filename(&item.filename)?;
    let mut manifest = model::installed(instance)?;
    let path = model::mod_path(instance, &item.filename)?;
    // Move aside so a failed manifest write can restore the mod.
    let staging = tempfile::tempdir_in(instance)?;
    let backup = staging.path().join(&item.filename);
    let exists = path.exists();
    if exists {
        std::fs::rename(&path, &backup)?;
    }
    manifest.retain(|m| !(m.source == item.source && m.project_id == item.project_id));
    if let Err(error) = save_manifest(instance, &manifest) {
        if exists {
            std::fs::rename(backup, path)?;
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loader_mapping() {
        assert_eq!(loader_id("neoforge").unwrap(), "6");
        assert!(loader_id("unknown").is_err());
    }
    #[test]
    fn disabled_mods_keep_metadata_and_can_be_removed() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("mods")).unwrap();
        let item = InstalledMod {
            filename: "disabled.jar".into(),
            project_id: "disabled".into(),
            ..Default::default()
        };
        std::fs::write(root.path().join("mods/disabled.jar.disabled"), b"disabled").unwrap();
        save_manifest(root.path(), std::slice::from_ref(&item)).unwrap();
        assert_eq!(model::installed(root.path()).unwrap().len(), 1);
        remove(root.path(), &item).unwrap();
        assert!(!root.path().join("mods/disabled.jar.disabled").exists());
        assert!(model::installed(root.path()).unwrap().is_empty());
    }
    #[test]
    fn removal_preserves_untracked_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("mods")).unwrap();
        std::fs::write(root.path().join("mods/other.jar"), b"keep").unwrap();
        let item = InstalledMod {
            source: Source::Modrinth,
            project_id: "test".into(),
            version_id: "1".into(),
            filename: "test.jar".into(),
            name: "Test".into(),
            ..Default::default()
        };
        std::fs::write(root.path().join("mods/test.jar"), b"test").unwrap();
        save_manifest(root.path(), std::slice::from_ref(&item)).unwrap();
        remove(root.path(), &item).unwrap();
        assert!(model::installed(root.path()).unwrap().is_empty());
        assert!(root.path().join("mods/other.jar").exists());
        assert!(!root.path().join("mods/test.jar").exists());
    }
}
