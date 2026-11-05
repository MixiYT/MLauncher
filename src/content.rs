//! Embedded categories of “Versions and builds”; workers never block the UI.
use crate::{
    media,
    model::{Config, Profile, SortOrder},
    packs::{Entry, Provider, Service, Version},
    ui::{self, Icon},
};
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Packs,
    Resources,
    Maps,
    Exchange,
}
enum Message {
    Found(u64, usize, String, anyhow::Result<(String, Vec<Entry>)>),
    Versions(Entry, anyhow::Result<Vec<Version>>),
    Progress(String),
    Done(anyhow::Result<Option<Profile>>),
}
#[derive(Default)]
pub struct Content {
    section: Section,
    provider: Provider,
    query: String,
    sort: SortOrder,
    mc: String,
    cache_root: PathBuf,
    installed_instance: PathBuf,
    installed_resources: Vec<crate::packs::ResourceRecord>,
    refresh_resources: bool,
    entries: Vec<Entry>,
    browse: crate::browse::Browse,
    selected: Option<Entry>,
    versions: Vec<Version>,
    version: usize,
    rx: Option<mpsc::Receiver<Message>>,
    cancel: Option<Arc<AtomicBool>>,
    pub busy: bool,
    mutation: bool,
    status: String,
    error: bool,
    indexed: bool,
    confirm: bool,
    pending_import: Option<PathBuf>,
    thumbnails: media::Thumbnails,
}
impl Drop for Content {
    fn drop(&mut self) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::Relaxed);
        }
    }
}
impl Content {
    pub fn invalidate_resources(&mut self) {
        self.refresh_resources = true;
    }
    pub fn set_cache(&mut self, root: &Path) {
        self.cache_root = root.join("catalogs");
        self.thumbnails.set_cache(root.join("icons"));
    }

