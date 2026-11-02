//! Original procedural block landscape: no third-party images or game assets.
use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2};

pub fn paint(ui: &egui::Ui, time: f32, accent: Color32) {
    let rect = ui.max_rect();
    let p = ui.painter();
    let w = rect.width();
    let h = rect.height();
    let at = |x: f32, y: f32| Pos2::new(rect.left() + x * w, rect.top() + y * h);
    for row in 0..100 {
        let t = row as f32 / 100.;
        p.rect_filled(
            Rect::from_min_max(at(0., t), at(1., t + 0.011)),
            0.,
            Color32::from_rgb(
                (22. + t * 9.) as u8,
                (46. + t * 23.) as u8,
                (61. + t * 12.) as u8,
            ),
        );
    }
    // Stars and distant fireflies.
    for i in 0..95 {
        let x = (noise(i * 3 + 10) + (time * 0.04 + i as f32).sin() * 0.004).rem_euclid(1.);
        let y = (noise(i * 7 + 30) * 0.65 + time * (0.0008 + noise(i) * 0.0008)).rem_euclid(0.66);
        let r = 0.7 + noise(i + 77) * 1.5;
        p.circle_filled(
            at(x, y),
            r * 3.,
            Color32::from_rgba_unmultiplied(171, 212, 217, 8),
        );
        p.circle_filled(
            at(x, y),
            r,
            Color32::from_rgba_unmultiplied(
                accent.r(),
                accent.g(),
                accent.b(),
                (50. + (time * 0.8 + i as f32).sin() * 25.) as u8,
            ),
        );
    }
    p.circle_filled(at(0.8, 0.19), 26., Color32::from_rgb(132, 163, 167));
    p.circle_filled(at(0.811, 0.18), 25., Color32::from_rgb(26, 53, 66));
    // Stepped terrain layers, from foggy horizon to foreground.
    for layer in 0..5 {
        let base = 0.57 + layer as f32 * 0.085;
        let count = 35;
        let mut points = vec![at(0., 1.)];
        for col in 0..=count {
            let x = col as f32 / count as f32;
            let height = base
                + (x * 9. + layer as f32 * 1.7).sin() * 0.035
                + noise(col + layer * 56) * 0.035;
            points.push(at(x, height));
            points.push(at(x + 1. / count as f32, height));
        }
        points.push(at(1.04, 1.));
        let colors = [
            Color32::from_rgb(31, 65, 71),
            Color32::from_rgb(28, 61, 63),
            Color32::from_rgb(25, 55, 53),
            Color32::from_rgb(22, 47, 44),
            Color32::from_rgb(18, 39, 37),
        ];
        // Concave silhouette triangulated as independent vertical rectangles.
        for pair in points[1..points.len() - 1].chunks(2) {
            if pair.len() == 2 {
                p.rect_filled(
                    Rect::from_min_max(pair[0], Pos2::new(pair[1].x, rect.bottom())),
                    0.,
                    colors[layer as usize],
                );
            }
        }
    }
    // River reflecting the sky, made of offset block-like banks.
    let river = [
        at(0.72, 0.69),
        at(0.76, 0.69),
        at(0.68, 0.77),
        at(0.74, 0.82),
        at(0.64, 0.88),
        at(0.72, 1.),
        at(0.52, 1.),
        at(0.55, 0.89),
        at(0.65, 0.81),
        at(0.61, 0.77),
    ];
    // egui convex polygons require convex geometry: use explicit triangle fan for this stylized river.
    let mut mesh = egui::Mesh::default();
    for point in river {
        mesh.colored_vertex(point, Color32::from_rgb(43, 77, 82));
    }
    for i in 1..river.len() - 1 {
        mesh.add_triangle(0, i as u32, i as u32 + 1);
    }
    p.add(Shape::mesh(mesh));
    for i in 0..12 {
        let y = 0.8 + i as f32 * 0.016;
        let x = 0.64 - ((y - 0.8) * 0.5) + (time * 0.2 + i as f32).sin() * 0.015;
        p.line_segment(
            [at(x, y), at(x + 0.014, y)],
            Stroke::new(
                1_f32,
                Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 20),
            ),
        );
    }
    for i in 0..18 {
        let x = noise(i * 17 + 3);
        let y = 0.67 + noise(i * 11) * 0.3;
        let size = (y - 0.5) * 85.;
        let pos = at(x, y);
        p.rect_filled(
            Rect::from_min_size(pos, Vec2::new(size * 0.18, size)),
            0.,
            Color32::from_rgb(24, 40, 39),
        );
        let foliage = Rect::from_center_size(
            pos + Vec2::new(size * 0.08, -size * 0.22),
            Vec2::new(size, size * 0.8),
        );
        p.rect_filled(foliage, 0., Color32::from_rgb(21, 47, 43));
        p.line_segment(
            [foliage.left_top(), foliage.right_top()],
            Stroke::new(2_f32, Color32::from_rgb(34, 61, 54)),
        );
    }
    p.rect_filled(rect, 0., Color32::from_black_alpha(20));
}

fn noise(seed: i32) -> f32 {
    let mut x = (seed as u32)
        .wrapping_mul(747796405)
        .wrapping_add(2891336453);
    x = ((x >> ((x >> 28) + 4)) ^ x).wrapping_mul(277803737);
    ((x >> 22) ^ x) as f32 / u32::MAX as f32
}
