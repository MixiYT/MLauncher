use eframe::egui::{self, Color32, Stroke, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    Legacy,
    Dark,
    Light,
    Amoled,
}
impl Theme {
    pub const ALL: [Self; 4] = [Self::Legacy, Self::Dark, Self::Light, Self::Amoled];
    pub fn label(self) -> &'static str {
        match self {
            Self::Legacy => "Legacy",
            Self::Dark => "Тёмная",
            Self::Light => "Светлая",
            Self::Amoled => "AMOLED",
        }
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: Theme,
    pub accent: [u8; 3],
    pub animate: bool,
    pub background: String,
    pub dim: u8,
    pub panel_opacity: u8,
    pub effect: crate::effects::Effect,
    pub light_background: bool,
    pub transitions: bool,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::Legacy,
            accent: [120, 186, 170],
            animate: true,
            background: String::new(),
            dim: 30,
            panel_opacity: 240,
            effect: Default::default(),
            light_background: false,
            transitions: true,
        }
    }
}
impl Appearance {
    pub fn accent_color(&self) -> Color32 {
        Color32::from_rgb(self.accent[0], self.accent[1], self.accent[2])
    }
    pub fn apply(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        let light = self.theme == Theme::Light;
        style.visuals = if light {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        let (panel, button) = match self.theme {
            Theme::Legacy => ([46, 62, 67], [70, 78, 80]),
            Theme::Dark => ([28, 30, 38], [45, 48, 58]),
            Theme::Light => ([237, 240, 245], [218, 224, 232]),
            Theme::Amoled => ([4, 4, 7], [20, 20, 26]),
        };
        let accent = self.accent_color();
        style.visuals.panel_fill =
            Color32::from_rgba_unmultiplied(panel[0], panel[1], panel[2], self.panel_opacity);
        style.visuals.window_fill = Color32::from_rgb(panel[0], panel[1], panel[2]);
        style.visuals.window_stroke = Stroke::new(1_f32, accent.gamma_multiply(0.65));
        style.visuals.window_corner_radius = if self.theme == Theme::Legacy {
            1.into()
        } else {
            8.into()
        };
        style.visuals.override_text_color = Some(if light {
            Color32::from_rgb(30, 36, 45)
        } else {
            Color32::from_rgb(215, 220, 225)
        });
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(button[0], button[1], button[2]);
        style.visuals.widgets.inactive.weak_bg_fill = style.visuals.widgets.inactive.bg_fill;
        style.visuals.widgets.hovered.bg_fill =
            accent.gamma_multiply(if light { 0.9 } else { 0.45 });
        style.visuals.widgets.active.bg_fill = accent.gamma_multiply(0.55);
        style.visuals.selection.bg_fill = accent.gamma_multiply(if light { 0.9 } else { 0.4 });
        style.visuals.hyperlink_color = if light {
            accent.gamma_multiply(0.55)
        } else {
            accent
        };
        for widget in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            widget.corner_radius = if self.theme == Theme::Legacy {
                1.into()
            } else {
                5.into()
            };
        }
        style.spacing.item_spacing = Vec2::new(5., 6.);
        style.spacing.button_padding = Vec2::new(10., 6.);
        ctx.set_style(style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appearance_roundtrip_and_old_defaults() {
        let old: Appearance = serde_json::from_str("{}").unwrap();
        assert_eq!(old.accent, [120, 186, 170]);
        let expected = Appearance {
            theme: Theme::Light,
            background: "C:/wallpaper.gif".into(),
            accent: [255, 120, 20],
            ..Default::default()
        };
        let actual: Appearance =
            serde_json::from_str(&serde_json::to_string(&expected).unwrap()).unwrap();
        assert!(actual == expected);
        for theme in Theme::ALL {
            Appearance {
                theme,
                ..Default::default()
            }
            .apply(&egui::Context::default());
        }
    }
}
