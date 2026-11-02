//! Official version catalogue. Selection never mutates another instance's game version.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path, time::Duration};

pub const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionKind {
    #[serde(rename = "release")]
    Release,
    #[serde(rename = "snapshot")]
    Snapshot,
    #[serde(rename = "old_beta")]
    Beta,
    #[serde(rename = "old_alpha")]
    Alpha,
}

impl VersionKind {
    pub const ALL: [Self; 4] = [Self::Release, Self::Snapshot, Self::Beta, Self::Alpha];
    pub fn label(self) -> &'static str {
        match self {
            Self::Release => "Release",
            Self::Snapshot => "Snapshot",
            Self::Beta => "Beta",
            Self::Alpha => "Alpha",
        }
    }
    pub fn index(self) -> usize {
        match self {
            Self::Release => 0,
            Self::Snapshot => 1,
            Self::Beta => 2,
            Self::Alpha => 3,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MinecraftVersion {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: VersionKind,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
    pub url: String,
    pub sha1: String,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    versions: Vec<MinecraftVersion>,
}

fn parse(bytes: &[u8]) -> Result<Vec<MinecraftVersion>> {
    let manifest: Manifest =
        serde_json::from_slice(bytes).context("Не удалось прочитать манифест Mojang")?;
    if manifest.versions.is_empty() {
        bail!("Пустой список версий Mojang");
    }
    Ok(manifest.versions)
}

pub fn matches(version: &MinecraftVersion, filters: &[bool; 4], query: &str) -> bool {
    filters[version.kind.index()]
        && version
            .id
            .to_ascii_lowercase()
            .contains(&query.trim().to_ascii_lowercase())
}

pub fn fetch() -> Result<Vec<MinecraftVersion>> {
    let bytes = reqwest::blocking::Client::builder()
        .user_agent("MLauncher/0.1.0")
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?
        .get(MANIFEST)
        .send()?
        .error_for_status()?
        .bytes()?;
    parse(&bytes)
}

/// Локальный каталог без сетевого запроса: используется при запуске игры.
pub fn cached(root: &Path) -> Result<Vec<MinecraftVersion>> {
    parse(&std::fs::read(root.join("versions-cache.json"))?)
}

/// A failed network request falls back to the last successfully parsed catalogue.
pub fn load(root: &Path) -> Result<(Vec<MinecraftVersion>, String)> {
    match fetch() {
        Ok(versions) => {
            let result = (|| -> Result<()> {
                std::fs::create_dir_all(root)?;
                let mut cache = tempfile::NamedTempFile::new_in(root)?;
                cache.write_all(&serde_json::to_vec(&Manifest {
                    versions: versions.clone(),
                })?)?;
                cache.as_file().sync_all()?;
                cache.persist(root.join("versions-cache.json"))?;
                Ok(())
            })();
            let status = if result.is_ok() {
                "Официальный каталог Mojang обновлён".into()
            } else {
                "Каталог загружен; не удалось сохранить оффлайн-кеш".into()
            };
            Ok((versions, status))
        }
        Err(error) => {
            let cache = std::fs::read(root.join("versions-cache.json"))
                .ok()
                .and_then(|bytes| parse(&bytes).ok());
            match cache {
                Some(versions) => Ok((versions, "Нет связи с Mojang. Показан сохранённый каталог; новые версии могут отсутствовать.".into())),
                None => Err(error.context("Нет связи с Mojang и сохранённого каталога версий")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn example(id: &str, kind: VersionKind) -> MinecraftVersion {
        MinecraftVersion {
            id: id.into(),
            kind,
            release_time: "2024-01-01T00:00:00Z".into(),
            url: String::new(),
            sha1: String::new(),
        }
    }
    #[test]
    fn all_four_categories_and_search() {
        for kind in VersionKind::ALL {
            let version = example("b1.7.3", kind);
            assert!(matches(&version, &[true; 4], "B1.7"));
            assert!(!matches(&version, &[false; 4], ""));
            assert!(!matches(&version, &[true; 4], "1.21"));
        }
        assert!(matches(
            &example("24w14a", VersionKind::Snapshot),
            &[false, true, false, false],
            "24w"
        ));
        assert!(!matches(
            &example("1.21", VersionKind::Release),
            &[false, true, false, false],
            ""
        ));
    }
    #[test]
    fn recognizes_mojang_type_names() {
        for (name, kind) in [
            ("release", VersionKind::Release),
            ("snapshot", VersionKind::Snapshot),
            ("old_beta", VersionKind::Beta),
            ("old_alpha", VersionKind::Alpha),
        ] {
            assert_eq!(
                serde_json::from_str::<VersionKind>(&format!("\"{name}\"")).unwrap(),
                kind
            );
        }
        assert!(parse(br#"{"versions":[]}"#).is_err());
    }
}
