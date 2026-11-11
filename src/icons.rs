//! The single application icon, shared by the title bar and native viewport.
use eframe::egui;
use std::{io::Read, path::Path};

pub fn custom_path(instance: &Path) -> std::path::PathBuf {
    instance.join("launcher/build-icon")
}

pub fn custom_image(path: &Path) -> anyhow::Result<(Vec<u8>, egui::ColorImage)> {
    const LIMIT: u64 = 5 * 1024 * 1024;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= LIMIT,
        "Значок должен занимать не больше 5 МиБ"
    );
    let image = crate::media::decode_image(&bytes, 64, 64)?;
    Ok((bytes, image))
}

pub fn custom_texture(ctx: &egui::Context, instance: &Path) -> Option<egui::TextureHandle> {
    let path = custom_path(instance);
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    let id = egui::Id::new(("custom_build_icon", &path));
    if let Some((cached, texture)) =
        ctx.data(|data| data.get_temp::<(std::time::SystemTime, egui::TextureHandle)>(id))
    {
        if cached == modified {
            return Some(texture);
        }
    }
    let (_, image) = custom_image(&path).ok()?;
    let texture = ctx.load_texture(
        path.display().to_string(),
        image,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(id, (modified, texture.clone())));
    Some(texture)
}

pub fn loader_texture(ui: &egui::Ui, loader: &str) -> Option<egui::TextureHandle> {
    let bytes: &[u8] = match loader {
        "fabric" => include_bytes!("../assets/loaders/fabric.png"),
        "forge" => include_bytes!("../assets/loaders/forge.png"),
        "neoforge" => include_bytes!("../assets/loaders/neoforge.png"),
        "quilt" => include_bytes!("../assets/loaders/quilt.png"),
        _ => return None,
    };
    let ctx = ui.ctx();
    let id = egui::Id::new(("loader_texture", loader));
    if let Some(texture) = ctx.data(|data| data.get_temp(id)) {
        return Some(texture);
    }
    let image = crate::media::decode_image(bytes, 128, 64).ok()?;
    let texture = ctx.load_texture(loader, image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    Some(texture)
}

pub fn bytes() -> &'static [u8] {
    include_bytes!("../assets/icon.png")
}

pub fn icon_data() -> anyhow::Result<egui::IconData> {
    let rgba = image::load_from_memory(bytes())?.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_decodes() {
        let icon = super::icon_data().unwrap();
        assert_eq!(
            icon.rgba.len(),
            icon.width as usize * icon.height as usize * 4
        );
    }
}
