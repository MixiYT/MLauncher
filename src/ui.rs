use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    User,
    Folder,
    Refresh,
    Settings,
    Mods,
    Plus,
    Back,
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
    MyBuilds,
    Close,
    Minimize,
    Maximize,
    Restore,
    Versions,
    Resources,
    Exchange,
    Upload,
    Star,
    Shield,
    Swords,
    Pickaxe,
    Compass,
    Lightning,
}

impl crate::model::BuildIcon {
    pub const PRESETS: [(Self, &'static str); 13] = [
        (Self::None, "Без значка"),
        (Self::Box, "Коробка"),
        (Self::Block, "Блок"),
        (Self::Mods, "Моды"),
        (Self::Landscape, "Пейзаж"),
        (Self::Home, "Дом"),
        (Self::Folder, "Папка"),
        (Self::Star, "Звезда"),
        (Self::Shield, "Щит"),
        (Self::Swords, "Мечи"),
        (Self::Pickaxe, "Кирка"),
        (Self::Compass, "Компас"),
        (Self::Lightning, "Молния"),
    ];

    pub fn glyph(self) -> Icon {
        match self {
            Self::Box => Icon::MyBuilds,
            Self::Block => Icon::Builds,
            Self::Mods => Icon::Mods,
            Self::Landscape => Icon::Resources,
            Self::Home => Icon::Home,
            Self::Folder => Icon::Folder,
            Self::Star => Icon::Star,
            Self::Shield => Icon::Shield,
            Self::Swords => Icon::Swords,
            Self::Pickaxe => Icon::Pickaxe,
            Self::Compass => Icon::Compass,
            Self::Lightning => Icon::Lightning,
            Self::None => Icon::Close,
            Self::Custom => Icon::Resources,
        }
    }
}

pub fn paint_surface(
    ui: &egui::Ui,
    rect: Rect,
    radius: egui::CornerRadius,
    fill: Color32,
    stroke: Stroke,
) {
    ui.painter()
        .rect(rect, radius, fill, stroke, egui::StrokeKind::Inside);
    if crate::appearance::InterfaceStyle::current(ui.ctx())
        == crate::appearance::InterfaceStyle::LiquidGlass
    {
        ui.painter().rect_stroke(
            rect.shrink(1.5),
            radius,
            Stroke::new(1_f32, Color32::from_white_alpha(12)),
            egui::StrokeKind::Inside,
        );
        let inset = (radius.nw as f32).min(rect.width() * 0.25);
        ui.painter().line_segment(
            [
                rect.left_top() + Vec2::new(inset, 1.),
                rect.right_top() + Vec2::new(-inset, 1.),
            ],
            Stroke::new(1_f32, Color32::from_white_alpha(55)),
        );
    }
}

pub fn paint_panel(ui: &egui::Ui, rect: Rect) {
    let visuals = ui.visuals();
    if crate::appearance::InterfaceStyle::current(ui.ctx())
        != crate::appearance::InterfaceStyle::Standard
    {
        ui.painter().add(
            visuals
                .window_shadow
                .as_shape(rect, visuals.window_corner_radius),
        );
    }
    if crate::appearance::InterfaceStyle::current(ui.ctx())
        == crate::appearance::InterfaceStyle::LiquidGlass
    {
        crate::glass::panel(ui, rect, visuals.window_corner_radius.nw as f32);
    }
    paint_surface(
        ui,
        rect,
        visuals.window_corner_radius,
        visuals.panel_fill,
        visuals.window_stroke,
    );
}

pub fn icon_button(ui: &mut egui::Ui, icon: Icon, tooltip: &str) -> Response {
    icon_button_size(ui, icon, tooltip, Vec2::new(40., 32.))
}

pub fn build_thumbnail(
    ui: &mut egui::Ui,
    icon: crate::model::BuildIcon,
    instance: &std::path::Path,
    size: f32,
) {
    if icon != crate::model::BuildIcon::None {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        paint_build_icon(ui, icon, instance, rect);
    }
}

pub fn combo_text(ui: &egui::Ui, name: &str, icon_size: Option<f32>) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let indent = icon_size.map_or(0., |size| size + ui.spacing().icon_spacing);
    job.first_row_min_height = icon_size.unwrap_or_default();
    job.append(
        name,
        indent,
        egui::TextFormat {
            font_id: egui::TextStyle::Button.resolve(ui.style()),
            color: Color32::PLACEHOLDER,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job
}

pub fn paint_build_icon(
    ui: &egui::Ui,
    icon: crate::model::BuildIcon,
    instance: &std::path::Path,
    rect: Rect,
) {
    if icon == crate::model::BuildIcon::Custom {
        if let Some(texture) = crate::icons::custom_texture(ui.ctx(), instance) {
            egui::Image::new(&texture).paint_at(ui, rect);
            return;
        }
    }
    paint_icon(ui, icon.glyph(), rect, ui.visuals().text_color());
}

pub fn account_icons(ui: &mut egui::Ui, kind: crate::auth::AccountKind, ely_skins: bool) {
    use crate::auth::AccountKind;
    let (icon, label) = match kind {
        AccountKind::Offline => (Icon::User, "Офлайн"),
        AccountKind::ElyBy => (Icon::Ely, "Ely.by"),
        AccountKind::Microsoft => (Icon::Microsoft, "Microsoft"),
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.), Sense::hover());
    paint_icon(ui, icon, rect, ui.visuals().text_color());
    response.on_hover_text(label);
    if kind == AccountKind::Offline && ely_skins {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.), Sense::hover());
        paint_icon(ui, Icon::Ely, rect, ui.visuals().text_color());
        response.on_hover_text("Скины Ely.by");
    }
}

