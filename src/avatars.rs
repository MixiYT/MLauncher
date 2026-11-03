//! Skin faces, including the hat layer; separate from mod thumbnail cancellation.
use crate::auth::{Account, AccountKind};
use anyhow::{Context, Result};
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    sync::mpsc,
};

#[derive(Default)]
pub struct Avatars {
    textures: HashMap<String, TextureHandle>,
    requested: HashSet<String>,
    receivers: Vec<mpsc::Receiver<(String, ColorImage)>>,
}
pub fn key(account: &Account) -> String {
    let name = if account.kind == AccountKind::ElyBy || account.ely_skins {
        account.name.as_str()
    } else {
        ""
    };
    format!(
        "{}-{}-{}-{}",
        crate::auth::kind_key(account.kind),
        account.uuid,
        name,
        account.ely_skins
    )
}
impl Avatars {
    pub fn refresh(&mut self, accounts: &[Account], root: &std::path::Path, ctx: &egui::Context) {
        let mut jobs = vec![];
        for a in accounts {
            let id = key(a);
            if self.requested.insert(id.clone()) {
                self.textures.insert(
                    id.clone(),
                    ctx.load_texture(&id, fallback(&a.uuid), TextureOptions::NEAREST),
                );
                if a.kind == AccountKind::Offline && !a.ely_skins {
                    continue;
                }
                jobs.push((
                    id,
                    a.name.clone(),
                    a.uuid.clone(),
                    a.kind == AccountKind::ElyBy || a.ely_skins,
                ));
            }
        }
        if jobs.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.receivers.push(rx);
        let root = root.join("avatars");
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let Ok(client) = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(12))
                .redirect(reqwest::redirect::Policy::limited(3))
                .build()
            else {
                return;
            };
            // Publish every cached face before waiting for any network request.
            let mut refresh = Vec::new();
            for (id, name, uuid, ely) in jobs {
                let cache = root.join(format!("{}.png", crate::model::directory_from_name(&id)));
                let cached = std::fs::read(&cache).ok().and_then(|b| face(&b).ok());
                let valid = cached.is_some();
                if let Some(image) = cached {
                    let _ = tx.send((id.clone(), image));
                    ctx.request_repaint();
                }
                if !valid || !fresh(&cache) {
                    refresh.push((id, name, uuid, ely, cache));
                }
            }
            for (id, name, uuid, ely, cache) in refresh {
                if let Ok(bytes) = skin(&client, &name, &uuid, ely) {
                    if let Ok(image) = face(&bytes) {
                        let _ = crate::downloads::atomic_write(&cache, &bytes);
                        let _ = tx.send((id, image));
                        ctx.request_repaint();
                    }
                }
            }
        });
    }
    pub fn drain(&mut self, ctx: &egui::Context) {
        self.receivers.retain(|rx| loop {
            match rx.try_recv() {
                Ok((id, image)) => {
                    self.textures.insert(
                        id.clone(),
                        ctx.load_texture(id, image, TextureOptions::NEAREST),
                    );
                }
                Err(mpsc::TryRecvError::Empty) => return true,
                Err(mpsc::TryRecvError::Disconnected) => return false,
            }
        });
    }

    pub fn get(&self, a: &Account) -> Option<&TextureHandle> {
        self.textures.get(&key(a))
    }
}
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
fn fresh(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age < CACHE_TTL)
}
fn bounded(response: reqwest::blocking::Response) -> Result<Vec<u8>> {
    let mut bytes = vec![];
    response
        .error_for_status()?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 4 * 1024 * 1024, "Скин слишком большой");
    Ok(bytes)
}
fn skin(client: &reqwest::blocking::Client, name: &str, uuid: &str, ely: bool) -> Result<Vec<u8>> {
    let url = if ely {
        let mut url = reqwest::Url::parse("https://skinsystem.ely.by/skins/")?;
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("URL"))?
            .push(name);
        url
    } else {
        let uuid = uuid.replace('-', "");
        anyhow::ensure!(
            uuid.len() == 32 && uuid.chars().all(|c| c.is_ascii_hexdigit()),
            "UUID"
        );
        let data: serde_json::Value = serde_json::from_slice(&bounded(
            client
                .get(format!(
                    "https://sessionserver.mojang.com/session/minecraft/profile/{uuid}"
                ))
                .send()?,
        )?)?;
        let property = data["properties"]
            .as_array()
            .context("Нет текстур")?
            .iter()
            .find(|p| p["name"] == "textures")
            .context("Нет текстур")?;
        use base64::Engine;
        let textures: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::STANDARD
                .decode(property["value"].as_str().context("Нет текстур")?)?,
        )?;
        let mut url = reqwest::Url::parse(
            textures["textures"]["SKIN"]["url"]
                .as_str()
                .context("Нет скина")?,
        )?;
        anyhow::ensure!(
            url.host_str() == Some("textures.minecraft.net"),
            "Недопустимый сервер скинов"
        );
        let _ = url.set_scheme("https");
        url
    };
    bounded(client.get(url).send()?)
}
fn face(bytes: &[u8]) -> Result<ColorImage> {
    // Bound decoding before cropping; skins can have HD dimensions.
    let decoded = crate::media::decode_image(bytes, 1024, 1024)?;
    anyhow::ensure!(
        decoded.size[0] >= 64 && decoded.size[1] >= 32,
        "Некорректный скин"
    );
    let rgba: Vec<u8> = decoded
        .pixels
        .iter()
        .flat_map(|c| c.to_srgba_unmultiplied())
        .collect();
    let image = image::RgbaImage::from_raw(decoded.size[0] as u32, decoded.size[1] as u32, rgba)
        .context("RGBA")?;
    let scale = image.width() / 64;
    let mut face =
        image::imageops::crop_imm(&image, 8 * scale, 8 * scale, 8 * scale, 8 * scale).to_image();
    let hat =
        image::imageops::crop_imm(&image, 40 * scale, 8 * scale, 8 * scale, 8 * scale).to_image();
    // Some legacy skins fill the entire hat region opaque; only apply a meaningful layer.
    if hat.pixels().any(|p| p.0[3] < 255) {
        image::imageops::overlay(&mut face, &hat, 0, 0);
    }
    let face = image::imageops::resize(&face, 8, 8, image::imageops::FilterType::Nearest);
    Ok(ColorImage::from_rgba_unmultiplied([8, 8], face.as_raw()))
}
fn fallback(uuid: &str) -> ColorImage {
    let dark = uuid.bytes().fold(0u8, u8::wrapping_add) % 2 == 0;
    let mut image = ColorImage::new([8, 8], egui::Color32::from_rgb(174, 126, 89));
    for y in 0..8 {
        for x in 0..8 {
            let color = if y < 2 || (x == 0 || x == 7) && y < 4 {
                [65, 43, 28]
            } else if y == 4 && (x == 2 || x == 5) {
                if dark {
                    [75, 90, 155]
                } else {
                    [75, 145, 75]
                }
            } else if y == 6 && (2..=5).contains(&x) {
                [93, 56, 40]
            } else {
                [174, 126, 89]
            };
            image[(x, y)] = egui::Color32::from_rgb(color[0], color[1], color[2]);
        }
    }
    image
}

#[cfg(test)]
mod tests {
    #[test]
    fn avatar_cache_expires_after_a_day() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("skin.png");
        assert!(!super::fresh(&path));
        let file = std::fs::File::create(&path).unwrap();
        assert!(super::fresh(&path));
        file.set_times(std::fs::FileTimes::new().set_modified(
            std::time::SystemTime::now() - super::CACHE_TTL - std::time::Duration::from_secs(1),
        ))
        .unwrap();
        assert!(!super::fresh(&path));
    }
    #[test]
    fn face_crop_and_hat() {
        let mut skin = image::RgbaImage::new(64, 64);
        for y in 8..16 {
            for x in 8..16 {
                skin.put_pixel(x, y, image::Rgba([100, 150, 200, 255]));
            }
        }
        skin.put_pixel(40, 8, image::Rgba([255, 0, 0, 255]));
        let mut bytes = std::io::Cursor::new(vec![]);
        image::DynamicImage::ImageRgba8(skin)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let face = super::face(bytes.get_ref()).unwrap();
        assert_eq!(face.size, [8, 8]);
        assert_eq!(face[(0, 0)], eframe::egui::Color32::RED);
    }
}
