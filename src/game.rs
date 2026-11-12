//! Minecraft installation and process supervision. Commands never go through a shell.
use crate::{
    auth::{Account, AccountKind},
    downloads::{self, Checksum, Download},
    java::{self, JavaSettings},
    model::Profile,
    versions,
};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use reqwest::blocking::Client;
use serde_json::{json, Value};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};
use zeroize::Zeroizing;

pub struct Identity {
    pub name: String,
    pub uuid: String,
    pub token: Zeroizing<String>,
    pub kind: AccountKind,
    pub ely_skins: bool,
    pub client_id: String,
}
impl Identity {
    pub fn from_account(account: &Account, client_id: String) -> Result<Self> {
        let token = if account.kind == AccountKind::Offline {
            Zeroizing::new("0".into())
        } else {
            Zeroizing::new(
                account
                    .session
                    .as_ref()
                    .context("Требуется повторный вход в аккаунт")?
                    .access_token
                    .to_string(),
            )
        };
        Ok(Self {
            name: account.name.clone(),
            uuid: account.uuid.clone(),
            token,
            kind: account.kind,
            ely_skins: account.ely_skins,
            client_id,
        })
    }
}
pub struct Request {
    pub overlay: Option<crate::overlay::Options>,
    pub profile: Profile,
    pub instance: PathBuf,
    pub storage: PathBuf,
    pub version_cache: PathBuf,
    pub settings: JavaSettings,
    pub identity: Identity,
}
pub enum Event {
    Progress(String),
    Log(String),
    Started,
    Finished(Result<i32, String>),
}
struct Metadata {
    value: Value,
    path: PathBuf,
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .with_context(|| format!("Нет поля {key} в метаданных"))
}
fn segment(value: &str) -> Result<&str> {
    if value.is_empty() || value.contains(['/', '\\', ':']) || value == "." || value == ".." {
        bail!("Небезопасный идентификатор");
    }
    Ok(value)
}
fn download_json(client: &Client, item: &Download, cancel: &AtomicBool) -> Result<Value> {
    downloads::download(client, item, cancel)?;
    Ok(serde_json::from_slice(&fs::read(&item.path)?)?)
}
fn artifact(value: &Value, root: &Path, fallback: Option<&str>) -> Result<Download> {
    let path = value["path"]
        .as_str()
        .or(fallback)
        .context("Нет пути файла")?;
    Ok(Download {
        url: string(value, "url")?.into(),
        path: root.join(downloads::relative(path)?),
        checksum: Checksum::Sha1(string(value, "sha1")?.into()),
        size: value["size"].as_u64(),
    })
}
fn metadata(
    client: &Client,
    storage: &Path,
    version_cache: &Path,
    version: &str,
    cancel: &AtomicBool,
) -> Result<Metadata> {
    segment(version)?;
    let cached = versions::cached(version_cache).ok();
    let entry = match cached
        .as_ref()
        .and_then(|list| list.iter().find(|v| v.id == version))
    {
        Some(entry) => entry.clone(),
        None => {
            let (list, _) = versions::load(version_cache)?;
            list.into_iter()
                .find(|v| v.id == version)
                .context("Версия отсутствует в официальном каталоге Mojang")?
        }
    };
    let path = storage
        .join("versions")
        .join(version)
        .join(format!("{version}.json"));
    let value = download_json(
        client,
        &Download {
            url: entry.url.clone(),
            path: path.clone(),
            checksum: Checksum::Sha1(entry.sha1.clone()),
            size: None,
        },
        cancel,
    )?;
    if value["id"] != version {
        bail!("ID версии в метаданных не совпал");
    }
    Ok(Metadata { value, path })
}

pub fn rules_allowed(rules: &Value) -> bool {
    let Some(rules) = rules.as_array() else {
        return true;
    };
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        let os = &rule["os"];
        let os_name = if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "osx"
        } else {
            "linux"
        };
        if os["name"].as_str().is_some_and(|name| name != os_name) {
            continue;
        }
        if let Some(arch) = os["arch"].as_str() {
            let matches = match arch {
                "x86" => cfg!(target_arch = "x86"),
                "x86_64" | "amd64" => cfg!(target_arch = "x86_64"),
                "aarch64" | "arm64" => cfg!(target_arch = "aarch64"),
                _ => false,
            };
            if !matches {
                continue;
            }
        }
        if let Some(pattern) = os["version"].as_str() {
            if !regex::Regex::new(pattern)
                .map(|r| r.is_match(if cfg!(windows) { "10.0" } else { "" }))
                .unwrap_or(false)
            {
                continue;
            }
        }
        if let Some(features) = rule["features"].as_object() {
            if features
                .iter()
                .any(|(key, expected)| expected.as_bool() != Some(key == "has_custom_resolution"))
            {
                continue;
            }
        }
        allowed = rule["action"] == "allow";
    }
    allowed
}
fn maven_path(name: &str) -> Result<String> {
    let parts: Vec<_> = name.split(':').collect();
    if !(3..=4).contains(&parts.len()) {
        bail!("Некорректная координата Maven");
    }
    let group = parts[0].replace('.', "/");
    let name = segment(parts[1])?;
    let version = segment(parts[2])?;
    let classifier = if let Some(c) = parts.get(3) {
        format!("-{}", segment(c)?)
    } else {
        String::new()
    };
    let path = format!("{group}/{name}/{version}/{name}-{version}{classifier}.jar");
    downloads::relative(&path)?;
    Ok(path)
}
fn loader_profile(
    client: &Client,
    request: &Request,
    cancel: &AtomicBool,
    java_major: u32,
) -> Result<Option<Value>> {
    if request.profile.loader == "vanilla" {
        return Ok(None);
    }
    let cache = request
        .instance
        .join("launcher")
        .join("loader-profile.json");
    if cache.is_file() {
        let value: Value = serde_json::from_slice(&fs::read(&cache)?)?;
        if value["inheritsFrom"] != request.profile.game_version {
            bail!("Версия загрузчика не соответствует инстансу");
        }
        return Ok(Some(value));
    }
    match request.profile.loader.as_str() {
        "fabric" | "quilt" => modrinth_style_loader(client, request, cancel),
        "forge" => forge_profile(client, request, cancel, java_major),
        "neoforge" => neoforge_profile(client, request, cancel, java_major),
        other => bail!(
            "Неизвестный загрузчик: {other}. Доступны: vanilla, fabric, quilt, forge, neoforge."
        ),
    }
}

