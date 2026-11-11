//! Original procedural block-world scenes; no Minecraft textures or external assets.
use crate::effects::{mix, Effect};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};

pub fn paint(ui: &egui::Ui, time: f32, effect: Effect, light: bool) {
    let p = ui.painter();
    let r = ui.max_rect();
    let at = |x: f32, y: f32| Pos2::new(r.left() + x * r.width(), r.top() + y * r.height());
    let rect = |x: f32, y: f32, w: f32, h: f32| {
        Rect::from_min_size(at(x, y), Vec2::new(w * r.width(), h * r.height()))
    };
    let sky = match effect {
        Effect::BlockHills => {
            if light {
                Color32::from_rgb(155, 196, 219)
            } else {
                Color32::from_rgb(15, 29, 49)
            }
        }
        Effect::CrystalCave => {
            if light {
                Color32::from_rgb(161, 172, 185)
            } else {
                Color32::from_rgb(13, 19, 31)
            }
        }
        Effect::Nether => {
            if light {
                Color32::from_rgb(175, 129, 116)
            } else {
                Color32::from_rgb(37, 16, 24)
            }
        }
        _ => {
            if light {
                Color32::from_rgb(173, 166, 190)
            } else {
                Color32::from_rgb(22, 16, 38)
            }
        }
    };
    p.rect_filled(r, 0., sky);
    match effect {
        Effect::BlockHills => {
            p.rect_filled(
                rect(0.78, 0.13, 0.065, 0.085),
                0.,
                if light {
                    Color32::from_rgb(233, 204, 126)
                } else {
                    Color32::from_rgb(156, 177, 192)
                },
            );
            for i in 0..7 {
                let n = i as f32;
                let x = (n * 0.19 + time * 0.004).rem_euclid(1.3) - 0.18;
                let y = 0.12 + (n * 0.37).fract() * 0.18;
                let cloud = mix(
                    sky,
                    Color32::from_rgb(190, 207, 214),
                    if light { 0.55 } else { 0.14 },
                );
                p.rect_filled(rect(x, y, 0.13, 0.025), 0., cloud);
                p.rect_filled(rect(x + 0.035, y - 0.02, 0.065, 0.025), 0., cloud);
            }
            for layer in 0..3 {
                let l = layer as f32;
                for i in 0..52 {
                    let n = i as f32;
                    let top = 0.52
                        + l * 0.13
                        + (n * 0.21 + l * 1.8).sin() * 0.065
                        + (n * 0.54 + l).cos() * 0.035;
                    let grass = [
                        Color32::from_rgb(56, 91, 70),
                        Color32::from_rgb(57, 112, 65),
                        Color32::from_rgb(70, 129, 62),
                    ][layer];
                    let dirt = [
                        Color32::from_rgb(47, 63, 57),
                        Color32::from_rgb(70, 65, 46),
                        Color32::from_rgb(88, 65, 42),
                    ][layer];
                    let x = n / 52.;
                    let top = (top * 40.).round() / 40.;
                    p.rect_filled(rect(x, top, 1. / 52. + 0.001, 1. - top), 0., dirt);
                    p.rect_filled(rect(x, top, 1. / 52. + 0.001, 0.025), 0., grass);
                    if layer == 2 && i % 7 == 2 {
                        p.rect_filled(
                            rect(x + 0.003, top - 0.095, 0.012, 0.095),
                            0.,
                            Color32::from_rgb(66, 48, 34),
                        );
                        p.rect_filled(
                            rect(x - 0.018, top - 0.16, 0.055, 0.075),
                            0.,
                            Color32::from_rgb(40, 86, 52),
                        );
                        p.rect_filled(
                            rect(x - 0.005, top - 0.19, 0.03, 0.05),
                            0.,
                            Color32::from_rgb(51, 100, 55),
                        );
                    }
                }
            }
        }
        Effect::CrystalCave => {
            let floor = |x: f32| 0.81 + (x * 11.).sin() * 0.052 + (x * 31.).cos() * 0.02;
            for i in 0..48 {
                let n = i as f32;
                let x = n / 48.;
                let width = 1. / 48. + 0.001;
                let ceiling = 0.17 + (x * 9.).cos() * 0.08 + (x * 28.).sin() * 0.04;
                let stone = mix(
                    sky,
                    Color32::from_rgb(91, 97, 132),
                    0.21 + (n * 0.7).sin().abs() * 0.08,
                );
                let edge = mix(stone, sky, 0.42);
                p.rect_filled(rect(x, 0., width, ceiling + 0.04), 0., edge);
                p.rect_filled(rect(x, 0., width, ceiling), 0., stone);
                p.rect_filled(rect(x, floor(x), width, 1. - floor(x)), 0., edge);
                p.rect_filled(rect(x, floor(x) + 0.035, width, 1. - floor(x)), 0., stone);
            }
            for i in 0..9 {
                let n = i as f32;
                let x = 0.06 + ((n * 0.381).fract()) * 0.91;
                let y = floor(x) + 0.02;
                let pulse = (time * 0.9 + n).sin() * 0.06 + 0.82;
                let c = if i % 2 == 0 {
                    Color32::from_rgb(93, 185, 209)
                } else {
                    Color32::from_rgb(163, 114, 226)
                };
                for shard in [-1., 1., 0.] {
                    let width = 0.009 + (n * 0.7).sin().abs() * 0.004;
                    let height =
                        (0.07 + (n * 0.69).sin().abs() * 0.09) * (1. - 0.44 * f32::abs(shard));
                    let axis = x + shard * 0.013;
                    let tip = at(axis + shard * 0.021, y - height);
                    let left = at(axis - width + shard * 0.013, y - height * 0.7);
                    let right = at(axis + width + shard * 0.013, y - height * 0.7);
                    let bottom = at(axis, y);
                    p.add(egui::Shape::convex_polygon(
                        vec![tip, left, at(axis - width * 0.65, y), bottom],
                        mix(sky, c, pulse * 0.59),
                        Stroke::NONE,
                    ));
                    p.add(egui::Shape::convex_polygon(
                        vec![tip, bottom, at(axis + width * 0.65, y), right],
                        mix(sky, c, pulse),
                        Stroke::NONE,
                    ));
                }
            }
            for i in 0..35 {
                let n = i as f32;
                let x = (n * 0.618).fract();
                let y = (n * 0.414 - time * 0.008).rem_euclid(0.65) + 0.15;
                let pulse = 0.3 + (time * 0.6 + n).sin().abs() * 0.35;
                p.rect_filled(
                    Rect::from_center_size(at(x, y), Vec2::splat(2. + n % 3.)),
                    0.,
                    mix(sky, Color32::from_rgb(114, 153, 192), pulse),
                );
            }
        }
        Effect::Nether => {
            for i in 0..30 {
                let n = i as f32;
                let top = 0.4 + (n * 0.61).sin().abs() * 0.31;
                p.rect_filled(
                    rect(n / 30., top, 1. / 30. + 0.002, 1. - top),
                    0.,
                    if i % 2 == 0 {
                        Color32::from_rgb(56, 42, 48)
                    } else {
                        Color32::from_rgb(45, 35, 43)
                    },
                );
                if i % 5 == 0 {
                    p.rect_filled(
                        rect(n / 30. + 0.008, top, 0.006, 1. - top),
                        0.,
                        Color32::from_rgb(164, 72, 35),
                    );
                }
            }
            p.rect_filled(rect(0., 0.88, 1., 0.12), 0., Color32::from_rgb(140, 52, 28));
            for i in 0..45 {
                let n = i as f32;
                let x = n / 45.;
                let pulse = 0.35 + 0.25 * (time * 0.8 + n).sin().abs();
                p.rect_filled(
                    rect(x, 0.89 + (n * 0.83).sin() * 0.012, 0.016, 0.006),
                    0.,
                    mix(
                        Color32::from_rgb(140, 52, 28),
                        Color32::from_rgb(220, 129, 49),
                        pulse,
                    ),
                );
                let px = (n * 0.618).fract();
                let py = 0.88 - (n * 0.414 + time * 0.025).rem_euclid(0.8);
                p.rect_filled(
                    Rect::from_center_size(at(px, py), Vec2::splat(2. + n % 3.)),
                    0.,
                    mix(sky, Color32::from_rgb(215, 115, 49), 0.6),
                );
            }
        }
        Effect::EndIslands => {
            for i in 0..8 {
                let n = i as f32;
                let x = 0.03 + (n * 0.381).fract() * 0.94;
                let y = 0.24 + n * 0.07 + (time * 0.27 + n * 2.).sin() * 0.012;
                let w = 0.035 + n * 0.015;
                let stone = Color32::from_rgb(172, 167, 116);
                let top_color = mix(sky, stone, 0.35 + n * 0.069);
                let front = mix(top_color, sky, 0.34);
                let side = mix(top_color, sky, 0.53);
                let top = vec![
                    at(x - w, y),
                    at(x - w * 0.5, y - 0.025),
                    at(x + w * 0.65, y - 0.025),
                    at(x + w, y),
                    at(x + w * 0.35, y + 0.02),
                    at(x - w * 0.7, y + 0.02),
                ];
                p.add(egui::Shape::convex_polygon(top, top_color, Stroke::NONE));
                p.add(egui::Shape::convex_polygon(
                    vec![
                        at(x - w * 0.7, y + 0.02),
                        at(x - w * 0.08, y + 0.02),
                        at(x + w * 0.08, y + w * 0.9),
                        at(x - w * 0.34, y + w * 0.68),
                    ],
                    front,
                    Stroke::NONE,
                ));
                p.add(egui::Shape::convex_polygon(
                    vec![
                        at(x - w * 0.08, y + 0.02),
                        at(x + w * 0.35, y + 0.02),
                        at(x + w, y),
                        at(x + w * 0.68, y + w * 0.53),
                        at(x + w * 0.08, y + w * 0.9),
                    ],
                    side,
                    Stroke::NONE,
                ));
                if i >= 4 && i % 2 == 1 {
                    let height = 0.085 + n * 0.011;
                    p.rect_filled(
                        rect(x - 0.01, y - height, 0.019, height),
                        0.,
                        mix(sky, Color32::from_rgb(42, 34, 57), 0.78),
                    );
                    p.rect_filled(
                        rect(x - 0.01, y - height, 0.006, height),
                        0.,
                        Color32::from_rgb(53, 43, 64),
                    );
                    p.circle_filled(
                        at(x, y - height - 0.012),
                        2.2 + (time * 2. + n).sin() * 0.9,
                        Color32::from_rgb(230, 151, 227),
                    );
                }
            }
            for i in 0..70 {
                let n = i as f32;
                let x = (n * 0.618 + time * 0.005).fract();
                let y = (n * 0.414 - time * 0.012).rem_euclid(1.);
                p.rect_filled(
                    Rect::from_center_size(at(x, y), Vec2::splat(2. + n % 4.)),
                    0.,
                    mix(
                        sky,
                        Color32::from_rgb(166, 104, 194),
                        0.25 + (time * 0.7 + n).sin().abs() * 0.4,
                    ),
                );
            }
        }
        _ => {}
    }
}
