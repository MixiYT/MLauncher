use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    User,
    Folder,
    Refresh,
    Menu,
    Mods,
    Plus,
    Back,
    Next,
    Home,
    Trash,
    Search,
    Download,
    Link,
    Check,
    Copy,
    Microsoft,
    Ely,
    Info,
    Edit,
    Builds,
    Close,
    Minimize,
    Maximize,
    Restore,
    Versions,
    Resources,
    Exchange,
    Upload,
}

pub fn icon_button(ui: &mut egui::Ui, icon: Icon, tooltip: &str) -> Response {
    icon_button_size(ui, icon, tooltip, Vec2::new(40., 32.))
}

pub fn icon_button_size(ui: &mut egui::Ui, icon: Icon, tooltip: &str, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        visuals.corner_radius,
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    paint_icon(
        ui,
        icon,
        Rect::from_center_size(rect.center(), Vec2::splat(21.)),
        visuals.fg_stroke.color,
    );
    response.on_hover_text(tooltip)
}

pub fn provider_button(ui: &mut egui::Ui, icon: Icon, text: &str) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.), Sense::click());
    let v = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        v.corner_radius,
        v.bg_fill,
        v.bg_stroke,
        egui::StrokeKind::Inside,
    );
    paint_icon(
        ui,
        icon,
        Rect::from_min_size(rect.min + Vec2::new(10., 6.), Vec2::splat(26.)),
        v.fg_stroke.color,
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(48., 0.),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(13.),
        v.fg_stroke.color,
    );
    response
}

pub fn icon_tab(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(13.),
        ui.visuals().text_color(),
    );
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(galley.size().x + 42., 32.), Sense::click());
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        visuals.corner_radius,
        if selected {
            ui.visuals().selection.bg_fill
        } else {
            visuals.bg_fill
        },
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    paint_icon(
        ui,
        icon,
        Rect::from_center_size(rect.left_center() + Vec2::new(16., 0.), Vec2::splat(18.)),
        visuals.fg_stroke.color,
    );
    ui.painter().galley(
        rect.left_center() + Vec2::new(30., -galley.size().y / 2.),
        galley,
        visuals.fg_stroke.color,
    );
    response
}