fn pinned_loader(request: &Request) -> Result<Option<String>> {
    let path = request.instance.join("launcher/pack-loader.json");
    if !path.is_file() {
        return Ok(None);
    }
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    if value["loader"] != request.profile.loader {
        bail!("Загрузчик сборки не совпадает с закреплённым манифестом");
    }
    let version = value["version"].as_str().unwrap_or_default();
    if version.is_empty() {
        Ok(None)
    } else {
        segment(version)?;
        Ok(Some(version.into()))
    }
}

fn modrinth_style_loader(
    client: &Client,
    request: &Request,
    cancel: &AtomicBool,
) -> Result<Option<Value>> {
    let cache = request
        .instance
        .join("launcher")
        .join("loader-profile.json");
    if cache.is_file() {
        let value: Value = serde_json::from_slice(&fs::read(&cache)?)?;
        if value["inheritsFrom"] != request.profile.game_version {
            bail!("Версия загрузчика не соответствует инстансу");
        }
        return Ok(Some(value));
    }
    let base = if request.profile.loader == "fabric" {
        "https://meta.fabricmc.net/v2"
    } else {
        "https://meta.quiltmc.org/v3"
    };
    let pinned = pinned_loader(request)?;
    let list = if pinned.is_none() {
        downloads::json(
            client,
            &format!("{base}/versions/loader/{}", request.profile.game_version),
            &request.storage.join("metadata").join(format!(
                "{}-{}.json",
                request.profile.loader, request.profile.game_version
            )),
            cancel,
        )?
    } else {
        json!([])
    };
    let loaders = list.as_array().context("Некорректный список загрузчиков")?;
    let loader = loaders
        .iter()
        .find(|v| v["loader"]["stable"] == true)
        .or_else(|| loaders.first());
    let version = if let Some(version) = &pinned {
        version.as_str()
    } else {
        string(
            &loader.context("Для этой версии нет совместимого загрузчика")?["loader"],
            "version",
        )?
    };
    segment(version)?;
    let value = downloads::json(
        client,
        &format!(
            "{base}/versions/loader/{}/{version}/profile/json",
            request.profile.game_version
        ),
        &cache,
        cancel,
    )?;
    if value["inheritsFrom"] != request.profile.game_version {
        bail!("Неверная базовая версия загрузчика");
    }
    Ok(Some(value))
}

fn forge_profile(
    client: &Client,
    request: &Request,
    cancel: &AtomicBool,
    java_major: u32,
) -> Result<Option<Value>> {
    let cache = request.instance.join("launcher").join("forge-profile.json");
    if cache.is_file() {
        let value: Value = serde_json::from_slice(&fs::read(&cache)?)?;
        if value["inheritsFrom"] != request.profile.game_version {
            bail!("Версия Forge не соответствует инстансу");
        }
        return Ok(Some(value));
    }
    let pinned = pinned_loader(request)?;
    let metadata_text = if pinned.is_none() {
        downloads::text_cached(
            client,
            "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml",
            &request.storage.join("metadata/forge-metadata.xml"),
            cancel,
        )?
    } else {
        String::new()
    };
    let versions = if pinned.is_none() {
        extract_xml_versions(&metadata_text).context("Не удалось прочитать версии Forge")?
    } else {
        vec![]
    };
    let mc_version = &request.profile.game_version;
    let coordinate = pinned.map(|v| {
        if v.starts_with(&format!("{mc_version}-")) {
            v
        } else {
            format!("{mc_version}-{v}")
        }
    });
    let forge_version = coordinate
        .as_ref()
        .or_else(|| {
            versions
                .iter()
                .rfind(|v| v.starts_with(&format!("{mc_version}-")))
        })
        .context(format!("Forge не найден для Minecraft {mc_version}"))?;
    segment(forge_version)?;
    let installer_name = format!("forge-{forge_version}-installer.jar");
    let installer_path = request.storage.join("downloads").join(&installer_name);
    let installer_url = format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{forge_version}/{installer_name}");
    let hash = installer_hash(
        client,
        &installer_url,
        &request
            .storage
            .join("metadata")
            .join(format!("{installer_name}.sha1")),
        cancel,
    )?;
    downloads::download(
        client,
        &Download {
            url: installer_url,
            path: installer_path.clone(),
            checksum: Checksum::Sha1(hash),
            size: None,
        },
        cancel,
    )?;
    downloads::check_cancel(cancel)?;
    progress_installer(
        client,
        request,
        cancel,
        &installer_path,
        &cache,
        forge_version,
        java_major,
    )
}