pub fn loader_icon(ui: &mut egui::Ui, loader: &str, official: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(26., 18.), Sense::hover());
    if official {
        if let Some(texture) = crate::icons::loader_texture(ui, loader) {
            let image = egui::Image::new(&texture)
                .max_size(rect.size())
                .maintain_aspect_ratio(true);
            let size = image.calc_size(rect.size(), Some(texture.size_vec2()));
            image.paint_at(ui, Rect::from_center_size(rect.center(), size));
            return;
        }
    }
    let rect = Rect::from_center_size(rect.center(), Vec2::splat(16.));
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * 16.;
    let stroke = Stroke::new(1.45_f32, ui.visuals().text_color());
    let line = |points: &[(f32, f32)], closed| {
        let points = points.iter().map(|&(x, y)| at(x, y)).collect();
        ui.painter().add(if closed {
            egui::Shape::closed_line(points, stroke)
        } else {
            egui::Shape::line(points, stroke)
        });
    };
    match loader {
        "fabric" => {
            line(
                &[(0.05, 0.33), (0.53, 0.08), (0.95, 0.29), (0.48, 0.55)],
                true,
            );
            line(
                &[
                    (0.05, 0.34),
                    (0.05, 0.66),
                    (0.48, 0.92),
                    (0.95, 0.64),
                    (0.95, 0.29),
                ],
                false,
            );
            line(&[(0.48, 0.55), (0.48, 0.92)], false);
        }
        "forge" => line(
            &[
                (0.02, 0.2),
                (0.98, 0.2),
                (0.8, 0.42),
                (0.59, 0.49),
                (0.59, 0.62),
                (0.75, 0.78),
                (0.28, 0.78),
                (0.43, 0.63),
                (0.43, 0.49),
                (0.18, 0.42),
            ],
            true,
        ),
        "neoforge" => {
            line(
                &[
                    (0.1, 0.15),
                    (0.42, 0.35),
                    (0.58, 0.35),
                    (0.9, 0.15),
                    (0.84, 0.65),
                    (0.5, 0.9),
                    (0.16, 0.65),
                ],
                true,
            );
            line(&[(0.27, 0.52), (0.38, 0.61)], false);
            line(&[(0.73, 0.52), (0.62, 0.61)], false);
        }
        "quilt" => {
            line(&[(0.5, 0.04), (0.96, 0.5), (0.5, 0.96), (0.04, 0.5)], true);
            line(&[(0.5, 0.04), (0.5, 0.96)], false);
            line(&[(0.04, 0.5), (0.96, 0.5)], false);
        }
        _ => paint_icon(ui, Icon::Builds, rect, stroke.color),
    }
}

