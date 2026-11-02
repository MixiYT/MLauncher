//! Compile-time product policy; Lite never enables expensive optional features.
pub const LITE: bool = cfg!(feature = "lite");
pub const NAME: &str = if LITE { "MLauncher Lite" } else { "MLauncher" };
pub const DATA_NAME: &str = if LITE { "MLauncherLite" } else { "MLauncher" };

pub fn configure(config: &mut crate::model::Config) {
    if !LITE {
        return;
    }
    config.multi_session = Default::default();
    config.curseforge_manual = false;
    config.appearance.theme = crate::appearance::Theme::Dark;
    config.appearance.panel_opacity = 255;
    config.appearance.dim = 0;
    // Lite keeps the accent and simple animated effects, but not heavy scenes or video.
    if !config.appearance.effect.lite_supported() {
        config.appearance.effect = crate::effects::Effect::Waves;
    }
    if crate::media::is_video(std::path::Path::new(&config.appearance.background)) {
        config.appearance.background.clear();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lite_policy_cannot_restore_removed_features_from_config() {
        let mut config = crate::model::Config::default();
        config.multi_session.enabled = true;
        config.multi_session.overlay = true;
        config.multi_session.native_overlay = true;
        config.curseforge_manual = true;
        config.appearance.background = "old.mp4".into();
        super::configure(&mut config);
        if super::LITE {
            assert!(
                !config.multi_session.enabled
                    && !config.multi_session.overlay
                    && !config.curseforge_manual
            );
            assert!(!config.multi_session.native_overlay);
            assert!(config.appearance.background.is_empty());
            assert_eq!(config.appearance.panel_opacity, 255);
            assert!(config.appearance.effect.lite_supported());
        } else {
            assert!(config.multi_session.enabled);
        }
    }
}
