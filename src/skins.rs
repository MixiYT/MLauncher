//! Local player skins and Ely.by textures shared by the avatar and Horizon.
use anyhow::{ensure, Context, Result};
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Cursor, Read},
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Serialize, Deserialize)]
pub struct LocalSkin {
    pub sha256: String,
    pub slim: bool,
}

impl LocalSkin {
    pub fn path(&self, root: &Path) -> Option<PathBuf> {
        (self.sha256.len() == 64 && self.sha256.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| root.join("skins").join(format!("{}.png", self.sha256)))
    }
}

fn bounded(input: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "Скин слишком большой");
    Ok(bytes)
}

pub fn import(root: &Path, path: &Path) -> Result<LocalSkin> {
    save(root, &bounded(File::open(path)?)?, false)
}

fn save(root: &Path, bytes: &[u8], slim: bool) -> Result<LocalSkin> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(64);
    limits.max_image_height = Some(64);
    reader.limits(limits);
    let image = reader
        .decode()
        .context("Выберите PNG-скин 64×64 или 64×32")?;
    ensure!(
        image.width() == 64 && matches!(image.height(), 32 | 64),
        "Выберите PNG-скин 64×64 или 64×32"
    );
    let skin = LocalSkin {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        slim: slim && image.height() == 64,
    };
    let path = skin.path(root).context("Некорректный хеш скина")?;
    if !path.is_file() {
        crate::downloads::atomic_write(&path, bytes)?;
    }
    Ok(skin)
}

fn texture_url(value: &str) -> Result<Url> {
    let mut url = Url::parse(value)?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && match url.host_str() {
                Some("ely.by") => url.path().starts_with("/storage/skins/"),
                Some("textures.minecraft.net") => url.path().starts_with("/texture/"),
                _ => false,
            },
        "Недопустимый адрес скина Ely.by"
    );
    let _ = url.set_scheme("https");
    Ok(url)
}

fn fetch_ely(root: &Path, name: &str) -> Result<Option<LocalSkin>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(6))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client
        .get(format!("https://skinsystem.ely.by/textures/{name}"))
        .query(&[("version", "2")])
        .send()?
        .error_for_status()?;
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(None);
    }
    let metadata: serde_json::Value = serde_json::from_slice(&bounded(response)?)?;
    let skin = &metadata["SKIN"];
    let Some(url) = skin["url"].as_str() else {
        return Ok(None);
    };
    let bytes = bounded(client.get(texture_url(url)?).send()?.error_for_status()?)?;
    Ok(Some(save(
        root,
        &bytes,
        skin["metadata"]["model"] == "slim",
    )?))
}

pub fn ely(root: &Path, name: &str) -> Result<Option<LocalSkin>> {
    crate::auth::validate_name(name)?;
    let path = root
        .join("skins/ely")
        .join(format!("{}.json", name.to_ascii_lowercase()));
    let cached = File::open(&path)
        .ok()
        .and_then(|file| serde_json::from_reader::<_, Option<LocalSkin>>(file.take(4096)).ok())
        .filter(|skin| {
            skin.as_ref()
                .is_none_or(|skin| skin.path(root).is_some_and(|p| p.is_file()))
        });
    if let Some(skin) = &cached {
        if fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age < CACHE_TTL)
        {
            return Ok(skin.clone());
        }
    }
    match fetch_ely(root, name) {
        Ok(skin) => {
            let _ = crate::downloads::atomic_write(&path, &serde_json::to_vec(&skin)?);
            Ok(skin)
        }
        Err(error) => cached.ok_or(error),
    }
}