pub fn icon_button_size(ui: &mut egui::Ui, icon: Icon, tooltip: &str, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let visuals = ui.style().interact(&response);
    paint_surface(
        ui,
        rect,
        visuals.corner_radius,
        visuals.bg_fill,
        visuals.bg_stroke,
    );
    paint_icon(
        ui,
        icon,
        Rect::from_center_size(rect.center(), Vec2::splat(21. * icon_scale(ui, &response))),
        visuals.fg_stroke.color,
    );
    response.on_hover_text(tooltip)
}

fn icon_scale(ui: &egui::Ui, response: &Response) -> f32 {
    if !ui.ctx().data(|data| {
        data.get_temp(egui::Id::new("animate_icons"))
            .unwrap_or(true)
    }) {
        return 1.;
    }
    let ctx = ui.ctx();
    let id = response.id.with("icon_press");
    let now = ctx.input(|input| input.time);
    if response.clicked() {
        ctx.data_mut(|data| data.insert_temp(id, now));
    }
    let press =
        ctx.animate_bool_with_time(id.with("down"), response.is_pointer_button_down_on(), 0.09);
    let click = ctx
        .data(|data| data.get_temp::<f64>(id))
        .map_or(1., |started| ((now - started) / 0.24).clamp(0., 1.) as f32);
    if click < 1. {
        ctx.request_repaint();
    }
    1. - press * 0.14 + (click * std::f32::consts::PI).sin() * 0.16
}

pub trait ComboAnimation {
    fn show_animated<R>(
        self,
        ui: &mut egui::Ui,
        menu: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<Option<R>>;
}

impl ComboAnimation for egui::ComboBox {
    fn show_animated<R>(
        self,
        ui: &mut egui::Ui,
        menu: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<Option<R>> {
        self.show_ui(ui, |ui| {
            let ctx = ui.ctx();
            if ctx.data(|data| {
                data.get_temp(egui::Id::new("animate_menus"))
                    .unwrap_or(true)
            }) {
                let id = ui.id().with("popup_animation");
                let frame = ctx.cumulative_pass_nr();
                let now = ctx.input(|input| input.time);
                let started = ctx.data_mut(|data| {
                    let (last_frame, start) = data.get_temp::<(u64, f64)>(id).unwrap_or((0, now));
                    let start = if frame > last_frame + 1 { now } else { start };
                    data.insert_temp(id, (frame, start));
                    start
                });
                let progress = ((now - started) / 0.16).clamp(0., 1.) as f32;
                if progress < 1. {
                    ctx.request_repaint();
                }
                ui.multiply_opacity(0.12 + 0.88 * (1. - (1. - progress).powi(3)));
            }
            menu(ui)
        })
    }
}

/// Shared fixed-height status row for all scrolling catalogs.
pub fn catalog_footer(
    ui: &mut egui::Ui,
    count: usize,
    loading: bool,
    failed: bool,
) -> egui::InnerResponse<bool> {
    ui.horizontal(|ui| {
        ui.set_min_height(32.);
        ui.label(format!("Загружено: {count}"));
        if loading {
            ui.spinner();
        }
        failed && ui.button("Повторить загрузку").clicked()
    })
}

pub fn catalog_thumbnail(
    ui: &mut egui::Ui,
    texture: Option<&egui::TextureHandle>,
    icon: Icon,
    size: f32,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().rect_filled(
        rect,
        ui.visuals().widgets.inactive.corner_radius,
        ui.visuals().selection.bg_fill,
    );
    if let Some(texture) = texture {
        let dimensions = texture.size_vec2();
        let image_rect = Rect::from_center_size(
            rect.center(),
            dimensions * (size / dimensions.x.max(dimensions.y).max(1.)),
        );
        ui.painter().image(
            texture.id(),
            image_rect,
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
            Color32::WHITE,
        );
    } else {
        paint_icon(ui, icon, rect.shrink(size * 0.2), ui.visuals().text_color());
    }
}

pub fn provider_button(ui: &mut egui::Ui, icon: Icon, text: &str) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.), Sense::click());
    let v = ui.style().interact(&response);
    paint_surface(ui, rect, v.corner_radius, v.bg_fill, v.bg_stroke);
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
    tab_button(ui, Some(icon), None, label, selected)
}