fn neoforge_profile(
    client: &Client,
    request: &Request,
    cancel: &AtomicBool,
    java_major: u32,
) -> Result<Option<Value>> {
    let cache = request
        .instance
        .join("launcher")
        .join("neoforge-profile.json");
    if cache.is_file() {
        let value: Value = serde_json::from_slice(&fs::read(&cache)?)?;
        if value["inheritsFrom"] != request.profile.game_version {
            bail!("Версия NeoForge не соответствует инстансу");
        }
        return Ok(Some(value));
    }
    let pinned = pinned_loader(request)?;
    let metadata_text = if pinned.is_none() {
        downloads::text_cached(
            client,
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml",
            &request.storage.join("metadata/neoforge-metadata.xml"),
            cancel,
        )?
    } else {
        String::new()
    };
    let versions = if pinned.is_none() {
        extract_xml_versions(&metadata_text).context("Не удалось прочитать версии NeoForge")?
    } else {
        vec![]
    };
    let mc_version = &request.profile.game_version;
    let prefix = neoforge_prefix(mc_version)?;
    let neo_version = pinned
        .as_ref()
        .or_else(|| {
            versions
                .iter()
                .rfind(|v| v.starts_with(&prefix) && !v.contains("beta"))
        })
        .context(format!("NeoForge не найден для Minecraft {mc_version}"))?;
    segment(neo_version)?;
    let installer_name = format!("neoforge-{neo_version}-installer.jar");
    let installer_path = request.storage.join("downloads").join(&installer_name);
    let installer_url = format!(
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/{neo_version}/{installer_name}"
    );
    let hash = installer_hash(
        client,
        &installer_url,
        &request
            .storage
            .join("metadata")
            .join(format!("{installer_name}.sha1")),
        cancel,
    )?;
    downloads::download(
        client,
        &Download {
            url: installer_url,
            path: installer_path.clone(),
            checksum: Checksum::Sha1(hash),
            size: None,
        },
        cancel,
    )?;
    downloads::check_cancel(cancel)?;
    progress_installer(
        client,
        request,
        cancel,
        &installer_path,
        &cache,
        neo_version,
        java_major,
    )
}

