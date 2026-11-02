//! The single application icon, shared by the title bar and native viewport.
use eframe::egui;

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