pub fn build_icon_tab(
    ui: &mut egui::Ui,
    icon: crate::model::BuildIcon,
    label: &str,
    selected: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.), Sense::click());
    let visuals = ui.style().interact_selectable(&response, selected);
    paint_surface(
        ui,
        rect,
        visuals.corner_radius,
        visuals.bg_fill,
        visuals.bg_stroke,
    );
    paint_icon(
        ui,
        icon.glyph(),
        Rect::from_center_size(rect.center(), Vec2::splat(18. * icon_scale(ui, &response))),
        visuals.fg_stroke.color,
    );
    response.on_hover_text(label)
}

pub fn tab_value<T: PartialEq>(
    ui: &mut egui::Ui,
    value: &mut T,
    selected: T,
    label: &str,
) -> Response {
    let mut response = tab_button(ui, None, None, label, *value == selected);
    if response.clicked() && *value != selected {
        *value = selected;
        response.mark_changed();
    }
    response
}

fn tab_button(
    ui: &mut egui::Ui,
    icon: Option<Icon>,
    texture: Option<&egui::TextureHandle>,
    label: &str,
    selected: bool,
) -> Response {
    use crate::appearance::InterfaceStyle;
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(13.),
        ui.visuals().text_color(),
    );
    let padding = if icon.is_some() { 42. } else { 24. };
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(galley.size().x + padding, 32.), Sense::click());
    let visuals = ui.style().interact(&response);
    let style = InterfaceStyle::current(ui.ctx());
    let enabled = ui.ctx().data(|d| {
        d.get_temp::<bool>(egui::Id::new("category_transitions"))
            .unwrap_or(true)
    });
    let phase = ui.ctx().animate_bool_with_time(
        response.id.with("selection"),
        selected,
        if enabled { 0.16 } else { 0. },
    );
    let fill = egui::lerp(
        egui::Rgba::from(visuals.bg_fill)..=egui::Rgba::from(ui.visuals().selection.bg_fill),
        phase,
    )
    .into();
    match style {
        InterfaceStyle::Material => {
            if response.hovered() || phase > 0.001 {
                paint_surface(
                    ui,
                    rect.shrink2(Vec2::new(0., 2.)),
                    10.into(),
                    fill,
                    Stroke::NONE,
                );
            }
            if phase > 0.001 {
                ui.painter().rect_filled(
                    Rect::from_min_max(
                        rect.left_bottom() + Vec2::new(8., -3.),
                        rect.right_bottom() - Vec2::new(8., 0.),
                    ),
                    2.,
                    ui.visuals().hyperlink_color.gamma_multiply(phase),
                );
            }
        }
        InterfaceStyle::Simple => {
            let color: Color32 = egui::lerp(
                egui::Rgba::from(visuals.bg_stroke.color)
                    ..=egui::Rgba::from(ui.visuals().hyperlink_color),
                phase,
            )
            .into();
            let stroke = Stroke::new(egui::lerp(visuals.bg_stroke.width..=1., phase), color);
            paint_surface(ui, rect, 2.into(), fill, stroke);
        }
        InterfaceStyle::Standard | InterfaceStyle::LiquidGlass => {
            paint_surface(ui, rect, visuals.corner_radius, fill, visuals.bg_stroke);
        }
    }
    let color = if selected && style != InterfaceStyle::Standard {
        ui.visuals().selection.stroke.color
    } else {
        visuals.fg_stroke.color
    };
    let text_x = if let Some(icon) = icon {
        let icon_rect = Rect::from_center_size(
            rect.left_center() + Vec2::new(16., 0.),
            Vec2::splat(18. * icon_scale(ui, &response)),
        );
        if let Some(texture) = texture {
            egui::Image::new(texture).paint_at(ui, icon_rect);
        } else {
            paint_icon(ui, icon, icon_rect, color);
        }
        30.
    } else {
        12.
    };
    ui.painter().galley(
        rect.left_center() + Vec2::new(text_x, -galley.size().y / 2.),
        galley,
        color,
    );
    response.on_hover_text(label)
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
        Icon::Star => {
            let points = (0..10)
                .map(|i| {
                    let angle = i as f32 * std::f32::consts::PI / 5. - std::f32::consts::FRAC_PI_2;
                    let radius = if i % 2 == 0 { 0.45 } else { 0.22 };
                    at(0.5 + angle.cos() * radius, 0.53 + angle.sin() * radius)
                })
                .collect();
            p.add(egui::Shape::closed_line(points, s));
        }
        Icon::Shield => {
            p.add(egui::Shape::closed_line(
                vec![
                    at(0.5, 0.09),
                    at(0.85, 0.24),
                    at(0.82, 0.58),
                    at(0.69, 0.78),
                    at(0.5, 0.92),
                    at(0.31, 0.78),
                    at(0.18, 0.58),
                    at(0.15, 0.24),
                ],
                s,
            ));
        }
        Icon::Swords => {
            line((0.16, 0.15), (0.87, 0.87));
            line((0.84, 0.15), (0.13, 0.87));
            line((0.61, 0.8), (0.8, 0.61));
            line((0.2, 0.61), (0.39, 0.8));
        }
        Icon::Pickaxe => {
            line((0.13, 0.88), (0.66, 0.35));
            p.add(egui::Shape::line(
                vec![
                    at(0.25, 0.12),
                    at(0.55, 0.18),
                    at(0.81, 0.43),
                    at(0.88, 0.73),
                ],
                s,
            ));
        }
        Icon::Compass => {
            p.circle_stroke(at(0.5, 0.5), rect.width() * 0.42, s);
            p.add(egui::Shape::closed_line(
                vec![
                    at(0.71, 0.28),
                    at(0.58, 0.58),
                    at(0.29, 0.72),
                    at(0.42, 0.42),
                ],
                s,
            ));
        }
        Icon::Lightning => {
            p.add(egui::Shape::closed_line(
                vec![
                    at(0.6, 0.08),
                    at(0.21, 0.55),
                    at(0.46, 0.55),
                    at(0.38, 0.92),
                    at(0.8, 0.42),
                    at(0.54, 0.42),
                ],
                s,
            ));
        }
        Icon::Versions => {
            for y in [0.22, 0.5, 0.78] {
                p.circle_filled(at(0.14, y), rect.width() * 0.04, color);
                line((0.3, y), (0.9, y));
            }
        }
        Icon::Resources => {
            // A texture tile, using the same stroke as the other navigation icons.
            outline(0.12, 0.12, 0.76, 0.76);
            for (x, y) in [
                (0.18, 0.18),
                (0.5, 0.18),
                (0.34, 0.34),
                (0.66, 0.34),
                (0.18, 0.5),
                (0.5, 0.5),
                (0.34, 0.66),
                (0.66, 0.66),
            ] {
                p.rect_filled(
                    Rect::from_min_max(at(x, y), at(x + 0.16, y + 0.16)),
                    0.,
                    color,
                );
            }
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
        Icon::MyBuilds => {
            outline(0.1, 0.34, 0.8, 0.52);
            line((0.2, 0.15), (0.8, 0.15));
            line((0.37, 0.53), (0.63, 0.53));
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
            p.add(egui::Shape::closed_line(
                vec![
                    at(0.08, 0.25),
                    at(0.38, 0.25),
                    at(0.5, 0.37),
                    at(0.92, 0.37),
                    at(0.92, 0.85),
                    at(0.08, 0.85),
                ],
                s,
            ));
            line((0.12, 0.47), (0.88, 0.47));
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
        Icon::Home => {
            line((0.1, 0.48), (0.5, 0.12));
            line((0.5, 0.12), (0.9, 0.48));
            outline(0.24, 0.45, 0.52, 0.43);
            outline(0.44, 0.65, 0.15, 0.23);
        }
        Icon::Settings => {
            let points = (0..64)
                .map(|i| {
                    let angle = i as f32 * std::f32::consts::TAU / 64.;
                    let radius = if i % 8 < 4 { 0.44 } else { 0.34 };
                    at(0.5 + angle.cos() * radius, 0.5 + angle.sin() * radius)
                })
                .collect();
            p.add(egui::Shape::closed_line(points, s));
            p.circle_stroke(rect.center(), rect.width() * 0.14, s);
        }
        Icon::Mods => {
            // Plug-in puzzle piece.
            p.add(egui::Shape::line(
                vec![
                    at(0.1, 0.35),
                    at(0.36, 0.35),
                    at(0.36, 0.22),
                    at(0.42, 0.12),
                    at(0.56, 0.12),
                    at(0.64, 0.22),
                    at(0.64, 0.35),
                    at(0.88, 0.35),
                    at(0.88, 0.6),
                    at(0.75, 0.6),
                    at(0.67, 0.67),
                    at(0.67, 0.77),
                    at(0.75, 0.84),
                    at(0.88, 0.84),
                    at(0.88, 0.92),
                    at(0.1, 0.92),
                    at(0.1, 0.35),
                ],
                s,
            ));
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
            p.add(egui::Shape::closed_line(
                vec![
                    at(0.14, 0.86),
                    at(0.23, 0.57),
                    at(0.69, 0.11),
                    at(0.89, 0.31),
                    at(0.43, 0.77),
                ],
                s,
            ));
            line((0.57, 0.23), (0.77, 0.43));
            line((0.23, 0.57), (0.43, 0.77));
        }
    }
}

