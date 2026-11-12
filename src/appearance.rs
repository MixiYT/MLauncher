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
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterfaceStyle {
    #[default]
    Standard,
    Material,
    LiquidGlass,
    Simple,
}

impl InterfaceStyle {
    pub const ALL: [Self; 4] = [
        Self::Standard,
        Self::Material,
        Self::LiquidGlass,
        Self::Simple,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Стандартный",
            Self::Material => "Material",
            Self::LiquidGlass => "Liquid Glass",
            Self::Simple => "Simple",
        }
    }

    pub fn current(ctx: &egui::Context) -> Self {
        ctx.data(|data| data.get_temp(egui::Id::new("interface_style")))
            .unwrap_or_default()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: Theme,
    pub interface_style: InterfaceStyle,
    pub accent: [u8; 3],
    pub animate: bool,
    pub background: String,
    pub dim: u8,
    pub panel_opacity: u8,
    pub effect: crate::effects::Effect,
    pub light_background: bool,
    pub transitions: bool,
    pub show_build_icons: bool,
    pub build_icon_size: u8,
    pub show_account_icons: bool,
    pub animate_icons: bool,
    pub animate_menus: bool,
    pub classic_title_bar: bool,
    pub split_version_creation: bool,
    pub colored_loader_icons: bool,
    pub open_panel_limit: usize,
    pub fill_workspace: bool,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::Legacy,
            interface_style: InterfaceStyle::Standard,
            accent: [120, 186, 170],
            animate: true,
            background: String::new(),
            dim: 30,
            panel_opacity: 240,
            effect: Default::default(),
            light_background: false,
            transitions: true,
            show_build_icons: true,
            build_icon_size: 18,
            show_account_icons: true,
            animate_icons: true,
            animate_menus: true,
            classic_title_bar: false,
            split_version_creation: false,
            colored_loader_icons: false,
            open_panel_limit: 4,
            fill_workspace: false,
        }
    }
}
impl Appearance {
    pub const BUILD_ICON_SIZES: std::ops::RangeInclusive<u8> = 14..=30;

    pub fn build_icon_size(&self) -> f32 {
        self.build_icon_size
            .clamp(
                *Self::BUILD_ICON_SIZES.start(),
                *Self::BUILD_ICON_SIZES.end(),
            )
            .into()
    }