pub fn paint_icon(ui: &egui::Ui, icon: Icon, rect: Rect, color: Color32) {
    let p = ui.painter();
    let at = |x: f32, y: f32| {
        Pos2::new(
            rect.left() + x * rect.width(),
            rect.top() + y * rect.height(),
        )
    };
    let s = Stroke::new(1.8_f32, color);
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([at(a.0, a.1), at(b.0, b.1)], s);
    };
    let outline = |x: f32, y: f32, w: f32, h: f32| {
        p.rect_stroke(
            Rect::from_min_max(at(x, y), at(x + w, y + h)),
            0.,
            s,
            egui::StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Versions => {
            for y in [0.22, 0.5, 0.78] {
                p.circle_filled(at(0.14, y), rect.width() * 0.04, color);
                line((0.3, y), (0.9, y));
            }
        }
        Icon::Resources => {
            outline(0.08, 0.12, 0.84, 0.76);
            p.circle_stroke(at(0.7, 0.32), rect.width() * 0.09, s);
            line((0.12, 0.75), (0.4, 0.42));
            line((0.4, 0.42), (0.64, 0.75));
            line((0.64, 0.75), (0.78, 0.6));
            line((0.78, 0.6), (0.9, 0.75));
        }
        Icon::Exchange => {
            line((0.12, 0.3), (0.86, 0.3));
            line((0.86, 0.3), (0.65, 0.1));
            line((0.86, 0.3), (0.65, 0.5));
            line((0.88, 0.7), (0.14, 0.7));
            line((0.14, 0.7), (0.35, 0.5));
            line((0.14, 0.7), (0.35, 0.9));
        }
        Icon::Upload => {
            line((0.5, 0.67), (0.5, 0.08));
            line((0.28, 0.3), (0.5, 0.08));
            line((0.5, 0.08), (0.72, 0.3));
            line((0.15, 0.7), (0.15, 0.88));
            line((0.15, 0.88), (0.85, 0.88));
            line((0.85, 0.88), (0.85, 0.7));
        }
        Icon::Close => {
            line((0.22, 0.22), (0.78, 0.78));
            line((0.78, 0.22), (0.22, 0.78));
        }
        Icon::Minimize => line((0.2, 0.65), (0.8, 0.65)),
        Icon::Maximize => outline(0.2, 0.2, 0.6, 0.6),
        Icon::Restore => {
            outline(0.35, 0.15, 0.5, 0.5);
            outline(0.15, 0.35, 0.5, 0.5);
        }
        Icon::Builds => {
            // A three-dimensional block, rather than an ambiguous refresh arrow.
            for (a, b) in [
                ((0.5, 0.08), (0.9, 0.3)),
                ((0.9, 0.3), (0.9, 0.74)),
                ((0.9, 0.74), (0.5, 0.96)),
                ((0.5, 0.96), (0.1, 0.74)),
                ((0.1, 0.74), (0.1, 0.3)),
                ((0.1, 0.3), (0.5, 0.08)),
                ((0.1, 0.3), (0.5, 0.52)),
                ((0.9, 0.3), (0.5, 0.52)),
                ((0.5, 0.52), (0.5, 0.96)),
            ] {
                line(a, b);
            }
        }
        Icon::User => {
            p.circle_stroke(rect.center(), rect.width() * 0.43, s);
            p.circle_filled(at(0.5, 0.37), rect.width() * 0.15, color);
            p.add(egui::Shape::convex_polygon(
                vec![at(0.2, 0.78), at(0.3, 0.6), at(0.7, 0.6), at(0.8, 0.78)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Microsoft => {
            for (i, c) in [
                Color32::from_rgb(242, 80, 34),
                Color32::from_rgb(127, 186, 0),
                Color32::from_rgb(0, 164, 239),
                Color32::from_rgb(255, 185, 0),
            ]
            .into_iter()
            .enumerate()
            {
                let x = (i % 2) as f32 * 0.49;
                let y = (i / 2) as f32 * 0.49;
                p.rect_filled(Rect::from_min_max(at(x, y), at(x + 0.44, y + 0.44)), 0., c);
            }
        }
        Icon::Ely => {
            p.rect_filled(rect, 0., Color32::from_rgb(0, 143, 104));
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "E",
                egui::FontId::proportional(rect.height() * 0.9),
                Color32::WHITE,
            );
        }
        Icon::Folder => {
            p.add(egui::Shape::convex_polygon(
                vec![
                    at(0.08, 0.25),
                    at(0.4, 0.25),
                    at(0.52, 0.37),
                    at(0.92, 0.37),
                    at(0.82, 0.84),
                    at(0.08, 0.84),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Plus => {
            line((0.5, 0.12), (0.5, 0.88));
            line((0.12, 0.5), (0.88, 0.5));
        }
        Icon::Back => {
            line((0.8, 0.5), (0.2, 0.5));
            line((0.2, 0.5), (0.5, 0.2));
            line((0.2, 0.5), (0.5, 0.8));
        }
        Icon::Next => {
            line((0.2, 0.5), (0.8, 0.5));
            line((0.8, 0.5), (0.5, 0.2));
            line((0.8, 0.5), (0.5, 0.8));
        }
        Icon::Home => {
            line((0.1, 0.48), (0.5, 0.12));
            line((0.5, 0.12), (0.9, 0.48));
            outline(0.24, 0.45, 0.52, 0.43);
            outline(0.44, 0.65, 0.15, 0.23);
        }
        Icon::Menu => {
            for y in [0.22, 0.5, 0.78] {
                line((0.12, y), (0.88, y));
            }
        }
        Icon::Mods => {
            for (x, y) in [(0.12, 0.12), (0.56, 0.12), (0.12, 0.56), (0.56, 0.56)] {
                outline(x, y, 0.32, 0.32);
            }
        }
        Icon::Search => {
            p.circle_stroke(at(0.42, 0.42), rect.width() * 0.28, s);
            line((0.64, 0.64), (0.92, 0.92));
        }
        Icon::Trash => {
            outline(0.25, 0.29, 0.5, 0.57);
            line((0.14, 0.2), (0.86, 0.2));
            line((0.37, 0.08), (0.63, 0.08));
            line((0.42, 0.4), (0.42, 0.72));
            line((0.58, 0.4), (0.58, 0.72));
        }
        Icon::Download => {
            line((0.5, 0.08), (0.5, 0.64));
            line((0.28, 0.45), (0.5, 0.67));
            line((0.5, 0.67), (0.72, 0.45));
            line((0.15, 0.7), (0.15, 0.88));
            line((0.15, 0.88), (0.85, 0.88));
            line((0.85, 0.88), (0.85, 0.7));
        }
        Icon::Check => {
            line((0.15, 0.5), (0.4, 0.76));
            line((0.4, 0.76), (0.86, 0.2));
        }
        Icon::Link => {
            outline(0.1, 0.35, 0.55, 0.55);
            line((0.42, 0.57), (0.88, 0.11));
            line((0.55, 0.11), (0.88, 0.11));
            line((0.88, 0.11), (0.88, 0.44));
        }
        Icon::Copy => {
            outline(0.1, 0.1, 0.6, 0.64);
            p.rect_filled(
                Rect::from_min_max(at(0.3, 0.3), at(0.9, 0.94)),
                0.,
                ui.visuals().widgets.inactive.bg_fill,
            );
            outline(0.3, 0.3, 0.6, 0.64);
        }
        Icon::Refresh => {
            let points = (0..25)
                .map(|i| {
                    let angle = 0.3 + i as f32 / 24. * 5.2;
                    at(0.5 + angle.cos() * 0.35, 0.5 + angle.sin() * 0.35)
                })
                .collect();
            p.add(egui::Shape::line(points, s));
            line((0.82, 0.18), (0.79, 0.48));
            line((0.79, 0.48), (0.53, 0.38));
        }
        Icon::Info => {
            p.circle_stroke(rect.center(), rect.width() * 0.43, s);
            p.circle_filled(at(0.5, 0.27), rect.width() * 0.06, color);
            line((0.5, 0.45), (0.5, 0.75));
        }
        Icon::Edit => {
            p.line_segment([at(0.18, 0.82), at(0.3, 0.55)], Stroke::new(5_f32, color));
            p.line_segment([at(0.3, 0.55), at(0.68, 0.17)], Stroke::new(5_f32, color));
            p.line_segment([at(0.68, 0.17), at(0.83, 0.32)], Stroke::new(5_f32, color));
            p.line_segment([at(0.83, 0.32), at(0.45, 0.7)], Stroke::new(5_f32, color));
            p.line_segment([at(0.45, 0.7), at(0.18, 0.82)], Stroke::new(5_f32, color));
        }
    }
}