/// Category content fades in on selection changes; independent from panel motion and layout.
pub fn category_fade(ui: &mut egui::Ui, key: impl std::hash::Hash, enabled: bool) {
    let id = ui.id().with("category_transition");
    let key = egui::Id::new(key);
    let now = ui.input(|i| i.time);
    let (previous, started) = ui.ctx().data_mut(|d| {
        let old = d.get_temp::<(egui::Id, f64)>(id).unwrap_or((key, now - 1.));
        let state = if old.0 != key { (key, now) } else { old };
        d.insert_temp(id, state);
        state
    });
    let _ = previous;
    if enabled {
        let t = ((now - started) / 0.18).clamp(0., 1.) as f32;
        let alpha = t * t * (3. - 2. * t);
        ui.multiply_opacity(alpha);
        if t < 1. {
            ui.ctx().request_repaint();
        }
    }
}

#[cfg(test)]
mod category_tests {
    use super::*;
    #[test]
    fn build_selector_reserves_icon_space_without_extra_glyphs() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for size in [None, Some(14.), Some(30.)] {
                    let mut job = combo_text(ui, "Сборка с длинным названием", size);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(110.);
                    let galley = ui.fonts(|fonts| fonts.layout_job(job));
                    assert_eq!(galley.job.text, "Сборка с длинным названием");
                    let first = &galley.rows[0].glyphs[0];
                    assert_eq!(first.chr, 'С');
                    assert!(first.pos.x >= size.unwrap_or_default());
                    assert!(galley.size().y >= size.unwrap_or_default());
                }
            });
        });
    }

    #[test]
    fn category_fades_by_time_without_changing_layout_and_can_be_disabled() {
        let ctx = egui::Context::default();
        let mut opacity = 0.;
        let mut sample = |time, key, enabled| {
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        category_fade(ui, key, enabled);
                        opacity = ui.opacity();
                        ui.label("Category content");
                    });
                },
            );
            opacity
        };
        assert_eq!(sample(0., 1, true), 1.);
        assert_eq!(sample(1., 2, true), 0.);
        let middle = sample(1.09, 2, true);
        assert!(middle > 0.4 && middle < 0.6);
        assert_eq!(sample(1.3, 2, true), 1.);
        assert_eq!(sample(1.4, 3, false), 1.);
    }
}
