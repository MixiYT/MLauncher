//! Lightweight procedural backgrounds; no media downloads or decoder processes.
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    #[default]
    Landscape,
    None,
    Bubbles,
    Blur,
    Waves,
    Stars,
    Aurora,
    BlockHills,
    CrystalCave,
    Nether,
    EndIslands,
}
impl Effect {
    pub const ALL: [Self; 11] = [
        Self::Landscape,
        Self::None,
        Self::Bubbles,
        Self::Blur,
        Self::Waves,
        Self::Stars,
        Self::Aurora,
        Self::BlockHills,
        Self::CrystalCave,
        Self::Nether,
        Self::EndIslands,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Landscape => "Пейзаж",
            Self::None => "Без фона",
            Self::Bubbles => "Пузыри",
            Self::Blur => "Динамическое размытие",
            Self::Waves => "Волны",
            Self::Stars => "Звёзды",
            Self::Aurora => "Северное сияние",
            Self::BlockHills => "Блочные холмы",
            Self::CrystalCave => "Кристальная пещера",
            Self::Nether => "Базальт и лава",
            Self::EndIslands => "Острова Края",
        }
    }
    pub fn lite_supported(self) -> bool {
        matches!(
            self,
            Self::None | Self::Waves | Self::Bubbles | Self::Aurora
        )
    }
    pub const LITE: [Self; 4] = [Self::None, Self::Waves, Self::Bubbles, Self::Aurora];
}

pub fn paint(ui: &egui::Ui, time: f32, appearance: &crate::appearance::Appearance) {
    let effect = appearance.effect;
    let accent = effect_accent(appearance.accent_color());
    if effect == Effect::Landscape {
        crate::scenery::paint(ui, time, accent);
        return;
    }
    let r = ui.max_rect();
    let p = ui.painter();
    let light = appearance.light_background;
    if matches!(
        effect,
        Effect::BlockHills | Effect::CrystalCave | Effect::Nether | Effect::EndIslands
    ) {
        crate::voxel::paint(ui, time, effect, light);
        return;
    }
    let base = background_color(light);
    p.rect_filled(r, 0., base);
    let at = |x: f32, y: f32| Pos2::new(r.left() + x * r.width(), r.top() + y * r.height());
    // Opaque preblending avoids additive/gamma saturation on light backgrounds.
    let tint = |alpha: u8| mix(base, accent, alpha as f32 / 255.);
    match effect {
        Effect::Bubbles => {
            for i in 0..32 {
                let n = i as f32;
                let x = (n * 0.618034 + (time * 0.08 + n).sin() * 0.035)
                    .fract()
                    .abs();
                let y = 1.15 - (n * 0.371 + time * (0.012 + n % 4. * 0.003)).rem_euclid(1.4);
                let radius = 12. + (n * 17.3) % 48.;
                let center = at(x, y);
                p.circle_filled(center, radius, tint(if light { 18 } else { 12 }));
                p.circle_stroke(center, radius, Stroke::new(1.2_f32, tint(145)));
                p.circle_filled(center - Vec2::splat(radius * 0.3), radius * 0.12, tint(180));
            }
        }
        Effect::Blur => {
            let fields = blur_fields(time, accent);
            let mut mesh = egui::Mesh::default();
            let columns = 48u32;
            let rows = 28u32;
            for y in 0..=rows {
                for x in 0..=columns {
                    let u = x as f32 / columns as f32;
                    let v = y as f32 / rows as f32;
                    mesh.colored_vertex(at(u, v), blur_color(u, v, base, &fields, light));
                }
            }
            for y in 0..rows {
                for x in 0..columns {
                    let i = y * (columns + 1) + x;
                    let j = i + columns + 1;
                    mesh.add_triangle(i, i + 1, j);
                    mesh.add_triangle(i + 1, j + 1, j);
                }
            }
            p.add(egui::Shape::mesh(mesh));
        }
        Effect::Waves | Effect::Aurora => {
            let aurora = effect == Effect::Aurora;
            for band in 0..if aurora { 28 } else { 8 } {
                let b = band as f32;
                let points: Vec<_> = (0..=128)
                    .map(|i| {
                        let x = i as f32 / 128.;
                        let y = if aurora {
                            0.32 + b * 0.007
                                + (x * 7. + time * 0.24).sin() * 0.12
                                + (x * 13. - time * 0.15).sin() * 0.05
                        } else {
                            0.35 + b * 0.07 + (x * 6. + time * 0.4 + b * 0.45).sin() * 0.08
                        };
                        at(x, y)
                    })
                    .collect();
                let c = if aurora {
                    mix(
                        base,
                        Color32::from_rgb(50 + (band * 4) as u8, 210, 175),
                        0.18 + (b * 0.15).sin().abs() * 0.15,
                    )
                } else {
                    tint(110)
                };
                p.add(egui::Shape::line(
                    points,
                    Stroke::new(if aurora { 16_f32 } else { 3_f32 }, c),
                ));
            }
        }
        Effect::Stars => {
            for i in 0..160 {
                let n = i as f32;
                let center = at(
                    (n * 0.618034 + time * 0.001).fract(),
                    (n * 0.414214).fract(),
                );
                let glow = 0.45 + 0.55 * (time * (0.4 + n % 3. * 0.2) + n).sin().abs();
                let color = mix(
                    base,
                    if light {
                        Color32::from_rgb(45, 65, 110)
                    } else {
                        Color32::from_rgb(170, 200, 230)
                    },
                    glow * 0.9,
                );
                p.circle_filled(center, 0.7 + n % 3. * 0.6, color);
                if i % 17 == 0 {
                    let d = 4. + glow * 3.;
                    p.line_segment(
                        [center - Vec2::new(d, 0.), center + Vec2::new(d, 0.)],
                        Stroke::new(0.7_f32, color),
                    );
                    p.line_segment(
                        [center - Vec2::new(0., d), center + Vec2::new(0., d)],
                        Stroke::new(0.7_f32, color),
                    );
                }
            }
        }
        Effect::None
        | Effect::Landscape
        | Effect::BlockHills
        | Effect::CrystalCave
        | Effect::Nether
        | Effect::EndIslands => {}
    }
}

