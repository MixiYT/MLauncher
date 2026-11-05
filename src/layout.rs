//! In-viewport panels on a shared grid. No native windows or floating window containers.
use eframe::egui::{self, Pos2, Rect, Sense, Vec2};
use serde::{Deserialize, Serialize};

pub const GRID: f32 = 8.;
pub const GAP: f32 = 16.;
pub const PAD: f32 = 16.;
pub const ROW: f32 = 32.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelId {
    Launcher,
    Accounts,
    Settings,
    Versions,
    Mods,
    About,
    Instance,
    Help,
}
impl PanelId {
    pub const ALL: [Self; 8] = [
        Self::Launcher,
        Self::Accounts,
        Self::Settings,
        Self::Versions,
        Self::Mods,
        Self::About,
        Self::Instance,
        Self::Help,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Launcher => "launcher",
            Self::Accounts => "accounts",
            Self::Settings => "settings",
            Self::Versions => "versions",
            Self::Mods => "mods",
            Self::About => "about",
            Self::Instance => "instance",
            Self::Help => "help",
        }
    }
    pub fn default_size(self) -> Vec2 {
        match self {
            Self::Launcher => Vec2::new(280., 224.),
            Self::Mods => Vec2::new(640., 480.),
            _ => Vec2::new(520., 480.),
        }
    }
    fn min_size(self) -> Vec2 {
        if self == Self::Launcher {
            Vec2::new(264., 224.)
        } else {
            Vec2::new(360., 384.)
        }
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelSizes {
    pub launcher: [f32; 2],
    pub accounts: [f32; 2],
    pub settings: [f32; 2],
    pub versions: [f32; 2],
    pub mods: [f32; 2],
    pub about: [f32; 2],
    pub instance: [f32; 2],
    pub help: [f32; 2],
    pub viewport: [f32; 2],
}
impl Default for PanelSizes {
    fn default() -> Self {
        let array = |id: PanelId| {
            let size = id.default_size();
            [size.x, size.y]
        };
        Self {
            launcher: array(PanelId::Launcher),
            accounts: array(PanelId::Accounts),
            settings: array(PanelId::Settings),
            versions: array(PanelId::Versions),
            mods: array(PanelId::Mods),
            about: array(PanelId::About),
            instance: array(PanelId::Instance),
            help: array(PanelId::Help),
            viewport: [1104., 720.],
        }
    }
}
impl PanelSizes {
    pub fn get(&self, id: PanelId) -> Vec2 {
        let raw = match id {
            PanelId::Launcher => self.launcher,
            PanelId::Accounts => self.accounts,
            PanelId::Settings => self.settings,
            PanelId::Versions => self.versions,
            PanelId::Mods => self.mods,
            PanelId::About => self.about,
            PanelId::Instance => self.instance,
            PanelId::Help => self.help,
        };
        let fallback = id.default_size();
        let min = id.min_size();
        Vec2::new(
            sanitize(raw[0], fallback.x, min.x),
            sanitize(raw[1], fallback.y, min.y),
        )
    }
    pub fn set(&mut self, id: PanelId, size: Vec2) {
        let min = id.min_size();
        let fallback = id.default_size();
        let value = [
            sanitize(size.x, fallback.x, min.x),
            sanitize(size.y, fallback.y, min.y),
        ];
        match id {
            PanelId::Launcher => self.launcher = value,
            PanelId::Accounts => self.accounts = value,
            PanelId::Settings => self.settings = value,
            PanelId::Versions => self.versions = value,
            PanelId::Mods => self.mods = value,
            PanelId::About => self.about = value,
            PanelId::Instance => self.instance = value,
            PanelId::Help => self.help = value,
        }
    }
    pub fn viewport_size(&self) -> Vec2 {
        Vec2::new(
            sanitize(self.viewport[0], 1104., 720.),
            sanitize(self.viewport[1], 720., 560.),
        )
    }
}
pub fn snap(value: f32) -> f32 {
    (value / GRID).round() * GRID
}
fn floor(value: f32) -> f32 {
    (value / GRID).floor() * GRID
}
fn sanitize(value: f32, fallback: f32, min: f32) -> f32 {
    if value.is_finite() {
        snap(value.clamp(min, 4096.))
    } else {
        fallback
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub workspace: Rect,
    pub left: Option<Rect>,
    pub launcher: Rect,
    pub detail: Option<Rect>,
}
impl Geometry {
    #[cfg(test)]
    pub fn new(screen: Rect, sizes: &PanelSizes, detail: Option<PanelId>) -> Self {
        Self::new_two(screen, sizes, None, detail)
    }
    /// Two independent side panels may be open at once: one on the left, one on the right.
    pub fn new_two(
        screen: Rect,
        sizes: &PanelSizes,
        left: Option<PanelId>,
        right: Option<PanelId>,
    ) -> Self {
        let available = Vec2::new(
            floor((screen.width() - 2. * GAP).max(64.)),
            floor((screen.height() - 2. * GAP).max(64.)),
        );
        let requested = sizes.get(PanelId::Launcher);
        let mut launcher_width = requested.x.min(available.x);
        let mut left_width = left.map(|id| sizes.get(id).x).unwrap_or(0.);
        let mut right_width = right.map(|id| sizes.get(id).x).unwrap_or(0.);
        let gaps = GAP * ((left.is_some() as u8 + right.is_some() as u8) as f32);
        let min_detail = 220.;
        let excess = (launcher_width + left_width + right_width + gaps - available.x).max(0.);
        if excess > 0. {
            let left_extra = (left_width - min_detail).max(0.);
            let right_extra = (right_width - min_detail).max(0.);
            let shrinkable = left_extra + right_extra;
            if shrinkable > 0. {
                let factor = excess.min(shrinkable) / shrinkable;
                left_width -= left_extra * factor;
                right_width -= right_extra * factor;
            }
            let still = (launcher_width + left_width + right_width + gaps - available.x).max(0.);
            launcher_width = (launcher_width - still).max(220.);
        }
        left_width = left_width.clamp(0., available.x);
        right_width = right_width.clamp(0., available.x);
        let left_height = left
            .map(|id| sizes.get(id).y.min(available.y))
            .unwrap_or(0.);
        let right_height = right
            .map(|id| sizes.get(id).y.min(available.y))
            .unwrap_or(0.);
        let launcher_height = requested.y.min(available.y);
        let total = Vec2::new(
            left_width
                + launcher_width
                + right_width
                + GAP * ((left.is_some() as u8 + right.is_some() as u8) as f32),
            launcher_height.max(left_height).max(right_height),
        );
        let start = Pos2::new(
            snap(screen.center().x - total.x * 0.5),
            snap(screen.center().y - total.y * 0.5),
        );
        let workspace = Rect::from_min_size(start, total);
        let centered = |height: f32| snap(screen.center().y - height * 0.5);
        let left_rect = left.map(|_| {
            Rect::from_min_size(
                Pos2::new(start.x, centered(left_height)),
                Vec2::new(left_width, left_height),
            )
        });
        let launcher_x = start.x + if left.is_some() { left_width + GAP } else { 0. };
        let launcher = Rect::from_min_size(
            Pos2::new(launcher_x, centered(launcher_height)),
            Vec2::new(launcher_width, launcher_height),
        );
        let detail = right.map(|_| {
            Rect::from_min_size(
                Pos2::new(launcher_x + launcher_width + GAP, centered(right_height)),
                Vec2::new(right_width, right_height),
            )
        });
        Self {
            workspace,
            left: left_rect,
            launcher,
            detail,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ResizeRequest {
    pub panel: PanelId,
    pub size: Vec2,
    pub finished: bool,
}

/// Fixed bounds are independent of content size; only an explicit edge drag changes them.
pub fn panel(
    ctx: &egui::Context,
    id: PanelId,
    rect: Rect,
    contents: impl FnOnce(&mut egui::Ui),
) -> Option<ResizeRequest> {
    panel_animated(ctx, id, rect, 1., contents)
}

pub fn panel_animated(
    ctx: &egui::Context,
    id: PanelId,
    rect: Rect,
    opacity: f32,
    contents: impl FnOnce(&mut egui::Ui),
) -> Option<ResizeRequest> {
    egui::Area::new(egui::Id::new(("inline-panel", id.key())))
        .fixed_pos(rect.min)
        .movable(false)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_opacity(opacity);
            ui.set_min_size(rect.size());
            ui.set_max_size(rect.size());
            crate::ui::paint_panel(ui, rect);
            let inner = rect.shrink(PAD);
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("content")
                    .max_rect(inner)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.set_clip_rect(inner);
            child.spacing_mut().item_spacing = Vec2::splat(GRID);
            child.spacing_mut().interact_size.y = ROW;
            child.spacing_mut().scroll.floating = false;
            child.spacing_mut().scroll.bar_width = GRID;
            child.spacing_mut().combo_height = 320.;
            child.spacing_mut().text_edit_width = inner.width();
            contents(&mut child);
            #[cfg(test)]
            ui.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new(("panel_bounds", id)),
                    (inner, child.min_rect()),
                )
            });
            resize_handles(ui, id, rect)
        })
        .inner
}

