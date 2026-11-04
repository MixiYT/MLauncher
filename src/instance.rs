//! Per-instance settings and local content management.
use crate::{
    java,
    model::{self, BuildIcon, Config, Profile},
    ui::{self, Icon},
};
use anyhow::{Context, Result};
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    General,
    Java,
    Mods,
    Resources,
    Worlds,
    Screenshots,
}
impl Tab {
    const ALL: [Self; 6] = [
        Self::General,
        Self::Java,
        Self::Mods,
        Self::Resources,
        Self::Worlds,
        Self::Screenshots,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::General => "Общее",
            Self::Java => "Java и запуск",
            Self::Mods => "Моды",
            Self::Resources => "Ресурспаки",
            Self::Worlds => "Миры",
            Self::Screenshots => "Скриншоты",
        }
    }
    fn folder(self) -> &'static str {
        match self {
            Self::Mods => "mods",
            Self::Resources => "resourcepacks",
            Self::Worlds => "saves",
            Self::Screenshots => "screenshots",
            _ => "",
        }
    }
}
pub enum Action {
    Save,
    Close,
    Catalog(bool),
    Changed,
}
pub struct Editor {
    pub directory: String,
    pub draft: Profile,
    java: java::JavaSettings,
    custom_java: bool,
    pub tab: Tab,
    entries: Vec<PathBuf>,
    refresh: bool,
    rename: Option<PathBuf>,
    rename_text: String,
    remove: Option<PathBuf>,
    thumbnails: std::collections::HashMap<PathBuf, Option<egui::TextureHandle>>,
    preview: Option<(PathBuf, egui::TextureHandle)>,
    status: String,
    error: bool,
}
impl Editor {
    pub fn new(profile: &Profile, defaults: &java::JavaSettings) -> Self {
        Self {
            directory: profile.directory.clone(),
            draft: profile.clone(),
            java: profile.java.clone().unwrap_or_else(|| defaults.clone()),
            custom_java: profile.java.is_some(),
            tab: Tab::General,
            entries: vec![],
            refresh: true,
            rename: None,
            rename_text: String::new(),
            remove: None,
            thumbnails: Default::default(),
            preview: None,
            status: String::new(),
            error: false,
        }
    }
    pub fn save(&mut self, config: &mut Config, root: &Path) -> Result<()> {
        anyhow::ensure!(
            !self.draft.name.trim().is_empty(),
            "Введите название сборки"
        );
        anyhow::ensure!(
            self.java.min_memory <= self.java.max_memory,
            "Минимум памяти больше максимума"
        );
        let index = config
            .profiles
            .iter()
            .position(|p| p.directory == self.directory)
            .context("Сборка больше не существует")?;
        self.draft.name = self.draft.name.trim().to_owned();
        self.draft.java = self.custom_java.then(|| self.java.clone());
        config.profiles[index] = self.draft.clone();
        let renamed = config.sync_directory(index, root);
        self.directory = config.profiles[index].directory.clone();
        self.draft.directory = self.directory.clone();
        config.save(root)?;
        self.status = if renamed {
            "Настройки сохранены"
        } else {
            "Настройки сохранены. Папка сборки сохранена по прежнему пути."
        }
        .into();
        self.error = false;
        Ok(())
    }
    pub fn report(&mut self, result: Result<()>) {
        match result {
            Ok(()) => {
                self.status = "Готово".into();
                self.error = false;
                self.refresh = true;
            }
            Err(e) => {
                self.status = format!("{e:#}");
                self.error = true;
            }
        }
    }
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        instance: &Path,
        blocked: bool,
    ) -> Option<Action> {
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui::icon_button(ui, Icon::Back, "К списку сборок").clicked() {
                action = Some(Action::Close);
            }
            ui.add(egui::Label::new(RichText::new(&self.draft.name).strong()).truncate());
            if ui
                .add_enabled(!blocked, egui::Button::new("Сохранить"))
                .clicked()
            {
                action = Some(Action::Save);
            }
        });
        ui.horizontal_wrapped(|ui| {
            for tab in Tab::ALL {
                if ui
                    .selectable_value(&mut self.tab, tab, tab.label())
                    .changed()
                {
                    self.refresh = true;
                    self.rename = None;
                    self.remove = None;
                    self.preview = None;
                }
            }
        });
        ui.separator();
        if blocked {
            ui.label("Изменения доступны после остановки игры и завершения установки.");
        }
        if !self.status.is_empty() {
            ui.add(
                egui::Label::new(RichText::new(&self.status).color(if self.error {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().text_color()
                }))
                .wrap(),
            );
        }
        egui::ScrollArea::vertical().id_salt(("instance_editor", &self.directory, self.tab.folder()))
            .auto_shrink([false, false]).show(ui, |ui| {
            if self.tab == Tab::General {
                ui.add_enabled_ui(!blocked, |ui| {
                    ui.label("Иконка сборки");
                    ui.horizontal_wrapped(|ui| {
                        for (preset, label) in BuildIcon::PRESETS {
                            if ui::icon_tab(ui, preset.glyph(), label, self.draft.icon == preset)
                                .clicked()
                            {
                                self.draft.icon = preset;
                            }
                        }
                    });
                    ui.label("Название сборки");
                    ui.add(egui::TextEdit::singleline(&mut self.draft.name).desired_width(f32::INFINITY));
                });
                ui.label(format!("Minecraft {} · {}", self.draft.game_version, self.draft.loader));
                ui.add(egui::Label::new(instance.display().to_string()).wrap());
                if ui.button("Открыть папку сборки").clicked() { self.report(open::that(instance).map_err(Into::into)); }
                ui.label("Настройки Java относятся только к этой сборке. Моды, ресурспаки, миры и скриншоты находятся в соседних разделах.");
            } else if self.tab == Tab::Java {
                ui.add_enabled_ui(!blocked, |ui| {
                    ui.checkbox(&mut self.custom_java, "Свои настройки Java и запуска");
                    ui.add_enabled_ui(self.custom_java, |ui| {
                        ui.label("Java (пусто — автоматический подбор)");
                        ui.add(egui::TextEdit::singleline(&mut self.java.executable).hint_text("Авто").desired_width(f32::INFINITY));
                        if ui.button("Выбрать java.exe…").clicked() {
                            if let Some(path) = rfd::FileDialog::new().add_filter("Java", &["exe"]).pick_file() { self.java.executable = path.display().to_string(); }
                        }
                        ui.checkbox(&mut self.java.auto_download, "Автоматически скачивать нужную Java");
                        egui::Grid::new("instance_java").num_columns(2).show(ui, |ui| {
                            ui.label("Память минимум, МиБ"); ui.add(egui::DragValue::new(&mut self.java.min_memory).range(128..=65536).speed(128)); ui.end_row();
                            ui.label("Память максимум, МиБ"); ui.add(egui::DragValue::new(&mut self.java.max_memory).range(512..=65536).speed(128)); ui.end_row();
                            ui.label("Ширина окна"); ui.add(egui::DragValue::new(&mut self.java.window_width).range(320..=7680)); ui.end_row();
                            ui.label("Высота окна"); ui.add(egui::DragValue::new(&mut self.java.window_height).range(240..=4320)); ui.end_row();
                        });
                        ui.checkbox(&mut self.java.fullscreen, "Полноэкранный режим");
                        egui::ComboBox::from_id_salt("instance_gc").selected_text(match self.java.collector {
                            java::Collector::Default => "GC: по умолчанию", java::Collector::G1 => "G1", java::Collector::Zgc => "ZGC", java::Collector::Shenandoah => "Shenandoah",
                        }).show_ui(ui, |ui| {
                            for (value, text) in [(java::Collector::Default, "По умолчанию"), (java::Collector::G1, "G1"), (java::Collector::Zgc, "ZGC"), (java::Collector::Shenandoah, "Shenandoah")] { ui.selectable_value(&mut self.java.collector, value, text); }
                        });
                        ui.label("JVM-аргументы, один на строку");
                        ui.add(egui::TextEdit::multiline(&mut self.java.extra_args).desired_width(f32::INFINITY).desired_rows(4));
                    });
                });
            } else {
                let folder = instance.join(self.tab.folder());
                ui.horizontal_wrapped(|ui| {
                    if ui::icon_button(ui, Icon::Refresh, "Обновить список").clicked() { self.refresh = true; }
                    if ui::icon_button(ui, Icon::Folder, "Открыть папку").clicked() {
                        self.report(std::fs::create_dir_all(&folder).map_err(Into::into).and_then(|_| open::that(&folder).map_err(Into::into)));
                    }
                    if matches!(self.tab, Tab::Mods | Tab::Resources) {
                        if ui.button("Каталог").clicked() { action = Some(Action::Catalog(self.tab == Tab::Resources)); }
                        if ui.add_enabled(!blocked, egui::Button::new("Импорт…")).clicked() {
                            let ext = if self.tab == Tab::Mods { "jar" } else { "zip" };
                            if let Some(paths) = rfd::FileDialog::new().add_filter("Minecraft", &[ext]).pick_files() {
                                for path in paths {
                                    if self.tab == Tab::Mods { self.report(crate::manual::import(&path, instance)); }
                                    else { self.report(import_file(&folder, &path, ext)); }
                                }
                                action = Some(Action::Changed);
                            }
                        }
                    }
                });
                if self.refresh {
                    self.entries = list(&folder, self.tab);
                    self.thumbnails.clear();
                    self.refresh = false;
                }
                if let Some(path) = self.remove.clone() {
                    ui.group(|ui| {
                        ui.add(egui::Label::new(format!("Убрать «{}» в корзину сборки?", filename(&path))).wrap());
                        ui.horizontal_wrapped(|ui| {
                            if ui.add_enabled(!blocked, egui::Button::new("Убрать")).clicked() {
                                self.report(trash(instance, &folder, &path)); self.remove = None; self.preview = None;
                                action = Some(Action::Changed);
                            }
                            if ui.button("Отмена").clicked() { self.remove = None; }
                        });
                    });
                }
                if let Some(path) = self.rename.clone() {
                    ui.label("Новое имя папки мира");
                    ui.add(egui::TextEdit::singleline(&mut self.rename_text).desired_width(f32::INFINITY));
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!blocked, egui::Button::new("Переименовать")).clicked() {
                            self.report(rename_world(&folder, &path, &self.rename_text)); self.rename = None;
                        }
                        if ui.button("Отмена").clicked() { self.rename = None; }
                    });
                }
                if let Some((path, texture)) = &self.preview {
                    ui.add(egui::Label::new(filename(path)).truncate());
                    ui.add(egui::Image::new(texture).max_width(ui.available_width()).max_height(260.).maintain_aspect_ratio(true));
                }
                if self.entries.is_empty() { ui.label("Пока пусто"); }
                for path in self.entries.clone() {
                    ui.push_id(&path, |ui| {
                        ui.group(|ui| {
                            let width = ui.available_width();
                            let button_width = if width < 260. { 32. } else { 40. };
                            let row_button = |ui: &mut egui::Ui, icon, label| {
                                ui::icon_button_size(ui, icon, label, egui::vec2(button_width, 32.))
                            };
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, 42.),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                if ui.add_enabled_ui(!blocked, |ui| row_button(ui, Icon::Trash, "Убрать в корзину сборки")).inner.clicked() { self.remove = Some(path.clone()); }
                                if self.tab == Tab::Screenshots && row_button(ui, Icon::Resources, "Предпросмотр").clicked() {
                                    let result = std::fs::read(&path).map_err(anyhow::Error::from)
                                        .and_then(|bytes| crate::media::decode_image(&bytes, 4096, 4096));
                                    match result { Ok(image) => self.preview = Some((path.clone(), ctx.load_texture(path.display().to_string(), image, egui::TextureOptions::LINEAR))), Err(e) => self.report(Err(e)) }
                                }
                                if row_button(ui, Icon::Folder, "Открыть").clicked() { self.report(open::that(&path).map_err(Into::into)); }
                                if self.tab == Tab::Worlds && ui.add_enabled_ui(!blocked, |ui| row_button(ui, Icon::Edit, "Имя папки…")).inner.clicked() {
                                    self.rename_text = filename(&path); self.rename = Some(path.clone());
                                }
                                if self.tab == Tab::Screenshots && row_button(ui, Icon::Download, "Сохранить копию…").clicked() {
                                    if let Some(dest) = rfd::FileDialog::new().set_file_name(filename(&path)).save_file() { self.report(std::fs::copy(&path, dest).map(|_| ()).map_err(Into::into)); }
                                }
                                if self.tab == Tab::Mods && ui.add_enabled_ui(!blocked, |ui| {
                                    let disabled = path.extension().is_some_and(|ext| ext == "disabled");
                                    row_button(ui, if disabled { Icon::Check } else { Icon::Close }, if disabled { "Включить мод" } else { "Отключить мод" })
                                }).inner.clicked() {
                                    self.report(toggle_mod(&folder, &path)); action = Some(Action::Changed);
                                }
                                let width = ui.available_width().max(0.);
                                ui.allocate_ui_with_layout(egui::vec2(width, 42.), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    let fallback = match self.tab {
                                        Tab::Mods => Icon::Mods,
                                        Tab::Worlds => Icon::Builds,
                                        _ => Icon::Resources,
                                    };
                                    if width >= 48. {
                                        let texture = if ui.is_rect_visible(ui.available_rect_before_wrap()) {
                                            self.thumbnails.entry(path.clone()).or_insert_with(|| {
                                                thumbnail(&path, self.tab).ok().map(|image| ctx.load_texture(
                                                    path.display().to_string(), image, egui::TextureOptions::LINEAR,
                                                ))
                                            }).as_ref()
                                        } else { None };
                                        ui::catalog_thumbnail(ui, texture, fallback, 36.);
                                    }
                                    ui.add(egui::Label::new(filename(&path)).truncate()).on_hover_text(path.display().to_string());
                                });
                            });
                        });
                    });
                }
                if instance.join("launcher/trash").is_dir() && ui.button("Открыть корзину сборки").clicked() { self.report(open::that(instance.join("launcher/trash")).map_err(Into::into)); }
            }
        });
        action
    }
}
fn thumbnail(path: &Path, tab: Tab) -> Result<egui::ColorImage> {
    use std::io::Read;

    let bytes = match tab {
        Tab::Worlds => read_thumbnail(std::fs::File::open(path.join("icon.png"))?)?,
        Tab::Screenshots => read_thumbnail(std::fs::File::open(path)?)?,
        Tab::Resources if path.is_dir() => {
            read_thumbnail(std::fs::File::open(path.join("pack.png"))?)?
        }
        Tab::Mods | Tab::Resources => {
            let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
            let icon = if tab == Tab::Resources {
                "pack.png".to_owned()
            } else {
                let mut icon = None;
                for manifest in ["fabric.mod.json", "quilt.mod.json"] {
                    if let Ok(file) = zip.by_name(manifest) {
                        let bytes = read_thumbnail(file.take(256 * 1024))?;
                        let json: serde_json::Value = serde_json::from_slice(&bytes)?;
                        let value = if manifest == "fabric.mod.json" {
                            &json["icon"]
                        } else {
                            &json["quilt_loader"]["metadata"]["icon"]
                        };
                        icon = value
                            .as_str()
                            .or_else(|| {
                                value
                                    .as_object()?
                                    .iter()
                                    .filter_map(|(size, path)| {
                                        Some((
                                            size.parse::<u32>().ok()?.abs_diff(64),
                                            path.as_str()?,
                                        ))
                                    })
                                    .min_by_key(|(distance, _)| *distance)
                                    .map(|(_, path)| path)
                            })
                            .map(str::to_owned);
                        if icon.is_some() {
                            break;
                        }
                    }
                }
                icon.or_else(|| {
                    zip.file_names()
                        .find(|name| {
                            let name = name.to_ascii_lowercase();
                            name.ends_with("/icon.png")
                                || name.ends_with("/logo.png")
                                || name == "logo.png"
                        })
                        .map(str::to_owned)
                })
                .context("Нет иконки мода")?
            };
            let file = zip.by_name(&icon)?;
            read_thumbnail(file)?
        }
        _ => anyhow::bail!("Нет миниатюры"),
    };
    crate::media::decode_image(&bytes, 72, 72)
}