    pub fn accent_color(&self) -> Color32 {
        Color32::from_rgb(self.accent[0], self.accent[1], self.accent[2])
    }
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("animate_icons"), self.animate_icons);
            data.insert_temp(egui::Id::new("animate_menus"), self.animate_menus);
        });
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
        let text = style.visuals.text_color();
        let border = if light {
            Color32::from_gray(180)
        } else {
            Color32::from_gray(76)
        };
        match self.interface_style {
            InterfaceStyle::Standard => {}
            InterfaceStyle::Material => {
                style.visuals.panel_fill =
                    Color32::from_rgb(panel[0], panel[1], panel[2]).lerp_to_gamma(accent, 0.06);
                style.visuals.window_fill = style.visuals.panel_fill;
                style.visuals.window_stroke = Stroke::NONE;
                style.visuals.window_corner_radius = 18.into();
                style.visuals.window_shadow = egui::epaint::Shadow {
                    offset: [0, 6],
                    blur: 20,
                    spread: 0,
                    color: Color32::from_black_alpha(if light { 35 } else { 70 }),
                };
                for widget in [
                    &mut style.visuals.widgets.inactive,
                    &mut style.visuals.widgets.hovered,
                    &mut style.visuals.widgets.active,
                    &mut style.visuals.widgets.open,
                ] {
                    widget.corner_radius = 12.into();
                    widget.bg_stroke = Stroke::NONE;
                }
                style.visuals.widgets.inactive.bg_fill =
                    style.visuals.panel_fill.lerp_to_gamma(accent, 0.12);
                style.visuals.widgets.inactive.weak_bg_fill =
                    style.visuals.widgets.inactive.bg_fill;
                style.visuals.widgets.noninteractive.corner_radius = 12.into();
                style.visuals.widgets.noninteractive.bg_stroke =
                    Stroke::new(1_f32, border.gamma_multiply(0.35));
                style.spacing.button_padding = Vec2::new(12., 6.);
            }
            InterfaceStyle::LiquidGlass => {
                let alpha = self.panel_opacity.clamp(140, 155);
                let glass = if light { [234, 242, 250] } else { [11, 17, 24] };
                style.visuals.panel_fill =
                    Color32::from_rgba_unmultiplied(glass[0], glass[1], glass[2], alpha);
                style.visuals.window_fill =
                    Color32::from_rgba_unmultiplied(glass[0], glass[1], glass[2], 228);
                style.visuals.window_stroke = Stroke::new(
                    1_f32,
                    Color32::from_white_alpha(if light { 142 } else { 43 }),
                );
                style.visuals.window_corner_radius = 20.into();
                style.visuals.window_shadow = egui::epaint::Shadow {
                    offset: [0, 6],
                    blur: 24,
                    spread: 1,
                    color: Color32::from_black_alpha(70),
                };
                let surface = if light {
                    Color32::from_white_alpha(122)
                } else {
                    Color32::from_white_alpha(20)
                };
                for widget in [
                    &mut style.visuals.widgets.inactive,
                    &mut style.visuals.widgets.hovered,
                    &mut style.visuals.widgets.active,
                    &mut style.visuals.widgets.open,
                ] {
                    widget.corner_radius = 14.into();
                    widget.bg_stroke = Stroke::new(
                        1_f32,
                        Color32::from_white_alpha(if light { 125 } else { 28 }),
                    );
                }
                style.visuals.widgets.inactive.bg_fill = surface;
                style.visuals.widgets.inactive.weak_bg_fill = surface;
                style.visuals.widgets.hovered.bg_fill = surface.lerp_to_gamma(accent, 0.25);
                style.visuals.widgets.active.bg_fill = surface.lerp_to_gamma(accent, 0.4);
                style.visuals.widgets.noninteractive.corner_radius = 14.into();
                style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(
                    1_f32,
                    Color32::from_white_alpha(if light { 80 } else { 20 }),
                );
            }
            InterfaceStyle::Simple => {
                style.visuals.panel_fill = Color32::from_rgb(panel[0], panel[1], panel[2]);
                style.visuals.window_fill = style.visuals.panel_fill;
                style.visuals.window_stroke = Stroke::new(1_f32, border);
                style.visuals.window_corner_radius = 2.into();
                style.visuals.window_shadow = egui::epaint::Shadow::NONE;
                style.visuals.popup_shadow = egui::epaint::Shadow::NONE;
                for widget in [
                    &mut style.visuals.widgets.inactive,
                    &mut style.visuals.widgets.hovered,
                    &mut style.visuals.widgets.active,
                    &mut style.visuals.widgets.open,
                ] {
                    widget.corner_radius = 2.into();
                    widget.bg_stroke = Stroke::new(1_f32, border);
                }
                style.visuals.widgets.noninteractive.corner_radius = 2.into();
                style.visuals.widgets.noninteractive.bg_stroke =
                    Stroke::new(1_f32, border.gamma_multiply(0.5));
                style.spacing.button_padding = Vec2::new(8., 5.);
            }
        }
        if self.interface_style != InterfaceStyle::Standard {
            style.visuals.menu_corner_radius = style.visuals.widgets.inactive.corner_radius;
            style.visuals.widgets.noninteractive.bg_fill = style.visuals.window_fill;
            style.visuals.slider_trailing_fill = self.interface_style == InterfaceStyle::Material;
            style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.5_f32, text);
            style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, text);
            style.visuals.widgets.active.fg_stroke = Stroke::new(1.5_f32, text);
            style.visuals.widgets.open.fg_stroke = Stroke::new(1.5_f32, text);
            style.visuals.selection.bg_fill = style
                .visuals
                .window_fill
                .to_opaque()
                .lerp_to_gamma(accent, if light { 0.35 } else { 0.3 });
            style.visuals.selection.stroke = Stroke::new(1_f32, text);
            style.visuals.extreme_bg_color = style.visuals.window_fill.to_opaque();
            style.visuals.faint_bg_color = style.visuals.widgets.inactive.bg_fill;
            if self.interface_style != InterfaceStyle::Simple {
                style.visuals.popup_shadow = style.visuals.window_shadow;
            }
        }
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("interface_style"), self.interface_style)
        });
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
        assert_eq!(old.interface_style, InterfaceStyle::Standard);
        let expected = Appearance {
            theme: Theme::Light,
            interface_style: InterfaceStyle::Material,
            background: "C:/wallpaper.gif".into(),
            accent: [255, 120, 20],
            ..Default::default()
        };
        let actual: Appearance =
            serde_json::from_str(&serde_json::to_string(&expected).unwrap()).unwrap();
        assert!(actual == expected);
        for theme in Theme::ALL {
            for interface_style in InterfaceStyle::ALL {
                let ctx = egui::Context::default();
                Appearance {
                    theme,
                    interface_style,
                    ..Default::default()
                }
                .apply(&ctx);
                assert_eq!(InterfaceStyle::current(&ctx), interface_style);
                let style = ctx.style();
                assert!(style.visuals.panel_fill.a() >= 140);
                let encoded = serde_json::to_string(&Appearance {
                    theme,
                    interface_style,
                    ..Default::default()
                })
                .unwrap();
                let decoded: Appearance = serde_json::from_str(&encoded).unwrap();
                assert_eq!(decoded.interface_style, interface_style);
            }
        }
    }
}