    pub fn warm(
        &mut self,
        ctx: &egui::Context,
        config: &Config,
        key: &str,
        section: Section,
    ) {
        self.set_section(section);
        self.mc = config.profiles[config.selected].game_version.clone();
        self.entries = crate::packs::cached_entries(&self.cache_file());
        self.thumbnails.request(
            self.entries.iter().map(|e| e.icon_url.clone()).collect(),
            ctx,
        );
        self.browse
            .sync(self.signature(config), ctx.input(|i| i.time), false);
        self.search(ctx, config, key);
    }
    fn signature(&self, config: &Config) -> String {
        format!(
            "{:?}|{}|{}|{}|{}",
            self.provider,
            self.section == Section::Resources,
            self.query.trim(),
            self.sort.modrinth(),
            config.profiles[config.selected].game_version
        )
    }
    fn cache_file(&self) -> PathBuf {
        self.cache_root.join(format!(
            "{:?}-{}-{}-{}.json",
            self.provider,
            self.section == Section::Resources,
            self.sort.modrinth(),
            if self.section == Section::Resources {
                self.mc.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
            } else {
                "all".into()
            }
        ))
    }
    pub fn mutation_busy(&self) -> bool {
        self.busy && self.mutation
    }
    pub fn set_section(&mut self, section: Section) {
        if section == self.section || self.busy {
            return;
        }
        self.section = section;
        self.browse.invalidate();
        self.entries.clear();
        self.selected = None;
        self.versions.clear();
        self.status.clear();
        self.confirm = false;
        if section == Section::Maps {
            self.provider = Provider::CurseForge;
        }
        if section == Section::Resources
            && !matches!(self.provider, Provider::Modrinth | Provider::CurseForge)
        {
            self.provider = Provider::Modrinth;
        }
    }
    pub fn drain(&mut self, ctx: &egui::Context) -> Option<Profile> {
        self.thumbnails.drain(ctx);
        let mut profile = None;
        if let Some(rx) = &self.rx {
            while let Ok(message) = rx.try_recv() {
                match message {
                    Message::Found(revision, offset, mc, result) => {
                        if !self.browse.finish(
                            revision,
                            offset,
                            result.as_ref().ok().map(|(_, entries)| entries.len()),
                        ) {
                            continue;
                        }
                        self.busy = false;
                        if matches!(self.section, Section::Resources | Section::Maps)
                            && mc != self.mc
                        {
                            continue;
                        }
                        match result {
                            Ok((query, entries)) => {
                                self.browse.resolved_query = Some(query);
                                self.thumbnails.request(
                                    entries.iter().map(|e| e.icon_url.clone()).collect(),
                                    ctx,
                                );
                                if offset == 0 {
                                    self.entries.clear();
                                }
                                for entry in entries {
                                    if !self
                                        .entries
                                        .iter()
                                        .any(|e| e.provider == entry.provider && e.id == entry.id)
                                    {
                                        self.entries.push(entry);
                                    }
                                }
                                self.status.clear();
                                self.error = false;
                            }
                            Err(e) => {
                                self.status = format!("{e:#}");
                                self.error = true;
                            }
                        }
                    }
                    Message::Versions(entry, result) => {
                        self.busy = false;
                        match result {
                            Ok(versions) => {
                                self.selected = Some(entry);
                                self.versions = versions;
                                self.version = 0;
                                self.confirm = false;
                                self.error = false;
                                self.status.clear();
                            }
                            Err(e) => {
                                self.status = format!("{e:#}");
                                self.error = true;
                            }
                        }
                    }
                    Message::Progress(status) => self.status = status,
                    Message::Done(result) => {
                        self.refresh_resources = true;
                        self.busy = false;
                        self.mutation = false;
                        self.cancel = None;
                        match result {
                            Ok(p) => {
                                profile = p;
                                self.status = "Готово".into();
                                self.error = false;
                                self.confirm = false;
                                self.pending_import = None;
                            }
                            Err(e) => {
                                self.status = format!("{e:#}");
                                self.error = true;
                            }
                        }
                    }
                }
            }
        }
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        profile
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        mutation: bool,
        job: impl FnOnce(mpsc::Sender<Message>, Arc<AtomicBool>, egui::Context) + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.rx = Some(rx);
        self.cancel = Some(cancel.clone());
        self.busy = true;
        self.mutation = mutation;
        self.error = false;
        self.status = "Загрузка…".into();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            job(tx, cancel, ctx.clone());
            ctx.request_repaint();
        });
    }
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        root: &Path,
        config: &Config,
        key: &str,
        blocked: bool,
    ) -> Option<Profile> {
        let completed = self.drain(ctx);
        if self.section == Section::Exchange {
            egui::ScrollArea::vertical()
                .id_salt("pack_exchange")
                .show(ui, |ui| {
                    self.exchange(ui, ctx, root, config, key, blocked);
                });
        } else {
            self.catalog(ui, ctx, root, config, key, blocked);
        }
        completed
    }
    fn status(&self, ui: &mut egui::Ui) {
        if self.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                if self.mutation && ui::icon_button(ui, Icon::Close, "Отменить операцию").clicked()
                {
                    if let Some(c) = &self.cancel {
                        c.store(true, Ordering::Relaxed);
                    }
                }
            });
        }
        if !self.status.is_empty() {
            ui.add(
                egui::Label::new(egui::RichText::new(&self.status).color(if self.error {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().text_color()
                }))
                .wrap(),
            );
        }
    }
    fn exchange(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        root: &Path,
        config: &Config,
        key: &str,
        blocked: bool,
    ) {
        ui.add_enabled_ui(!self.busy,|ui|{
            ui.label(egui::RichText::new(&config.profiles[config.selected].name).strong()).on_hover_text(config.instance_dir(root).display().to_string());
            ui.horizontal_wrapped(|ui|{
                ui::tab_value(ui, &mut self.indexed, false, "Полный ZIP").on_hover_text("Все моды внутри архива");
                ui::tab_value(ui, &mut self.indexed, true, "Индекс").on_hover_text("Моды скачиваются на другом компьютере по точным версиям и хешам");
                ui::icon_button(ui,Icon::Info,"Миры, аккаунты, ключи и журналы не экспортируются. Проверьте конфигурации модов на личные данные. Распространяйте файлы только с разрешения авторов. Мод без известного источника требует полного ZIP.");
            });
            ui.horizontal(|ui|{
                if ui.add_enabled_ui(!blocked,|ui|ui::icon_button(ui,Icon::Upload,"Экспортировать выбранную сборку")).inner.clicked(){
                    if let Some(path)=rfd::FileDialog::new().add_filter("MLauncher pack",&["zip"]).set_file_name("modpack.zip").save_file(){
                        let(profile,instance,indexed,key)=(config.profiles[config.selected].clone(),config.instance_dir(root),self.indexed,key.to_owned());
                        self.start(ctx,true,move|tx,cancel,ctx|{let result=Service::new(key).and_then(|s|s.export(&profile,&instance,&path,indexed,&cancel,&|p|{let _=tx.send(Message::Progress(p));ctx.request_repaint();})).map(|_|None);let _=tx.send(Message::Done(result));});
                    }
                }
                if ui.add_enabled_ui(!blocked,|ui|ui::icon_button(ui,Icon::Download,"Импортировать ZIP / mrpack отдельной сборкой")).inner.clicked(){
                    if let Some(path)=rfd::FileDialog::new().add_filter("Сборка",&["zip","mrpack"]).pick_file(){self.pending_import=Some(path);self.confirm=false;}
                }
            });
            if let Some(path)=self.pending_import.clone(){ui.group(|ui|{
                ui.add(egui::Label::new(path.file_name().unwrap_or_default().to_string_lossy()).truncate()).on_hover_text(path.display().to_string());
                ui.checkbox(&mut self.confirm,"Доверяю источнику сборки").on_hover_text("Будет создана отдельная сборка. Архив может содержать исполняемые Java-моды и ссылки для скачивания.");
                ui.horizontal(|ui|{
                    if ui.add_enabled_ui(self.confirm&&!blocked,|ui|ui::icon_button(ui,Icon::Check,"Начать импорт")).inner.clicked(){
                        let instances=config.game_root(root).join("instances");let key=key.to_owned();
                        self.start(ctx,true,move|tx,cancel,ctx|{let result=Service::new(key).and_then(|s|s.import(&path,&instances,&cancel,&|p|{let _=tx.send(Message::Progress(p));ctx.request_repaint();})).map(Some);let _=tx.send(Message::Done(result));});
                    }
                    if ui::icon_button(ui,Icon::Close,"Отменить импорт").clicked(){self.pending_import=None;}
                });
            });}
        });
        self.status(ui);
    }
    fn avatar(&self, ui: &mut egui::Ui, entry: &Entry, size: f32) {
        ui::catalog_thumbnail(
            ui,
            self.thumbnails.get(&entry.icon_url),
            if self.section == Section::Resources {
                Icon::Resources
            } else {
                Icon::Builds
            },
            size,
        );
    }
    fn search(&mut self, ctx: &egui::Context, config: &Config, key: &str) {
        if self.mutation_busy() {
            return;
        }
        let Some((revision, offset)) = self.browse.begin() else {
            return;
        };
        self.refresh_resources = true;
        let cache = self.cache_file();
        let cacheable =
            offset == 0 && self.query.trim().is_empty() && !self.cache_root.as_os_str().is_empty();
        let (provider, query, resources, mc, key, sort) = (
            self.provider,
            self.query.clone(),
            self.section == Section::Resources,
            config.profiles[config.selected].game_version.clone(),
            key.to_owned(),
            self.sort,
        );
        if offset == 0 {
            self.selected = None;
        }
        let resolved = self.browse.resolved_query.clone();
        let maps = self.section == Section::Maps;
        self.start(ctx, false, move |tx, _, _| {
            let result = Service::new(key).and_then(|s| {
                crate::browse::search_variants(&query, resolved, |q| {
                    if maps {
                        s.search_maps(q, &mc, sort, offset)
                    } else {
                        s.search_page(provider, q, resources, &mc, sort, offset)
                    }
                })
            });
            if cacheable && !maps {
                if let Ok((_, entries)) = &result {
                    if let Ok(bytes) =
                        serde_json::to_vec(&entries.iter().take(20).collect::<Vec<_>>())
                    {
                        if bytes.len() <= 512 * 1024 {
                            let _ = crate::downloads::atomic_write(&cache, &bytes);
                        }
                    }
                }
            }
            let _ = tx.send(Message::Found(revision, offset, mc, result));
        });
    }
    fn catalog(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        root: &Path,
        config: &Config,
        key: &str,
        blocked: bool,
    ) {
        let resources = self.section == Section::Resources;
        let maps = self.section == Section::Maps;
        let instance = config.instance_dir(root);
        if resources && (self.refresh_resources || self.installed_instance != instance) {
            self.installed_resources = crate::packs::resources(&instance);
            self.installed_instance = instance.clone();
            self.refresh_resources = false;
        }
        if resources || maps {
            let mc = config.profiles[config.selected].game_version.clone();
            if self.mc != mc {
                self.mc = mc;
                self.entries.clear();
                self.selected = None;
                self.versions.clear();
                self.status.clear();
            }
        }
        let mut search_now = false;
        ui.add_enabled_ui(!self.mutation_busy(),|ui|{
            ui.horizontal(|ui|{
                egui::ComboBox::from_id_salt("content_provider").width(120.).selected_text(self.provider.label()).show_ui(ui,|ui|{
                    for p in Provider::ALL {
                        if maps && p != Provider::CurseForge || resources&&!matches!(p,Provider::Modrinth|Provider::CurseForge){continue;}
                        if ui.selectable_value(&mut self.provider,p,p.label()).changed(){self.entries.clear();self.selected=None;self.status.clear();if !p.supports_sort(self.sort){self.sort=SortOrder::Relevance;}}
                    }
                });
                if resources {ui.add(egui::Label::new(&config.profiles[config.selected].name).truncate()).on_hover_text(format!("Minecraft {} · Ресурспак будет установлен в эту сборку. Включите его в настройках игры.",config.profiles[config.selected].game_version));}
                if matches!(self.provider,Provider::ATLauncher|Provider::Technic){ui::icon_button(ui,Icon::Info,"Экспериментальная поддержка. Старые патченные JAR, ручные загрузки и нестандартные типы модов поддерживаются не полностью. При несовместимости установка остановится без изменения существующих сборок.");}
            });
            let old_sort=self.sort;
            ui.horizontal(|ui|{
                egui::ComboBox::from_id_salt("content_sort").width((ui.available_width()-48.).max(80.)).selected_text(self.sort.label()).show_ui(ui,|ui|{
                    for sort in SortOrder::ALL {ui.add_enabled_ui(self.provider.supports_sort(sort),|ui|{ui.selectable_value(&mut self.sort,sort,sort.label());});}
                });
                if !matches!(self.provider,Provider::Modrinth|Provider::CurseForge){ui::icon_button(ui,Icon::Info,"Сортировка среди найденных результатов. Недоступные режимы не поддерживаются API этого каталога.");}
            });
            if old_sort!=self.sort {search_now=true;}
            ui.horizontal(|ui|{
                let response=ui.add_sized([(ui.available_width()-48.).max(64.),32.],egui::TextEdit::singleline(&mut self.query).hint_text(if maps {"Найти карту"} else if resources{"Найти ресурспак"}else{"Найти сборку"}));
                if response.changed(){ctx.request_repaint_after(std::time::Duration::from_millis(360));}
                if ui::icon_button(ui,Icon::Search,"Найти").clicked()||(response.lost_focus()&&ui.input(|i|i.key_pressed(egui::Key::Enter))){search_now=true;}
            });
            if let Some(entry)=self.selected.clone(){
                ui.group(|ui|{
                    ui.horizontal(|ui|{
                        self.avatar(ui,&entry,36.);
                        ui.add_sized([(ui.available_width()-48.).max(60.),32.],egui::Label::new(egui::RichText::new(&entry.name).strong()).truncate());
                        if ui::icon_button(ui,Icon::Close,"Закрыть выбор версии").clicked(){self.selected=None;}
                    });
                    if self.versions.is_empty(){ui.label("Нет доступных версий");return;}
                    egui::ComboBox::from_id_salt("pack_version").width(ui.available_width()).truncate().selected_text(&self.versions[self.version].name).show_ui(ui,|ui|{for(i,v)in self.versions.iter().enumerate(){ui.selectable_value(&mut self.version,i,&v.name);}});
                    ui.horizontal(|ui|{
                        if !resources {ui.checkbox(&mut self.confirm,"Доверяю источнику").on_hover_text("Сборка содержит исполняемые Java-моды. Продолжайте только для доверенного источника.");}
                        if ui.add_enabled_ui((resources||self.confirm)&&!blocked&&!self.busy,|ui|ui::icon_button(ui,Icon::Download,if resources{"Скачать ресурспак"}else{"Установить отдельной сборкой"})).inner.clicked(){
                            let(version,instances,instance,key)=(self.versions[self.version].id.clone(),config.game_root(root).join("instances"),config.instance_dir(root),key.to_owned());
                            self.start(ctx,true,move|tx,cancel,ctx|{let result=Service::new(key).and_then(|s|if resources{s.resource(&entry,&version,&instance,&cancel).map(|_|None)}else{s.install(&entry,&version,&instances,&cancel,&|p|{let _=tx.send(Message::Progress(p));ctx.request_repaint();}).map(Some)});let _=tx.send(Message::Done(result));});
                        }
                    });
                });
            }
        });
        let now = ctx.input(|i| i.time);
        if self
            .browse
            .sync(self.signature(config), now, !self.entries.is_empty())
        {
            self.entries.clear();
            self.selected = None;
            self.versions.clear();
        }
        if search_now {
            self.browse.reset(now);
            self.entries.clear();
        }
        if self.browse.pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.browse.due(now) && !self.mutation_busy() {
            self.search(ctx, config, key);
        }
        if self.mutation_busy() || self.error {
            self.status(ui);
        }
        let mut chosen = None;
        let mut list = egui::ScrollArea::vertical();
        if std::mem::take(&mut self.browse.reset_scroll) {
            list = list.vertical_scroll_offset(0.);
        }
        let area = list
            .id_salt(("content_results", resources))
            .auto_shrink([false, false])
            .min_scrolled_height(0.)
            .max_height((ui.available_height() - 40.).max(0.))
            .show(ui, |ui| {
                ui.add_enabled_ui(!self.mutation_busy(), |ui| {
                    for entry in &self.entries {
                        ui.push_id((&entry.id, entry.provider.label()), |ui| {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    self.avatar(ui, entry, 56.);
                                    let width = (ui.available_width() - 96.).max(40.);
                                    ui.vertical(|ui| {
                                        ui.set_width(width);
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&entry.name).strong(),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(&entry.name);
                                        let mut byline = if entry.author.is_empty() {
                                            entry.provider.label().to_owned()
                                        } else {
                                            entry.author.clone()
                                        };
                                        if entry.downloads > 0 {
                                            byline.push_str(&format!(
                                                " · ↓ {}",
                                                crate::compact_number(entry.downloads)
                                            ));
                                        }
                                        ui.add(
                                            egui::Label::new(egui::RichText::new(byline).small())
                                                .truncate(),
                                        );
                                    });
                                    let installed = maps
                                        && crate::packs::map_path(&instance, entry)
                                            .join("level.dat")
                                            .is_file()
                                        || resources
                                            && self.installed_resources.iter().any(|r| {
                                                r.provider == entry.provider
                                                    && r.project == entry.id
                                            });
                                    if ui
                                        .add_enabled_ui(
                                            !installed && !blocked && !self.busy,
                                            |ui| {
                                                ui::icon_button(
                                                    ui,
                                                    if installed {
                                                        Icon::Check
                                                    } else {
                                                        Icon::Download
                                                    },
                                                    if installed {
                                                        "Установлено"
                                                    } else if resources {
                                                        "Скачать совместимую версию"
                                                    } else {
                                                        "Выбрать версию и скачать"
                                                    },
                                                )
                                            },
                                        )
                                        .inner
                                        .clicked()
                                    {
                                        chosen = Some(entry.clone());
                                    }
                                    if ui::icon_button(ui, Icon::Link, "Страница проекта").clicked()
                                    {
                                        if let Ok(url) = reqwest::Url::parse(&entry.website) {
                                            if url.scheme() == "https" {
                                                ui.ctx().open_url(egui::OpenUrl::new_tab(
                                                    url.to_string(),
                                                ));
                                            }
                                        }
                                    }
                                });
                                let summary = summary(&entry.description, 220);
                                if !summary.is_empty() {
                                    ui.add(egui::Label::new(summary).wrap())
                                        .on_hover_text(summary_text(&entry.description));
                                }
                            });
                        });
                    }
                });
            });
        if !self.busy
            && self.selected.is_none()
            && self.browse.more
            && !self.browse.pending()
            && !self.browse.failed
            && crate::browse::near_end(
                area.state.offset.y,
                area.inner_rect.height(),
                area.content_size.y,
            )
        {
            self.search(ctx, config, key);
        }
        if ui::catalog_footer(ui, self.entries.len(), self.busy, self.browse.failed).inner {
            self.search(ctx, config, key);
        }
        if let Some(entry) = chosen {
            if maps || resources {
                let (key, mc, instance) = (
                    key.to_owned(),
                    config.profiles[config.selected].game_version.clone(),
                    config.instance_dir(root),
                );
                self.start(ctx, true, move |tx, cancel, _| {
                    let result = Service::new(key).and_then(|s| {
                        if maps {
                            s.download_map(&entry, &mc, &instance, &cancel)?;
                            return Ok(None);
                        }
                        let versions = s.versions(&entry, true, &mc)?;
                        let version = versions
                            .first()
                            .ok_or_else(|| anyhow::anyhow!("Нет совместимой версии"))?;
                        s.resource(&entry, &version.id, &instance, &cancel)?;
                        Ok(None)
                    });
                    let _ = tx.send(Message::Done(result));
                });
                return;
            }
            let (key, mc) = (
                key.to_owned(),
                config.profiles[config.selected].game_version.clone(),
            );
            self.start(ctx, false, move |tx, _, _| {
                let result = Service::new(key).and_then(|s| s.versions(&entry, resources, &mc));
                let _ = tx.send(Message::Versions(entry, result));
            });
        }
    }
}