fn read_thumbnail(reader: impl std::io::Read) -> Result<Vec<u8>> {
    use std::io::Read;
    const LIMIT: u64 = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    reader.take(LIMIT + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= LIMIT, "Изображение слишком большое");
    Ok(bytes)
}

fn filename(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
fn list(folder: &Path, tab: Tab) -> Vec<PathBuf> {
    let mut result: Vec<_> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let kind = entry.file_type().ok()?;
            if kind.is_symlink() {
                return None;
            }
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let keep = match tab {
                Tab::Worlds => kind.is_dir(),
                Tab::Mods => kind.is_file() && matches!(ext.as_str(), "jar" | "disabled"),
                Tab::Resources => kind.is_dir() || ext == "zip",
                Tab::Screenshots => {
                    kind.is_file() && matches!(ext.as_str(), "png" | "jpg" | "jpeg")
                }
                _ => false,
            };
            keep.then_some(path)
        })
        .collect();
    result.sort();
    result
}
fn checked_child(folder: &Path, path: &Path) -> Result<PathBuf> {
    let folder = folder.canonicalize()?;
    anyhow::ensure!(
        !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "Ссылки не поддерживаются"
    );
    let path = path.canonicalize()?;
    anyhow::ensure!(
        path.parent() == Some(folder.as_path()),
        "Файл находится вне папки сборки"
    );
    Ok(path)
}
fn trash(instance: &Path, folder: &Path, path: &Path) -> Result<()> {
    anyhow::ensure!(
        folder.canonicalize()?.starts_with(instance.canonicalize()?),
        "Папка находится вне сборки"
    );
    let path = checked_child(folder, path)?;
    let trash = instance
        .join("launcher/trash")
        .join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&trash)?;
    anyhow::ensure!(
        trash.canonicalize()?.starts_with(instance.canonicalize()?),
        "Корзина находится вне сборки"
    );
    std::fs::rename(
        &path,
        trash.join(path.file_name().context("Нет имени файла")?),
    )?;
    Ok(())
}
fn rename_world(folder: &Path, path: &Path, name: &str) -> Result<()> {
    anyhow::ensure!(
        model::is_safe_directory(name) && model::directory_from_name(name) == name,
        "Недопустимое имя папки"
    );
    let path = checked_child(folder, path)?;
    let target = folder.join(name);
    anyhow::ensure!(!target.exists(), "Такая папка уже существует");
    std::fs::rename(path, target)?;
    Ok(())
}
fn toggle_mod(folder: &Path, path: &Path) -> Result<()> {
    let path = checked_child(folder, path)?;
    let target = if path.extension().is_some_and(|ext| ext == "disabled") {
        path.with_extension("")
    } else {
        path.with_file_name(format!("{}.disabled", filename(&path)))
    };
    anyhow::ensure!(!target.exists(), "Такой файл уже существует");
    std::fs::rename(path, target)?;
    Ok(())
}
fn import_file(folder: &Path, source: &Path, ext: &str) -> Result<()> {
    anyhow::ensure!(
        source
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case(ext)),
        "Неподдерживаемый тип файла"
    );
    std::fs::create_dir_all(folder)?;
    let target = folder.join(source.file_name().context("Нет имени файла")?);
    let mut output = tempfile::NamedTempFile::new_in(folder)?;
    std::io::copy(&mut std::fs::File::open(source)?, &mut output)?;
    output.as_file().sync_all()?;
    output.persist_noclobber(target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_management_keeps_worlds_and_mods_recoverable() {
        let root = tempfile::tempdir().unwrap();
        let instance = root.path();
        let worlds = instance.join("saves");
        let world = worlds.join("World");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"world").unwrap();
        assert!(rename_world(&worlds, &world, "../outside").is_err());
        rename_world(&worlds, &world, "Renamed").unwrap();
        trash(instance, &worlds, &worlds.join("Renamed")).unwrap();
        let saved = std::fs::read_dir(instance.join("launcher/trash"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join("Renamed/level.dat");
        assert_eq!(std::fs::read(saved).unwrap(), b"world");
        let mods = instance.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let jar = mods.join("example.jar");
        std::fs::write(&jar, b"jar").unwrap();
        toggle_mod(&mods, &jar).unwrap();
        toggle_mod(&mods, &mods.join("example.jar.disabled")).unwrap();
        assert_eq!(std::fs::read(jar).unwrap(), b"jar");
    }
    #[test]
    fn editor_tabs_fit_a_narrow_panel() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let config = Config::default();
        let mut editor = Editor::new(&config.profiles[0], &config.java);
        editor.draft.name =
            "A long instance name that must never push controls outside the panel".into();
        for tab in Tab::ALL {
            editor.tab = tab;
            for _ in 0..3 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800., 600.),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        crate::layout::panel(
                            ctx,
                            crate::layout::PanelId::Instance,
                            egui::Rect::from_min_size(egui::pos2(16., 16.), egui::vec2(360., 480.)),
                            |ui| {
                                editor.ui(ui, ctx, root.path(), false);
                            },
                        );
                    },
                );
            }
            let (inner, used) =
                crate::layout::content_bounds(&ctx, crate::layout::PanelId::Instance).unwrap();
            assert!(
                used.right() <= inner.right() + 1.,
                "editor overflow: {used:?} / {inner:?}"
            );
            assert!(
                used.bottom() <= inner.bottom() + 1.,
                "editor overflow: {used:?} / {inner:?}"
            );
        }
    }
    #[test]
    fn settings_are_isolated_and_survive_renaming() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        let original = config.java.max_memory;
        let mut editor = Editor::new(&config.profiles[0], &config.java);
        editor.draft.name = "Renamed".into();
        editor.custom_java = true;
        editor.java.max_memory = 8192;
        editor.save(&mut config, root.path()).unwrap();
        let loaded = Config::load(root.path()).unwrap();
        assert_eq!(loaded.profiles[0].java.as_ref().unwrap().max_memory, 8192);
        assert_eq!(loaded.java.max_memory, original);
        assert_eq!(loaded.profiles[0].directory, "Renamed");
    }
}