fn installer_hash(client: &Client, url: &str, cache: &Path, cancel: &AtomicBool) -> Result<String> {
    let hash = downloads::text_cached(client, &format!("{url}.sha1"), cache, cancel)?;
    let hash = hash
        .split_whitespace()
        .next()
        .context("Нет SHA1 установщика")?;
    if hash.len() != 40 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        bail!("Некорректный SHA1 установщика");
    }
    Ok(hash.into())
}
fn neoforge_prefix(mc: &str) -> Result<String> {
    let parts = mc.split('.').collect::<Vec<_>>();
    if parts.first() != Some(&"1") || parts.len() < 2 {
        bail!("Неподдерживаемая версия NeoForge");
    }
    let minor = parts[1].parse::<u32>()?;
    let patch = parts.get(2).unwrap_or(&"0").parse::<u32>()?;
    if minor < 20 || minor == 20 && patch < 2 {
        bail!("Для этой версии выберите Forge");
    }
    Ok(format!("{minor}.{patch}."))
}
fn progress_installer(
    client: &Client,
    request: &Request,
    cancel: &AtomicBool,
    installer: &Path,
    cache: &Path,
    _loader_version: &str,
    java_major: u32,
) -> Result<Option<Value>> {
    let java = java::select(
        &request.storage,
        java_major,
        &request.settings,
        cancel,
        &|_| {},
    )?;
    // Forge expects a launcher installation with the base version present. Its generated
    // libraries must live in the same shared library root used by our classpath.
    let base = metadata(
        client,
        &request.storage,
        &request.version_cache,
        &request.profile.game_version,
        cancel,
    )?;
    let mc_version = &request.profile.game_version;
    let jar = request
        .storage
        .join("versions")
        .join(mc_version)
        .join(format!("{mc_version}.jar"));
    downloads::download(
        client,
        &Download {
            url: string(&base.value["downloads"]["client"], "url")?.into(),
            path: jar,
            checksum: Checksum::Sha1(string(&base.value["downloads"]["client"], "sha1")?.into()),
            size: base.value["downloads"]["client"]["size"].as_u64(),
        },
        cancel,
    )?;
    let profiles = request.storage.join("launcher_profiles.json");
    if !profiles.exists() {
        downloads::atomic_write(&profiles, br#"{"profiles":{}}"#)?;
    }
    let mut zip = zip::ZipArchive::new(File::open(installer)?)?;
    let modern = zip.file_names().any(|name| name == "version.json");
    let profile: Value = if modern {
        let mut file = zip.by_name("version.json")?;
        let mut bytes = vec![];
        file.read_to_end(&mut bytes)?;
        serde_json::from_slice(&bytes)?
    } else {
        let mut file = zip.by_name("install_profile.json")?;
        let mut bytes = vec![];
        file.read_to_end(&mut bytes)?;
        let value: Value = serde_json::from_slice(&bytes)?;
        value["versionInfo"].clone()
    };
    let profile_id = string(&profile, "id")?;
    segment(profile_id)?;
    let profile_path = request
        .storage
        .join("versions")
        .join(profile_id)
        .join(format!("{profile_id}.json"));
    let log_path = request.instance.join("launcher/logs/loader-installer.log");
    let (stdout, stderr) = if request.profile.privacy.disable_logs {
        (Stdio::null(), Stdio::null())
    } else {
        fs::create_dir_all(request.instance.join("launcher/logs"))?;
        let log = File::create(&log_path)?;
        (Stdio::from(log.try_clone()?), Stdio::from(log))
    };
    let mut command = java::java_command(&java);
    command
        .arg("-jar")
        .arg(installer)
        .arg("--installClient")
        .arg(&request.storage)
        .current_dir(&request.storage)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .context("Не удалось запустить установщик загрузчика")?;
    let start = std::time::Instant::now();
    let status = loop {
        if cancel.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(600) {
            let _ = child.kill();
            let _ = child.wait();
            downloads::check_cancel(cancel)?;
            bail!("Истекло время установки загрузчика");
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if !status.success() {
        if request.profile.privacy.disable_logs {
            bail!("Ошибка установщика загрузчика ({status})");
        }
        bail!(
            "Ошибка установщика загрузчика. Журнал: {}",
            log_path.display()
        );
    }
    downloads::check_cancel(cancel)?;
    let value: Value = serde_json::from_slice(
        &fs::read(&profile_path).context("Установщик не создал профиль загрузчика")?,
    )?;
    if value["inheritsFrom"].as_str() != Some(mc_version) {
        bail!("Установщик создал профиль с неверной базовой версией");
    }
    fs::create_dir_all(cache.parent().context("Нет папки кэша")?)?;
    fs::write(cache, serde_json::to_vec_pretty(&value)?)?;
    Ok(Some(value))
}
struct Libraries {
    downloads: Vec<Download>,
    classpath: Vec<PathBuf>,
    natives: Vec<PathBuf>,
}
fn libraries(
    client: &Client,
    metadata: &Value,
    loader: Option<&Value>,
    storage: &Path,
    cancel: &AtomicBool,
) -> Result<Libraries> {
    let root = storage.join("libraries");
    let mut result = Libraries {
        downloads: vec![],
        classpath: vec![],
        natives: vec![],
    };
    let mut seen = HashSet::new();
    let base = metadata["libraries"]
        .as_array()
        .context("Нет списка библиотек")?;
    let extra = loader.and_then(|v| v["libraries"].as_array());
    for lib in extra.into_iter().flatten().chain(base.iter()) {
        if !rules_allowed(&lib["rules"]) {
            continue;
        }
        let name = string(lib, "name")?;
        let key = name.split(':').take(2).collect::<Vec<_>>().join(":");
        let classifier = name.split(':').nth(3).unwrap_or("");
        if !seen.insert(format!("{key}:{classifier}")) {
            continue;
        }
        if lib["downloads"]["artifact"].is_object() {
            let value = &lib["downloads"]["artifact"];
            if value["url"].as_str().unwrap_or("").is_empty() {
                let path = root.join(downloads::relative(string(value, "path")?)?);
                if !path.is_file() {
                    bail!(
                        "Отсутствует созданная установщиком библиотека {}",
                        path.display()
                    );
                }
                result.classpath.push(path);
                continue;
            }
            let item = artifact(value, &root, None)?;
            result.classpath.push(item.path.clone());
            result.downloads.push(item);
        } else if lib["downloads"].is_null() {
            let path = maven_path(name)?;
            let local = root.join(downloads::relative(&path)?);
            if local.is_file()
                && (name.starts_with("net.minecraftforge:") || name.starts_with("net.neoforged:"))
            {
                result.classpath.push(local);
                continue;
            }
            let base = lib["url"]
                .as_str()
                .unwrap_or("https://libraries.minecraft.net/");
            let url = format!("{}{path}", base.trim_end_matches('/').to_owned() + "/");
            let checksum = if let Some(hash) = lib["sha1"].as_str() {
                hash.into()
            } else {
                let response = client
                    .get(format!("{url}.sha1"))
                    .send()?
                    .error_for_status()?;
                let mut hash = String::new();
                response.take(256).read_to_string(&mut hash)?;
                hash.split_whitespace()
                    .next()
                    .context("Нет SHA1 библиотеки")?
                    .to_owned()
            };
            if checksum.len() != 40 || !checksum.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("Некорректный SHA1 библиотеки");
            }
            let item = Download {
                url,
                path: root.join(downloads::relative(&path)?),
                checksum: Checksum::Sha1(checksum),
                size: lib["size"].as_u64(),
            };
            result.classpath.push(item.path.clone());
            result.downloads.push(item);
        }
        let os = if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "osx"
        } else {
            "linux"
        };
        if let Some(classifier) = lib["natives"][os].as_str() {
            let classifier = classifier.replace(
                "${arch}",
                if cfg!(target_pointer_width = "64") {
                    "64"
                } else {
                    "32"
                },
            );
            let value = &lib["downloads"]["classifiers"][classifier];
            let item = artifact(value, &root, None)?;
            result.natives.push(item.path.clone());
            result.downloads.push(item);
        }
        downloads::check_cancel(cancel)?;
    }
    Ok(result)
}
fn assets(
    client: &Client,
    metadata: &Value,
    storage: &Path,
    instance: &Path,
    cancel: &AtomicBool,
    progress: &(impl Fn(String) + Sync),
) -> Result<(String, PathBuf)> {
    let index = &metadata["assetIndex"];
    let id = string(index, "id")?;
    segment(id)?;
    let path = storage.join("assets/indexes").join(format!("{id}.json"));
    let value = download_json(
        client,
        &Download {
            url: string(index, "url")?.into(),
            path,
            checksum: Checksum::Sha1(string(index, "sha1")?.into()),
            size: index["size"].as_u64(),
        },
        cancel,
    )?;
    let objects = value["objects"].as_object().context("Нет ресурсов игры")?;
    let mut items = vec![];
    let mut seen = HashSet::new();
    for object in objects.values() {
        let hash = string(object, "hash")?;
        if hash.len() != 40 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            bail!("Некорректный хеш ресурса");
        }
        if seen.insert(hash) {
            items.push(Download {
                url: format!(
                    "https://resources.download.minecraft.net/{}/{hash}",
                    &hash[..2]
                ),
                path: storage.join("assets/objects").join(&hash[..2]).join(hash),
                checksum: Checksum::Sha1(hash.into()),
                size: object["size"].as_u64(),
            });
        }
    }
    downloads::batch(client, &items, cancel, 16, |done, total| {
        if done % 100 == 0 || done == total {
            progress(format!("Ресурсы: {done}/{total}"));
        }
    })?;
    let virtual_root = storage.join("assets/virtual").join(id);
    let mapped = if value["map_to_resources"] == true {
        Some(instance.join("resources"))
    } else if value["virtual"] == true {
        Some(virtual_root.clone())
    } else {
        None
    };
    if let Some(root) = mapped {
        for (name, object) in objects {
            downloads::check_cancel(cancel)?;
            let target = root.join(downloads::relative(name)?);
            if target.is_file() {
                continue;
            }
            let hash = string(object, "hash")?;
            let source = storage.join("assets/objects").join(&hash[..2]).join(hash);
            fs::create_dir_all(target.parent().context("Папка ресурса")?)?;
            if fs::hard_link(&source, &target).is_err() {
                fs::copy(&source, &target)?;
            }
        }
    }
    Ok((
        id.into(),
        if value["virtual"] == true {
            virtual_root
        } else {
            storage.join("assets")
        },
    ))
}
fn expand(value: &str, variables: &HashMap<String, String>) -> Result<String> {
    let mut value = value.to_owned();
    for (key, replacement) in variables {
        value = value.replace(&format!("${{{key}}}"), replacement);
    }
    if value.contains("${") {
        bail!("Неизвестный параметр запуска в метаданных");
    }
    Ok(value)
}
fn arguments(list: &Value, variables: &HashMap<String, String>) -> Result<Vec<String>> {
    let mut output = vec![];
    if let Some(list) = list.as_array() {
        for item in list {
            if let Some(text) = item.as_str() {
                output.push(expand(text, variables)?);
            } else if rules_allowed(&item["rules"]) {
                match &item["value"] {
                    Value::String(s) => output.push(expand(s, variables)?),
                    Value::Array(items) => {
                        for item in items {
                            output.push(expand(
                                item.as_str().context("Некорректный аргумент")?,
                                variables,
                            )?);
                        }
                    }
                    _ => bail!("Некорректные параметры запуска"),
                }
            }
        }
    }
    Ok(output)
}
struct Prepared {
    java: PathBuf,
    args: Vec<String>,
    secrets: Vec<Zeroizing<String>>,
}
fn prepare(
    request: &Request,
    cancel: &AtomicBool,
    progress: &(impl Fn(String) + Sync),
) -> Result<Prepared> {
    fs::create_dir_all(&request.storage)?;
    fs::create_dir_all(&request.instance)?;
    let client = downloads::client()?;
    progress("Метаданные Minecraft…".into());
    let metadata = metadata(
        &client,
        &request.storage,
        &request.version_cache,
        &request.profile.game_version,
        cancel,
    )?;
    let version = &metadata.value;
    let major = version["javaVersion"]["majorVersion"].as_u64().unwrap_or(8) as u32;
    let exe = java::select(&request.storage, major, &request.settings, cancel, progress)?;
    let mut identity_uuid = request.identity.uuid.clone();
    let mut agent = None;
    if request.identity.kind == AccountKind::ElyBy || request.identity.ely_skins {
        if matches!(version["type"].as_str(), Some("old_alpha" | "old_beta")) {
            bail!("Для Alpha/Beta интеграция Ely.by пока не проверена. Используйте обычный аккаунт без скинов.");
        }
        progress("Подключение Ely.by…".into());
        if request.identity.kind == AccountKind::Offline {
            crate::auth::validate_name(&request.identity.name)?;
            let response = client
                .get(format!(
                    "https://authserver.ely.by/api/users/profiles/minecraft/{}",
                    request.identity.name
                ))
                .send()?
                .error_for_status()?;
            if response.status().as_u16() != 204 {
                let profile: Value = response.json()?;
                identity_uuid = uuid::Uuid::parse_str(string(&profile, "id")?)?
                    .simple()
                    .to_string();
            }
        } else if request.identity.kind == AccountKind::ElyBy {
            let response = client
                .post("https://authserver.ely.by/auth/validate")
                .json(&json!({"accessToken":&*request.identity.token}))
                .send()?;
            if !response.status().is_success() {
                bail!("Сессия Ely.by истекла. Войдите повторно.");
            }
        }
        let manifest = downloads::json_cached(
            &client,
            "https://authlib-injector.yushi.moe/artifact/latest.json",
            &request.storage.join("metadata/authlib-injector.json"),
            cancel,
        )?;
        let hash = string(&manifest["checksums"], "sha256")?;
        let path = request
            .storage
            .join("agents")
            .join(format!("authlib-injector-{hash}.jar"));
        downloads::download(
            &client,
            &Download {
                url: string(&manifest, "download_url")?.into(),
                path: path.clone(),
                checksum: Checksum::Sha256(hash.into()),
                size: None,
            },
            cancel,
        )?;
        agent = Some(path);
    }
    let loader = loader_profile(&client, request, cancel, major)?;
    progress("Библиотеки Minecraft…".into());
    let mut libraries = libraries(&client, version, loader.as_ref(), &request.storage, cancel)?;
    let jar = request
        .storage
        .join("versions")
        .join(&request.profile.game_version)
        .join(format!("{}.jar", request.profile.game_version));
    libraries.downloads.push(Download {
        url: string(&version["downloads"]["client"], "url")?.into(),
        path: jar.clone(),
        checksum: Checksum::Sha1(string(&version["downloads"]["client"], "sha1")?.into()),
        size: version["downloads"]["client"]["size"].as_u64(),
    });
    downloads::batch(&client, &libraries.downloads, cancel, 12, |done, total| {
        if done % 20 == 0 || done == total {
            progress(format!("Файлы игры: {done}/{total}"));
        }
    })?;
    libraries.classpath.push(jar);
    let natives = request.instance.join("launcher/natives");
    fs::create_dir_all(&natives)?;
    for path in &libraries.natives {
        downloads::extract(path, &natives, true, cancel)?;
    }
    let (asset_id, game_assets) = assets(
        &client,
        version,
        &request.storage,
        &request.instance,
        cancel,
        progress,
    )?;
    let cp = std::env::join_paths(&libraries.classpath)
        .context("Некорректный classpath")?
        .to_string_lossy()
        .into_owned();
    let session = if request.identity.kind == AccountKind::Offline {
        "0".to_owned()
    } else {
        format!("token:{}:{}", *request.identity.token, identity_uuid)
    };
    let mut variables = HashMap::new();
    for (key, value) in [
        ("auth_player_name", request.identity.name.clone()),
        ("auth_uuid", identity_uuid),
        ("auth_access_token", request.identity.token.to_string()),
        ("auth_session", session.clone()),
        (
            "version_name",
            loader
                .as_ref()
                .and_then(|v| v["id"].as_str())
                .unwrap_or(&request.profile.game_version)
                .to_owned(),
        ),
        (
            "version_type",
            version["type"].as_str().unwrap_or("release").into(),
        ),
        (
            "game_directory",
            request.instance.to_string_lossy().into_owned(),
        ),
        (
            "assets_root",
            request
                .storage
                .join("assets")
                .to_string_lossy()
                .into_owned(),
        ),
        ("assets_index_name", asset_id),
        ("game_assets", game_assets.to_string_lossy().into_owned()),
        ("natives_directory", natives.to_string_lossy().into_owned()),
        ("classpath", cp),
        (
            "classpath_separator",
            if cfg!(windows) { ";" } else { ":" }.into(),
        ),
        (
            "library_directory",
            request
                .storage
                .join("libraries")
                .to_string_lossy()
                .into_owned(),
        ),
        ("launcher_name", "MLauncher".into()),
        ("launcher_version", env!("CARGO_PKG_VERSION").into()),
        (
            "user_type",
            if request.identity.kind == AccountKind::Microsoft {
                "msa"
            } else {
                "legacy"
            }
            .into(),
        ),
        ("user_properties", "{}".into()),
        ("clientid", request.identity.client_id.clone()),
        ("auth_xuid", String::new()),
        (
            "resolution_width",
            request.settings.window_width.clamp(320, 7680).to_string(),
        ),
        (
            "resolution_height",
            request.settings.window_height.clamp(240, 4320).to_string(),
        ),
    ] {
        variables.insert(key.into(), value);
    }
    let mut args = java::jvm_args(&request.settings, major)?;
    if let Some(path) = agent {
        args.push(format!("-javaagent:{}=ely.by", path.display()));
        args.push("-Dauthlibinjector.side=client".into());
    }
    if version["arguments"]["jvm"].is_array() {
        args.extend(arguments(&version["arguments"]["jvm"], &variables)?);
    } else {
        args.extend([
            format!("-Djava.library.path={}", natives.display()),
            "-cp".into(),
            variables["classpath"].clone(),
        ]);
    }
    if let Some(loader) = &loader {
        args.extend(arguments(&loader["arguments"]["jvm"], &variables)?);
    }
    if version["logging"]["client"]["file"].is_object() {
        let logging = &version["logging"]["client"];
        let item = artifact(
            &logging["file"],
            &request.storage.join("assets/log_configs"),
            logging["file"]["id"].as_str(),
        )?;
        downloads::download(&client, &item, cancel)?;
        args.push(string(logging, "argument")?.replace("${path}", &item.path.to_string_lossy()));
    }
    args.push(string(loader.as_ref().unwrap_or(version), "mainClass")?.into());
    if version["arguments"]["game"].is_array() {
        args.extend(arguments(&version["arguments"]["game"], &variables)?);
    } else {
        let legacy = loader
            .as_ref()
            .filter(|l| l["minecraftArguments"].is_string())
            .unwrap_or(version);
        for arg in string(legacy, "minecraftArguments")?.split_whitespace() {
            args.push(expand(arg, &variables)?);
        }
    }
    if let Some(loader) = &loader {
        args.extend(arguments(&loader["arguments"]["game"], &variables)?);
    }
    if request.settings.fullscreen {
        args.push("--fullscreen".into());
    }
    downloads::check_cancel(cancel)?;
    let _ = &metadata.path;
    Ok(Prepared {
        java: exe,
        args,
        secrets: vec![
            Zeroizing::new(request.identity.token.to_string()),
            Zeroizing::new(session),
        ],
    })
}
fn redact(line: &str, secrets: &[Zeroizing<String>]) -> String {
    let mut output = line.to_owned();
    for secret in secrets {
        if secret.len() > 8 {
            output = output.replace(secret.as_str(), "[REDACTED]");
        }
    }
    output
}
fn lock_instance(instance: &Path) -> Result<File> {
    fs::create_dir_all(instance)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(instance.join(".session.lock"))?;
    lock.try_lock_exclusive()
        .context("Эта сборка уже запущена. Для второй сессии создайте отдельную копию сборки.")?;
    Ok(lock)
}
pub fn run(
    request: Request,
    cancel: Arc<AtomicBool>,
    events: impl Fn(Event) + Send + Sync,
) -> Result<i32> {
    fs::create_dir_all(&request.instance)?;
    let instance_lock = lock_instance(&request.instance)?;
    crate::agent::recover(&request.instance)?;
    fs::create_dir_all(&request.storage)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(request.storage.join(".installation.lock"))?;
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                downloads::check_cancel(&cancel)?;
                events(Event::Progress("Ожидание другой установки…".into()));
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => return Err(e.into()),
        }
    }
    let mut prepared = prepare(&request, &cancel, &|s| events(Event::Progress(s)))?;
    // Shared assets/libraries are locked only during installation, not during gameplay.
    FileExt::unlock(&lock)?;
    let capture_logs = !request.profile.privacy.disable_logs;
    let log = Arc::new(Mutex::new(if capture_logs {
        let log_dir = request.instance.join("launcher/logs");
        fs::create_dir_all(&log_dir)?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis();
        Some(File::create(
            log_dir.join(format!("launch-{timestamp}.log")),
        )?)
    } else {
        None
    }));
    downloads::check_cancel(&cancel)?;
    let mut command = java::java_command(&prepared.java);
    command
        .env_remove("MLAUNCHER_OVERLAY_PORT")
        .env_remove("MLAUNCHER_OVERLAY_TOKEN")
        .env_remove("MLAUNCHER_OVERLAY_ACCENT")
        .env_remove("MLAUNCHER_OVERLAY_STYLE")
        .env_remove("MLAUNCHER_OVERLAY_THEME")
        .env_remove("MLAUNCHER_OVERLAY_INTERFACE")
        .env_remove("MLAUNCHER_OVERLAY_EFFECT")
        .env_remove("MLAUNCHER_OVERLAY_TEST")
        .env_remove("MLAUNCHER_OVERLAY_TEST_QUERY");
    let bridge = if let Some(options) = &request.overlay {
        match crate::overlay::Bridge::start(
            &request.profile,
            request.instance.clone(),
            options.key.clone(),
        ) {
            Ok(bridge) => {
                bridge.configure(&mut command);
                command
                    .env(
                        "MLAUNCHER_OVERLAY_ACCENT",
                        format!(
                            "{},{},{}",
                            options.accent[0], options.accent[1], options.accent[2]
                        ),
                    )
                    .env(
                        "MLAUNCHER_OVERLAY_STYLE",
                        if options.light { "light" } else { "dark" },
                    )
                    .env("MLAUNCHER_OVERLAY_EFFECT", &options.effect)
                    .env("MLAUNCHER_OVERLAY_THEME", format!("{:?}", options.theme))
                    .env(
                        "MLAUNCHER_OVERLAY_INTERFACE",
                        format!("{:?}", options.interface_style),
                    );
                prepared
                    .secrets
                    .push(Zeroizing::new(bridge.secret().to_owned()));
                Some(bridge)
            }
            Err(error) => {
                events(Event::Log(format!(
                    "[Оверлей] Не удалось запустить связь: {error:#}"
                )));
                None
            }
        }
    } else {
        None
    };
    command
        .args(&prepared.args)
        .current_dir(&request.instance)
        .stdin(Stdio::null())
        .stdout(if capture_logs {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(if capture_logs {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = command.spawn().context("Не удалось запустить Minecraft")?;
    if let (Some(options), Some(bridge)) = (&request.overlay, &bridge) {
        let message=match crate::overlay::inject(&child,&options.dll){Ok(())=>"[Оверлей] DLL подключена. Shift+Tab после загрузки OpenGL. Vulkan не поддерживается.".into(),Err(e)=>format!("[Оверлей] Отключён для этой сессии: {e:#}")};
        bridge.log(&message);
        if let Ok(mut guard) = log.lock() {
            if let Some(log) = guard.as_mut() {
                let _ = writeln!(log, "{message}");
            }
        }
        events(Event::Log(message));
    }
    events(Event::Started);
    let secrets = Arc::new(prepared.secrets);
    let (tx, rx) = mpsc::sync_channel::<String>(512);
    let mut threads = vec![];
    let stdout = child
        .stdout
        .take()
        .map(|stream| Box::new(stream) as Box<dyn Read + Send>);
    let stderr = child
        .stderr
        .take()
        .map(|stream| Box::new(stream) as Box<dyn Read + Send>);
    for stream in [stdout, stderr].into_iter().flatten() {
        let (tx, secrets, log) = (tx.clone(), secrets.clone(), log.clone());
        threads.push(std::thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            loop {
                let mut bytes = vec![];
                let result = reader
                    .by_ref()
                    .take(64 * 1024)
                    .read_until(b'\n', &mut bytes);
                if !matches!(result,Ok(n)if n>0) {
                    break;
                }
                let line = redact(String::from_utf8_lossy(&bytes).trim_end(), &secrets);
                if let Ok(mut guard) = log.lock() {
                    if let Some(log) = guard.as_mut() {
                        let _ = writeln!(log, "{line}");
                    }
                }
                let _ = tx.try_send(line);
            }
        }));
    }
    drop(tx);
    let status = loop {
        if let Some(bridge) = &bridge {
            if let Some(message) = bridge.take_status() {
                let line = format!("[Оверлей] {message}");
                if let Ok(mut guard) = log.lock() {
                    if let Some(log) = guard.as_mut() {
                        let _ = writeln!(log, "{line}");
                    }
                }
                events(Event::Log(line));
            }
        }
        for line in rx.try_iter().take(100) {
            if let Some(bridge) = &bridge {
                bridge.log(&line);
            }
            events(Event::Log(line));
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            break child.wait()?;
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    for thread in threads {
        let _ = thread.join();
    }
    for line in rx.try_iter() {
        if let Some(bridge) = &bridge {
            bridge.log(&line);
        }
        events(Event::Log(line));
    }
    FileExt::unlock(&instance_lock)?;
    Ok(status.code().unwrap_or(-1))
}

/// Explicit integration probe: official loader installer + classpath files, no account or game process.
pub fn loader_smoke_test() -> Result<()> {
    let root = tempfile::tempdir()?;
    let storage = root.path().join("shared");
    let client = downloads::client()?;
    let cancel = AtomicBool::new(false);
    for (loader, mc, major, pin) in [
        ("fabric", "1.20.1", 17, "0.16.7"),
        ("forge", "1.21.1", 21, ""),
        ("neoforge", "1.21.1", 21, ""),
    ] {
        let instance = root.path().join(loader);
        fs::create_dir_all(&instance)?;
        if !pin.is_empty() {
            downloads::atomic_write(
                &instance.join("launcher/pack-loader.json"),
                &serde_json::to_vec(&json!({"loader":loader,"version":pin}))?,
            )?;
        }
        let request = Request {
            overlay: None,
            profile: crate::model::Profile {
                privacy: Default::default(),
                java: None,
                icon: Default::default(),
                name: loader.into(),
                game_version: mc.into(),
                loader: loader.into(),
                directory: loader.into(),
            },
            instance,
            storage: storage.clone(),
            version_cache: storage.join("cache"),
            settings: java::JavaSettings {
                auto_download: false,
                ..Default::default()
            },
            identity: Identity::from_account(
                &crate::auth::Account::offline("LoaderTest", false)?,
                String::new(),
            )?,
        };
        println!("Testing {loader} {mc} official installation…");
        let metadata = metadata(&client, &storage, &request.version_cache, mc, &cancel)?;
        let profile =
            loader_profile(&client, &request, &cancel, major)?.context("No loader profile")?;
        let libraries = libraries(&client, &metadata.value, Some(&profile), &storage, &cancel)?;
        downloads::batch(&client, &libraries.downloads, &cancel, 8, |_, _| {})?;
        for path in &libraries.classpath {
            if !path.is_file() {
                bail!("Missing classpath file {}", path.display());
            }
        }
        println!(
            "{loader}: profile {} + {} classpath files OK",
            profile["id"],
            libraries.classpath.len()
        );
    }
    Ok(())
}

/// Извлекает версии из Maven metadata XML.
fn extract_xml_versions(xml: &str) -> Result<Vec<String>> {
    let mut versions = vec![];
    let mut in_versions = false;
    for line in xml.lines() {
        let line = line.trim();
        if line == "<versions>" {
            in_versions = true;
            continue;
        }
        if line == "</versions>" {
            break;
        }
        if in_versions {
            if let Some(version) = line
                .strip_prefix("<version>")
                .and_then(|s| s.strip_suffix("</version>"))
            {
                versions.push(version.to_owned());
            }
        }
    }
    if versions.is_empty() {
        bail!("В XML не найдено ни одной версии");
    }
    Ok(versions)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sessions_can_lock_different_instances_but_not_the_same_one() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        let lock = lock_instance(&first).unwrap();
        assert!(lock_instance(&first).is_err());
        let other = lock_instance(&second).unwrap();
        drop(lock);
        assert!(lock_instance(&first).is_ok());
        drop(other);
    }
    #[test]
    fn neoforge_versions_and_pack_pins() {
        assert_eq!(neoforge_prefix("1.21.1").unwrap(), "21.1.");
        assert_eq!(neoforge_prefix("1.20.4").unwrap(), "20.4.");
        assert!(neoforge_prefix("1.20.1").is_err());
    }
    #[test]
    fn rule_order_and_features() {
        assert!(rules_allowed(&Value::Null));
        assert!(!rules_allowed(
            &json!([{"action":"allow","os":{"name":"osx"}}])
        ));
        assert!(!rules_allowed(
            &json!([{"action":"allow","features":{"is_demo_user":true}}])
        ));
        assert!(rules_allowed(
            &json!([{"action":"allow","features":{"has_custom_resolution":true}}])
        ));
        assert!(!rules_allowed(
            &json!([{"action":"allow"},{"action":"disallow"}])
        ));
    }
    #[test]
    fn maven_and_argument_expansion() {
        assert_eq!(
            maven_path("net.fabricmc:fabric-loader:0.16.9").unwrap(),
            "net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar"
        );
        assert!(maven_path("a:../../bad:1").is_err());
        let vars = HashMap::from([("name".into(), "Player".into())]);
        assert_eq!(expand("${name}", &vars).unwrap(), "Player");
        assert!(expand("${missing}", &vars).is_err());
    }
    #[test]
    fn log_tokens_are_redacted() {
        assert_eq!(
            redact(
                "token abcdefghijkl end",
                &[Zeroizing::new("abcdefghijkl".into())]
            ),
            "token [REDACTED] end"
        );
    }
}