#[cfg(test)]
pub fn content_bounds(ctx: &egui::Context, id: PanelId) -> Option<(Rect, Rect)> {
    ctx.data(|data| data.get_temp(egui::Id::new(("panel_bounds", id))))
}
fn resize_handles(ui: &mut egui::Ui, panel: PanelId, rect: Rect) -> Option<ResizeRequest> {
    let corner = 14.;
    let edge = 6.;
    let handles = [
        (
            "top",
            Rect::from_min_max(
                rect.min + Vec2::new(corner, 0.),
                egui::pos2(rect.right() - corner, rect.top() + edge),
            ),
            Vec2::new(0., -2.),
            egui::CursorIcon::ResizeVertical,
        ),
        (
            "bottom",
            Rect::from_min_max(
                egui::pos2(rect.left() + corner, rect.bottom() - edge),
                rect.max - Vec2::new(corner, 0.),
            ),
            Vec2::new(0., 2.),
            egui::CursorIcon::ResizeVertical,
        ),
        (
            "left",
            Rect::from_min_max(
                rect.min + Vec2::new(0., corner),
                egui::pos2(rect.left() + edge, rect.bottom() - corner),
            ),
            Vec2::new(-2., 0.),
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            "right",
            Rect::from_min_max(
                egui::pos2(rect.right() - edge, rect.top() + corner),
                rect.max - Vec2::new(0., corner),
            ),
            Vec2::new(2., 0.),
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            "corner",
            Rect::from_min_size(rect.max - Vec2::splat(corner), Vec2::splat(corner)),
            Vec2::splat(2.),
            egui::CursorIcon::ResizeNwSe,
        ),
    ];
    let mut request = None;
    for (key, hit, factor, cursor) in handles {
        let id = ui.id().with(("resize", key));
        let state = id.with("base");
        let last = id.with("last");
        let response = ui
            .interact(hit, id, Sense::click_and_drag())
            .on_hover_cursor(cursor);
        if response.drag_started() {
            ui.data_mut(|data| data.insert_temp(state, rect.size()));
        }
        if response.double_clicked() {
            request = Some(ResizeRequest {
                panel,
                size: panel.default_size(),
                finished: true,
            });
        } else if response.dragged() {
            let base = ui
                .data(|data| data.get_temp::<Vec2>(state))
                .unwrap_or(rect.size());
            let delta = ui.input(|i| {
                i.pointer
                    .interact_pos()
                    .zip(i.pointer.press_origin())
                    .map(|(a, b)| a - b)
                    .unwrap_or_default()
            });
            let size = base + delta * factor;
            ui.data_mut(|data| data.insert_temp(last, size));
            request = Some(ResizeRequest {
                panel,
                size,
                finished: false,
            });
        } else if response.drag_stopped() {
            let size = ui
                .data(|data| data.get_temp::<Vec2>(last))
                .unwrap_or(rect.size());
            request = Some(ResizeRequest {
                panel,
                size,
                finished: true,
            });
        }
        if response.hovered() || response.dragged() {
            ui.painter()
                .rect_filled(hit, 2., ui.visuals().selection.bg_fill);
        }
    }
    let color = ui.visuals().window_stroke.color;
    for offset in [4., 8., 12.] {
        ui.painter().line_segment(
            [
                rect.max - Vec2::new(offset, 3.),
                rect.max - Vec2::new(3., offset),
            ],
            egui::Stroke::new(1_f32, color),
        );
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panels_share_grid_but_keep_independent_heights() {
        for size in [Vec2::new(720., 560.), Vec2::new(1920., 1080.)] {
            for id in PanelId::ALL
                .into_iter()
                .filter(|id| *id != PanelId::Launcher)
            {
                let g = Geometry::new(
                    Rect::from_min_size(Pos2::ZERO, size),
                    &PanelSizes::default(),
                    Some(id),
                );
                let right = g.detail.unwrap();
                assert_eq!(g.launcher.size(), PanelId::Launcher.default_size());
                assert!((g.launcher.center().y - right.center().y).abs() <= GRID);
                assert_eq!(right.left() - g.launcher.right(), GAP);
                assert!(g.workspace.left() >= 0. && g.workspace.right() <= size.x);
                assert!(g.workspace.bottom() <= size.y);
                for coordinate in [right.left(), right.top(), right.width(), right.height()] {
                    assert_eq!(coordinate % GRID, 0.);
                }
            }
        }
    }
    #[test]
    fn two_side_panels_keep_launcher_centered() {
        let size = Vec2::new(1600., 900.);
        let g = Geometry::new_two(
            Rect::from_min_size(Pos2::ZERO, size),
            &PanelSizes::default(),
            Some(PanelId::Settings),
            Some(PanelId::Versions),
        );
        let left = g.left.unwrap();
        let right = g.detail.unwrap();
        assert!(left.right() + GAP <= g.launcher.left());
        assert!(g.launcher.right() + GAP <= right.left());
        assert!(g.workspace.left() >= 0. && g.workspace.right() <= size.x);
        assert!((left.center().y - g.launcher.center().y).abs() <= GRID);
        assert!((right.center().y - g.launcher.center().y).abs() <= GRID);
    }
    #[test]
    fn sizes_are_independent_and_sanitized() {
        let mut sizes = PanelSizes::default();
        sizes.set(PanelId::Mods, Vec2::new(800., 600.));
        assert_eq!(sizes.get(PanelId::Mods), Vec2::new(800., 600.));
        assert_eq!(
            sizes.get(PanelId::Versions),
            PanelId::Versions.default_size()
        );
        sizes.set(PanelId::Accounts, Vec2::new(f32::NAN, -100.));
        assert_eq!(sizes.get(PanelId::Accounts), Vec2::new(520., 384.));
    }
    #[test]
    fn shrinking_viewport_does_not_erase_preference() {
        let mut sizes = PanelSizes::default();
        sizes.set(PanelId::Mods, Vec2::new(1000., 800.));
        let small = Geometry::new(
            Rect::from_min_size(Pos2::ZERO, Vec2::new(720., 560.)),
            &sizes,
            Some(PanelId::Mods),
        );
        assert!(small.detail.unwrap().width() < 1000.);
        let large = Geometry::new(
            Rect::from_min_size(Pos2::ZERO, Vec2::new(1920., 1080.)),
            &sizes,
            Some(PanelId::Mods),
        );
        assert_eq!(large.detail.unwrap().size(), Vec2::new(1000., 800.));
    }
}