fn summary_text(description: &str) -> String {
    static TAGS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let plain = TAGS
        .get_or_init(|| regex::Regex::new("<[^>]*>").expect("static regex"))
        .replace_all(description, " ");
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn summary(description: &str, limit: usize) -> String {
    let plain = summary_text(description);
    let mut value = plain.chars().take(limit).collect::<String>();
    if plain.chars().count() > limit {
        value.push('…');
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cards_and_exchange_fit_narrow_panels() {
        let root = tempfile::tempdir().unwrap();
        let config = Config::default();
        for section in [Section::Packs, Section::Resources, Section::Exchange] {
            for selected in [false, true] {
                let ctx = egui::Context::default();
                let mut state = Content::default();
                state.set_section(section);
                state.mc = config.profiles[config.selected].game_version.clone();
                for i in 0..8 {
                    state.entries.push(Entry{id:format!("fixture-{i}"),name:"A long example project title with a description".into(),description:"A useful project description, wrapping naturally across a narrow card without hiding the icon or action buttons.".into(),provider:Provider::Modrinth,website:String::new(),icon_url:String::new(),author:"Example author".into(),downloads:123456,updated:"2026-01-02".into(),published:"2025-12-31".into()});
                }
                if selected {
                    state.selected = Some(state.entries[0].clone());
                    state.versions.push(Version {
                        id: "fixture".into(),
                        name: "1.20.1 — a long compatible version title".into(),
                    });
                }
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
                                crate::layout::PanelId::Versions,
                                egui::Rect::from_min_size(
                                    egui::pos2(16., 16.),
                                    egui::vec2(360., 384.),
                                ),
                                |ui| {
                                    state.ui(ui, ctx, root.path(), &config, "", false);
                                },
                            );
                        },
                    );
                }
                let (inner, used) =
                    crate::layout::content_bounds(&ctx, crate::layout::PanelId::Versions).unwrap();
                assert!(
                    used.right() <= inner.right() + 1.,
                    "Card overflow: {used:?} / {inner:?}"
                );
                assert!(
                    used.bottom() <= inner.bottom() + 1.,
                    "Panel overflow: {used:?} / {inner:?}"
                );
            }
        }
    }
    #[test]
    fn category_switch_clears_incompatible_selection() {
        let mut state = Content::default();
        state.provider = Provider::Technic;
        state.confirm = true;
        state.set_section(Section::Resources);
        assert!(state.provider == Provider::Modrinth);
        assert!(!state.confirm);
        state.set_section(Section::Exchange);
        assert!(state.section == Section::Exchange);
    }
    #[test]
    fn descriptions_are_short_plain_text() {
        assert_eq!(
            summary("<b>Pack</b>\n description", 100),
            "Pack description"
        );
        assert_eq!(summary("Сборка", 3), "Сбо…");
    }
}