pub(crate) fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0., 1.);
    let channel = |x: u8, y: u8| (x as f32 * (1. - t) + y as f32 * t).round() as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}
fn background_color(light: bool) -> Color32 {
    if light {
        Color32::from_rgb(232, 239, 247)
    } else {
        Color32::from_rgb(8, 12, 23)
    }
}
fn effect_accent(color: Color32) -> Color32 {
    let max = color.r().max(color.g()).max(color.b());
    let min = color.r().min(color.g()).min(color.b());
    if max - min < 30 {
        Color32::from_rgb(66, 144, 170)
    } else {
        mix(Color32::from_rgb(45, 65, 95), color, 0.7)
    }
}
fn blur_fields(time: f32, accent: Color32) -> [(f32, f32, Color32); 5] {
    std::array::from_fn(|i| {
        let n = i as f32;
        let color = match i % 3 {
            0 => accent,
            1 => Color32::from_rgb(104, 85, 160),
            _ => Color32::from_rgb(50, 120, 135),
        };
        (
            0.5 + (time * 0.08 + n * 2.1).sin() * 0.48,
            0.5 + (time * 0.065 + n * 1.7).cos() * 0.48,
            color,
        )
    })
}
fn blur_color(
    x: f32,
    y: f32,
    base: Color32,
    fields: &[(f32, f32, Color32)],
    light: bool,
) -> Color32 {
    let mut sum = [0f32; 3];
    let mut total = 0.;
    for (cx, cy, color) in fields {
        let weight = (-((x - cx).powi(2) + (y - cy).powi(2)) / 0.085).exp();
        total += weight;
        sum[0] += color.r() as f32 * weight;
        sum[1] += color.g() as f32 * weight;
        sum[2] += color.b() as f32 * weight;
    }
    if total < 0.0001 {
        return base;
    }
    let color = Color32::from_rgb(
        (sum[0] / total) as u8,
        (sum[1] / total) as u8,
        (sum[2] / total) as u8,
    );
    mix(base, color, total.min(1.) * if light { 0.26 } else { 0.48 })
}

#[cfg(test)]
mod tests {
    #[test]
    fn effects_render_on_light_and_dark_backgrounds() {
        let ctx = eframe::egui::Context::default();
        for effect in super::Effect::ALL {
            for light_background in [false, true] {
                let appearance = crate::appearance::Appearance {
                    effect,
                    light_background,
                    ..Default::default()
                };
                let output = ctx.run(
                    eframe::egui::RawInput {
                        screen_rect: Some(eframe::egui::Rect::from_min_size(
                            eframe::egui::Pos2::ZERO,
                            eframe::egui::vec2(1100., 720.),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        eframe::egui::CentralPanel::default()
                            .show(ctx, |ui| super::paint(ui, 42., &appearance));
                    },
                );
                assert!(!output.shapes.is_empty());
            }
        }
    }
}
