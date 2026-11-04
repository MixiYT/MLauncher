#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod api;
mod appearance;
mod assistant;
mod auth;
mod avatars;
mod browse;
mod content;
mod downloads;
mod effects;
mod game;
mod icons;
mod instance;
mod java;
mod layout;
mod manual;
mod media;
mod microsoft;
mod model;
mod overlay;
mod packs;
mod scenery;
mod secrets;
#[cfg(windows)]
mod tray;
mod ui;
mod variant;
mod versions;
mod voxel;

use eframe::egui::{self, Color32, RichText, Vec2};
use model::{Category, Config, InstalledMod, Profile, Project, SearchOptions, SortOrder, Source};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
};
use ui::{icon_button, Icon};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Home,
    Accounts,
    Settings,
    Versions,
    Mods,
    About,
    Logs,
    Help,
    BuildEdit,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum VersionTab {
    Catalog,
    Builds,
    Packs,
    Exchange,
}
impl VersionTab {
    fn group(self, tabs: model::LibraryTabs) -> model::LibraryGroup {
        use model::LibraryGroup;
        match self {
            Self::Catalog => LibraryGroup::Catalog,
            Self::Builds => LibraryGroup::MyBuilds,
            Self::Packs => tabs.packs,
            Self::Exchange => tabs.exchange,
        }
    }
}

enum Event {
    Search(u64, usize, anyhow::Result<(String, Vec<Project>)>),
    Progress(String),
    Installed(anyhow::Result<usize>),
    Updated(anyhow::Result<bool>),
    Updates(anyhow::Result<Vec<(Source, String, Option<String>)>>),
    Auth(anyhow::Result<auth::Account>),
    SessionRefreshed(auth::AccountKind, String, anyhow::Result<auth::Account>),
    AuthProgress(String),
    Device(microsoft::DevicePrompt),
    Logout(anyhow::Result<()>),
    Versions(anyhow::Result<(Vec<versions::MinecraftVersion>, String)>),
    Categories(Source, anyhow::Result<Vec<Category>>),
    JavaFound(Vec<java::Installation>),
    Game(u64, game::Event),
    Analysis(anyhow::Result<String>),
    Wallpaper(anyhow::Result<String>),
}

struct Launcher {
    overlay_revision: u64,
    sessions: Vec<GameSession>,
    next_session: u64,
    root: PathBuf,
    config: Config,
    page: Page,
    secondary_page: Option<Page>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    busy: bool,
    status: String,
    error: bool,
    source: Source,
    query: String,
    projects: Vec<Project>,
    installed: Vec<InstalledMod>,
    installed_tab: bool,
    resource_tab: bool,
    maps_tab: bool,
    manual_trust: bool,
    key: String,
    searched: bool,
    mod_browse: browse::Browse,
    remove_candidate: Option<InstalledMod>,
    account_kind: Option<auth::AccountKind>,
    account_name: String,
    account_password: String,
    account_totp: String,
    offline_skins: bool,
    auth_busy: bool,
    auth_status: String,
    cancel_auth: Option<Sender<()>>,
    device: Option<microsoft::DevicePrompt>,
    version_list: Vec<versions::MinecraftVersion>,
    version_query: String,
    versions_loading: bool,
    versions_status: String,
    instance_version: Option<String>,
    instance_loader: String,
    instance_name: String,
    instance_force_new: bool,
    search_options: SearchOptions,
    categories: std::collections::HashMap<Source, Vec<Category>>,
    categories_loading: std::collections::HashSet<Source>,
    category_error: String,
    thumbnails: media::Thumbnails,
    background: media::Background,
    applied_appearance: appearance::Appearance,
    show_appearance: bool,
    background_path_input: String,
    installed_icons_requested: bool,
    layout_dirty_at: Option<f64>,
    java_installations: Vec<java::Installation>,
    java_searching: bool,
    game_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    game_running: bool,
    #[cfg(windows)]
    tray: tray::Tray,
    game_logs: std::collections::VecDeque<String>,
    launch_status: String,
    mod_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    updates: std::collections::HashMap<String, Option<String>>,
    version_tab: VersionTab,
    editing_build: Option<usize>,
    edit_build_name: String,
    delete_build: Option<usize>,
    delete_build_folder: bool,
    avatars: avatars::Avatars,
    build_editor: Option<instance::Editor>,
    ai_key: zeroize::Zeroizing<String>,
    ai_question: String,
    ai_report: String,
    ai_answer: String,
    ai_busy: bool,
    wallpaper_busy: bool,
    content: content::Content,
    resource_catalog: content::Content,
    brand: Option<egui::TextureHandle>,
    panel_motion: std::collections::HashMap<layout::PanelId, PanelMotion>,
}

#[derive(Clone, Copy)]
struct PanelMotion {
    page: Page,
    from_rect: egui::Rect,
    to_rect: egui::Rect,
    from_alpha: f32,
    to_alpha: f32,
    started: f64,
}
impl PanelMotion {
    fn new(page: Page, rect: egui::Rect, now: f64, fade: bool) -> Self {
        Self {
            page,
            from_rect: rect,
            to_rect: rect,
            from_alpha: if fade { 0. } else { 1. },
            to_alpha: 1.,
            started: now,
        }
    }
    fn sample(self, now: f64) -> (egui::Rect, f32) {
        let phase = ((now - self.started) / 0.20).clamp(0., 1.) as f32;
        let eased = 1. - (1. - phase).powi(3);
        (
            self.from_rect.lerp_towards(&self.to_rect, eased),
            egui::lerp(self.from_alpha..=self.to_alpha, eased),
        )
    }
    fn active(self, now: f64) -> bool {
        now - self.started < 0.20
    }
    fn target(&mut self, rect: egui::Rect, alpha: f32, now: f64, animate: bool) {
        if !animate {
            self.from_rect = rect;
            self.to_rect = rect;
            self.from_alpha = alpha;
            self.to_alpha = alpha;
            self.started = now - 1.;
        } else if self.to_rect != rect || self.to_alpha != alpha {
            (self.from_rect, self.from_alpha) = self.sample(now);
            self.to_rect = rect;
            self.to_alpha = alpha;
            self.started = now;
        }
    }
}

struct GameSession {
    id: u64,
    name: String,
    instance: PathBuf,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    running: bool,
}

impl Launcher {
    fn content_mutation_busy(&self) -> bool {
        self.content.mutation_busy() || self.resource_catalog.mutation_busy()
    }
    fn new(
        ctx: &egui::Context,
        root: PathBuf,
        mut config: Config,
        start_workers: bool,
        startup_warning: String,
    ) -> Self {
        variant::configure(&mut config);
        if config.microsoft_client_id.is_empty() {
            config.microsoft_client_id = std::env::var("MICROSOFT_CLIENT_ID").unwrap_or_default();
        }
        config.appearance.apply(ctx);
        let applied_appearance = config.appearance.clone();
        let background_path_input = config.appearance.background.clone();
        let (ai_key, cf_key) = secrets::load_api_keys(&root);
        let background = media::Background::with_cache(&root);
        let brand = media::decode_image(icons::bytes(), 32, 32)
            .ok()
            .map(|image| ctx.load_texture("brand-icon", image, egui::TextureOptions::LINEAR));
        // Расшифрованные сессии не сохраняются в config.json.
        let mut stored = secrets::load(&root);
        for account in &mut config.accounts {
            if let Some(session) = stored.remove(&secrets::key(account.kind, &account.uuid)) {
                account.session = Some(session);
            }
        }
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            overlay_revision: 0,
            sessions: vec![],
            next_session: 1,
            root,
            config,
            page: Page::Home,
            secondary_page: None,
            tx,
            rx,
            busy: false,
            status: startup_warning,
            error: false,
            source: Source::Modrinth,
            query: String::new(),
            projects: vec![],
            installed: vec![],
            installed_tab: false,
            resource_tab: false,
            maps_tab: false,
            manual_trust: false,
            key: std::env::var("CURSEFORGE_API_KEY").unwrap_or(cf_key),
            searched: false,
            mod_browse: Default::default(),
            remove_candidate: None,
            account_kind: None,
            account_name: String::new(),
            account_password: String::new(),
            account_totp: String::new(),
            offline_skins: true,
            auth_busy: false,
            auth_status: String::new(),
            cancel_auth: None,
            device: None,
            version_list: vec![],
            version_query: String::new(),
            versions_loading: false,
            versions_status: String::new(),
            instance_version: None,
            instance_loader: "fabric".into(),
            instance_name: String::new(),
            instance_force_new: false,
            search_options: Default::default(),
            categories: Default::default(),
            categories_loading: Default::default(),
            category_error: String::new(),
            thumbnails: Default::default(),
            background,
            applied_appearance,
            show_appearance: true,
            background_path_input,
            installed_icons_requested: false,
            layout_dirty_at: None,
            java_installations: vec![],
            java_searching: false,
            game_cancel: None,
            game_running: false,
            #[cfg(windows)]
            tray: Default::default(),
            game_logs: Default::default(),
            launch_status: String::new(),
            mod_cancel: None,
            updates: Default::default(),
            version_tab: VersionTab::Catalog,
            editing_build: None,
            edit_build_name: String::new(),
            delete_build: None,
            delete_build_folder: false,
            avatars: Default::default(),
            build_editor: None,
            panel_motion: Default::default(),
            ai_key: zeroize::Zeroizing::new(ai_key),
            ai_question: String::new(),
            ai_report: String::new(),
            ai_answer: String::new(),
            ai_busy: false,
            wallpaper_busy: false,
            content: Default::default(),
            resource_catalog: Default::default(),
            brand,
        };
        let removed = app.config.prune_missing(&app.root);
        if removed > 0 {
            app.config.ensure_default();
            let _ = std::fs::create_dir_all(app.config.instance_dir(&app.root));
            let _ = app.config.save(&app.root);
        }
        app.refresh_installed();
        if start_workers {
            if !variant::LITE {
                app.content.warm(
                    ctx,
                    &app.root,
                    &app.config,
                    &app.key,
                    content::Section::Packs,
                );
                app.resource_catalog.warm(
                    ctx,
                    &app.root,
                    &app.config,
                    &app.key,
                    content::Section::Resources,
                );
            }
            app.thumbnails.set_cache(app.root.join("cache/icons"));
            app.load_versions(ctx);
            app.background.load(
                &app.config.appearance.background,
                app.config.appearance.animate,
                ctx,
            );
            app.refresh_sessions(ctx);
        }
        app
    }

    /// Проверяет сохранённые сессии в фоне: Ely.by обновляет токен, Microsoft — refresh token.
    fn refresh_sessions(&self, ctx: &egui::Context) {
        for account in &self.config.accounts {
            let Some(session) = &account.session else {
                continue;
            };
            if account.kind == auth::AccountKind::Offline {
                continue;
            }
            let cloned = auth::Session {
                access_token: zeroize::Zeroizing::new(session.access_token.to_string()),
                client_token: zeroize::Zeroizing::new(session.client_token.to_string()),
                refresh_token: session
                    .refresh_token
                    .as_ref()
                    .map(|token| zeroize::Zeroizing::new(token.to_string())),
            };
            let kind = account.kind;
            let uuid = account.uuid.clone();
            let name = account.name.clone();
            let ely_skins = account.ely_skins;
            let client_id = self.config.microsoft_client_id.clone();
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let (_keep, cancel) = mpsc::channel::<()>();
                let result = match kind {
                    auth::AccountKind::ElyBy => {
                        auth::refresh_elyby(&cloned).map(|session| auth::Account {
                            name,
                            uuid: uuid.clone(),
                            kind,
                            ely_skins,
                            session: Some(session),
                        })
                    }
                    auth::AccountKind::Microsoft => {
                        microsoft::refresh(&client_id, &cloned, &cancel, &|_| {})
                    }
                    auth::AccountKind::Offline => unreachable!("offline-сессии не обновляются"),
                };
                let _ = tx.send(Event::SessionRefreshed(kind, uuid, result));
                ctx.request_repaint();
            });
        }
    }

    fn navigate(&mut self, page: Page) {
        use zeroize::Zeroize;
        self.account_password.zeroize();
        self.account_totp.zeroize();
        if page == Page::Home {
            self.secondary_page = None;
        }
        self.page = page;
    }
    fn toggle_page(&mut self, page: Page) {
        if self.secondary_page == Some(page) {
            self.secondary_page = None;
            return;
        }
        if self.page == page {
            let previous = self.secondary_page.take().unwrap_or(Page::Home);
            self.navigate(previous);
            return;
        }
        if self.page != Page::Home && self.secondary_page.is_none() {
            self.secondary_page = Some(self.page);
            self.page = page;
        } else {
            self.navigate(page);
        }
    }
    fn toggle_library(&mut self, tab: VersionTab) {
        let tabs = self.config.library_tabs;
        let open = self.page == Page::Versions || self.secondary_page == Some(Page::Versions);
        let same_group = self.version_tab.group(tabs) == tab.group(tabs);
        if !open || !same_group {
            self.version_tab = tab;
            self.instance_version = None;
            if open {
                return;
            }
        }
        self.toggle_page(Page::Versions);
    }

    fn close_pages(&mut self) {
        self.navigate(Page::Home);
    }
    fn panel_id_of(&self, page: Page) -> layout::PanelId {
        use layout::PanelId;
        match page {
            Page::Home => PanelId::Launcher,
            Page::Accounts => PanelId::Accounts,
            Page::Settings => PanelId::Settings,
            Page::Versions if self.instance_version.is_some() => PanelId::Instance,
            Page::Versions => PanelId::Versions,
            Page::Mods => PanelId::Mods,
            Page::About => PanelId::About,
            Page::Logs => PanelId::About,
            Page::Help => PanelId::Help,
            Page::BuildEdit => PanelId::Instance,
        }
    }
    fn panel_id(&self) -> layout::PanelId {
        use layout::PanelId;
        match self.page {
            Page::Home => PanelId::Launcher,
            Page::Accounts => PanelId::Accounts,
            Page::Settings => PanelId::Settings,
            Page::Versions => {
                if self.instance_version.is_some() {
                    PanelId::Instance
                } else {
                    PanelId::Versions
                }
            }
            Page::Mods => PanelId::Mods,
            Page::About => PanelId::About,
            Page::Logs => PanelId::About,
            Page::Help => PanelId::Help,
            Page::BuildEdit => PanelId::Instance,
        }
    }
    fn resize_panel(
        &mut self,
        request: layout::ResizeRequest,
        screen: egui::Rect,
        ctx: &egui::Context,
    ) {
        let detail = if self.page == Page::Home {
            None
        } else {
            Some(self.panel_id())
        };
        let left = self
            .secondary_page
            .filter(|page| *page != Page::Home && *page != self.page)
            .map(|page| self.panel_id_of(page));
        self.config.panels.set(request.panel, request.size);
        // Clamp explicit user resizes, but never overwrite saved sizes merely because the app window got smaller.
        let geometry = layout::Geometry::new_two(screen, &self.config.panels, left, detail);
        if request.panel == layout::PanelId::Launcher {
            self.config
                .panels
                .set(layout::PanelId::Launcher, geometry.launcher.size());
        }
        if let Some((id, rect)) = detail.zip(geometry.detail) {
            if request.panel == id {
                self.config.panels.set(id, rect.size());
            }
        }
        if let Some((id, rect)) = left.zip(geometry.left) {
            if request.panel == id {
                self.config.panels.set(id, rect.size());
            }
        }
        self.layout_dirty_at = Some(ctx.input(|i| i.time));
        if request.finished {
            self.persist_layout();
        }
        ctx.request_repaint();
    }
    fn persist_layout(&mut self) {
        if self.layout_dirty_at.take().is_some() {
            if let Err(error) = self.config.save(&self.root) {
                self.result(Err(error), "");
            }
        }
    }
    fn find_java(&mut self, ctx: &egui::Context) {
        if self.java_searching {
            return;
        }
        self.java_searching = true;
        let (root, tx, ctx) = (
            self.config.game_root(&self.root).join("shared"),
            self.tx.clone(),
            ctx.clone(),
        );
        std::thread::spawn(move || {
            let _ = tx.send(Event::JavaFound(java::discover(&root)));
            ctx.request_repaint();
        });
    }
    fn start_game(&mut self, ctx: &egui::Context) {
        if !self.sessions.is_empty() && !self.config.multi_session.enabled {
            return;
        }
        if self.sessions.len() >= self.config.multi_session.limit.clamp(1, 8) {
            self.result(Err(anyhow::anyhow!("Достигнут лимит сессий")), "");
            return;
        }
        let instance = self.config.instance_dir(&self.root);
        if self.sessions.iter().any(|s| s.instance == instance) {
            self.result(
                Err(anyhow::anyhow!(
                    "Сборка уже запущена. Выберите другую сборку или создайте отдельную копию."
                )),
                "",
            );
            return;
        }
        if self.busy || self.content_mutation_busy() {
            self.result(
                Err(anyhow::anyhow!("Дождитесь завершения установки модов")),
                "",
            );
            return;
        }
        let Some(account) = self.config.accounts.get(self.config.selected_account) else {
            self.navigate(Page::Accounts);
            return;
        };
        let identity =
            match game::Identity::from_account(account, self.config.microsoft_client_id.clone()) {
                Ok(value) => value,
                Err(e) => {
                    self.result(Err(e), "");
                    self.navigate(Page::Accounts);
                    return;
                }
            };
        self.save();
        if self.sessions.is_empty() {
            self.game_logs.clear();
        }
        self.launch_status = "Подготовка…".into();
        self.error = false;
        self.status.clear();
        let mut native_overlay = if !variant::LITE
            && self.config.multi_session.native_overlay
            && self.on_game_start() != model::OnGameStart::Close
        {
            Some(overlay::Options::installed(
                self.key.clone(),
                self.config.appearance.accent,
                self.config.appearance.light_background,
                format!("{:?}", self.config.appearance.effect),
            ))
        } else {
            None
        };
        if let Some(options) = &native_overlay {
            if let Err(error) = overlay::check(&options.dll) {
                self.result(Err(error), "");
                native_overlay = None;
            }
        }
        let request = game::Request {
            overlay: native_overlay,
            profile: self.config.profiles[self.config.selected].clone(),
            instance: self.config.instance_dir(&self.root),
            storage: self.config.game_root(&self.root).join("shared"),
            settings: self.config.multi_session.java(
                self.config.profiles[self.config.selected]
                    .java
                    .as_ref()
                    .unwrap_or(&self.config.java),
            ),
            identity,
        };
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.game_cancel = Some(cancel.clone());
        let id = self.next_session;
        self.next_session += 1;
        self.sessions.push(GameSession {
            id,
            name: request.profile.name.clone(),
            instance: request.instance.clone(),
            cancel: cancel.clone(),
            running: false,
        });
        #[cfg(windows)]
        let restore_window = if self.on_game_start() == model::OnGameStart::Minimize {
            self.tray.window()
        } else {
            0
        };
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = game::run(request, cancel, |event| {
                let _ = tx.send(Event::Game(id, event));
                ctx.request_repaint();
            })
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Game(id, game::Event::Finished(result)));
            #[cfg(windows)]
            tray::restore(restore_window);
            ctx.request_repaint();
        });
    }
    fn result(&mut self, result: anyhow::Result<()>, message: &str) {
        match result {
            Ok(()) => {
                self.status = message.into();
                self.error = false;
            }
            Err(e) => {
                self.status = format!("{e:#}");
                self.error = true;
            }
        }
    }
    fn save(&mut self) {
        let result = self
            .config
            .save(&self.root)
            .and_then(|_| secrets::save_api_keys(&self.root, &self.ai_key, &self.key));
        self.result(result, "");
    }
    fn refresh_installed(&mut self) {
        self.installed_icons_requested = false;
        match model::installed(&self.config.instance_dir(&self.root)) {
            Ok(items) => self.installed = items,
            Err(e) => {
                self.installed.clear();
                self.result(Err(e), "");
            }
        }
    }
    fn changed_instance(&mut self) {
        self.save();
        self.refresh_installed();
        self.projects.clear();
        self.searched = false;
        self.mod_browse.invalidate();
        self.remove_candidate = None;
    }
    fn prune_missing_builds(&mut self) -> bool {
        let removed = self.config.prune_missing(&self.root);
        if removed > 0 {
            self.config.ensure_default();
            let _ = std::fs::create_dir_all(self.config.instance_dir(&self.root));
            self.changed_instance();
            self.result(Ok(()), &format!("Убрано сборок без папки: {removed}"));
            true
        } else {
            false
        }
    }
    fn select_instance(&mut self, version: &str, loader: &str, name: &str, force: bool) {
        if self.busy || self.game_cancel.is_some() {
            return;
        }
        match self.config.select_instance(version, loader, name, force) {
            Ok(()) => {
                // Папка создаётся сразу, чтобы сборка не исчезла при следующей проверке.
                let _ = std::fs::create_dir_all(self.config.instance_dir(&self.root));
                self.changed_instance()
            }
            Err(e) => self.result(Err(e), ""),
        }
    }
    fn open_folder(&mut self) {
        let path = self.config.instance_dir(&self.root);
        let result = std::fs::create_dir_all(&path)
            .map_err(anyhow::Error::from)
            .and_then(|_| open::that(path).map_err(Into::into));
        self.result(result, "");
    }
    fn load_versions(&mut self, ctx: &egui::Context) {
        if self.versions_loading {
            return;
        }
        self.versions_loading = true;
        let (tx, root, ctx) = (self.tx.clone(), self.root.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Event::Versions(versions::load(&root)));
            ctx.request_repaint();
        });
    }
    fn search(&mut self, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        let Some((revision, offset)) = self.mod_browse.begin() else {
            return;
        };
        self.error = false;
        self.status.clear();
        let (key, source, query, profile, tx, ctx) = (
            self.key.clone(),
            self.source,
            self.query.clone(),
            self.config.profiles[self.config.selected].clone(),
            self.tx.clone(),
            ctx.clone(),
        );
        let options = self.search_options.clone();
        let resolved = self.mod_browse.resolved_query.clone();
        std::thread::spawn(move || {
            let result = api::Api::new(key).and_then(|a| {
                browse::search_variants(&query, resolved, |q| {
                    a.search_filtered(source, q, &profile, offset, &options)
                })
            });
            let _ = tx.send(Event::Search(revision, offset, result));
            ctx.request_repaint();
        });
    }
    fn load_categories(&mut self, ctx: &egui::Context) {
        let source = self.source;
        if self.categories.contains_key(&source) || !self.categories_loading.insert(source) {
            return;
        }
        let (key, tx, ctx) = (self.key.clone(), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = api::Api::new(key).and_then(|a| a.categories(source));
            let _ = tx.send(Event::Categories(source, result));
            ctx.request_repaint();
        });
    }
    fn install(&mut self, project: Project, ctx: &egui::Context) {
        if self.busy || self.game_cancel.is_some() {
            return;
        }
        self.busy = true;
        self.error = false;
        self.updates.clear();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.mod_cancel = Some(cancel.clone());
        let (key, profile, instance, tx, ctx) = (
            self.key.clone(),
            self.config.profiles[self.config.selected].clone(),
            self.config.instance_dir(&self.root),
            self.tx.clone(),
            ctx.clone(),
        );
        std::thread::spawn(move || {
            let result = api::Api::new(key).and_then(|a| {
                a.install(&project, &profile, &instance, &cancel, |s| {
                    let _ = tx.send(Event::Progress(s));
                    ctx.request_repaint();
                })
            });
            let _ = tx.send(Event::Installed(result));
            ctx.request_repaint();
        });
    }
    fn mod_key(source: Source, id: &str) -> String {
        format!("{}:{id}", source)
    }
    fn check_updates(&mut self, ctx: &egui::Context) {
        if self.busy || self.game_cancel.is_some() || self.installed.is_empty() {
            return;
        }
        self.busy = true;
        self.error = false;
        self.status = "Проверка обновлений…".into();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.mod_cancel = Some(cancel.clone());
        let (key, profile, items, tx, ctx) = (
            self.key.clone(),
            self.config.profiles[self.config.selected].clone(),
            self.installed.clone(),
            self.tx.clone(),
            ctx.clone(),
        );
        std::thread::spawn(move || {
            let result = api::Api::new(key).and_then(|api| {
                let mut found = vec![];
                let mut last_error = None;
                for item in items {
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    match api.latest_version(item.source, &item.project_id, &profile) {
                        Ok(version) => {
                            let newer = version
                                .as_ref()
                                .is_some_and(|value| *value != item.version_id);
                            found.push((
                                item.source,
                                item.project_id.clone(),
                                if newer { version } else { None },
                            ));
                        }
                        Err(error) => last_error = Some(error),
                    }
                }
                if found.is_empty() {
                    if let Some(error) = last_error {
                        return Err(error);
                    }
                }
                Ok(found)
            });
            let _ = tx.send(Event::Updates(result));
            ctx.request_repaint();
        });
    }
    fn update_mod(&mut self, item: InstalledMod, ctx: &egui::Context) {
        if self.busy || self.game_cancel.is_some() {
            return;
        }
        self.busy = true;
        self.error = false;
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.mod_cancel = Some(cancel.clone());
        let (key, profile, instance, tx, ctx) = (
            self.key.clone(),
            self.config.profiles[self.config.selected].clone(),
            self.config.instance_dir(&self.root),
            self.tx.clone(),
            ctx.clone(),
        );
        std::thread::spawn(move || {
            let result = api::Api::new(key).and_then(|api| {
                api.update(&item, &profile, &instance, &cancel, |s| {
                    let _ = tx.send(Event::Progress(s));
                    ctx.request_repaint();
                })
            });
            let _ = tx.send(Event::Updated(result));
            ctx.request_repaint();
        });
    }
    fn store_account(&mut self, account: auth::Account, select: bool) {
        if let Some(i) = self
            .config
            .accounts
            .iter()
            .position(|a| a.uuid == account.uuid && a.kind == account.kind)
        {
            self.config.accounts[i] = account;
            if select {
                self.config.selected_account = i;
            }
        } else {
            self.config.accounts.push(account);
            if select {
                self.config.selected_account = self.config.accounts.len() - 1;
            }
        }
        self.save();
        if let Err(error) = secrets::save(&self.root, &self.config.accounts) {
            self.auth_status = format!("Сессия не сохранена: {error:#}");
        }
    }
    fn add_account(&mut self, account: auth::Account) {
        self.store_account(account, true);
        self.account_kind = None;
        self.auth_status = "Аккаунт добавлен".into();
        self.device = None;
    }
    fn start_microsoft(&mut self, ctx: &egui::Context) {
        if self.auth_busy {
            return;
        }
        if uuid::Uuid::parse_str(self.config.microsoft_client_id.trim()).is_err() {
            self.auth_status = "Для входа укажите Client ID приложения в настройках.".into();
            return;
        }
        self.save();
        self.auth_busy = true;
        self.device = None;
        self.auth_status = "Получение кода…".into();
        let (cancel, rx) = mpsc::channel();
        self.cancel_auth = Some(cancel);
        let (id, tx, ctx) = (
            self.config.microsoft_client_id.clone(),
            self.tx.clone(),
            ctx.clone(),
        );
        std::thread::spawn(move || {
            let result = microsoft::authenticate(
                &id,
                &rx,
                |prompt| {
                    let _ = tx.send(Event::Device(prompt));
                    ctx.request_repaint();
                },
                |s| {
                    let _ = tx.send(Event::AuthProgress(s.into()));
                    ctx.request_repaint();
                },
            );
            let _ = tx.send(Event::Auth(result));
            ctx.request_repaint();
        });
    }
    fn process_events(&mut self, ctx: &egui::Context) {
        let revision = overlay::REVISION.load(std::sync::atomic::Ordering::Relaxed);
        if revision != self.overlay_revision {
            self.overlay_revision = revision;
            self.resource_catalog.invalidate_resources();
        }
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Analysis(result) => {
                    self.ai_busy = false;
                    self.ai_answer = match result {
                        Ok(answer) => answer,
                        Err(error) => format!("{error:#}"),
                    };
                }
                Event::Wallpaper(result) => {
                    self.wallpaper_busy = false;
                    match result {
                        Ok(path) => {
                            self.config.appearance.background = path;
                            self.background_path_input = self.config.appearance.background.clone();
                            self.save();
                        }
                        Err(error) => self.result(Err(error), ""),
                    }
                }
                Event::JavaFound(installations) => {
                    self.java_searching = false;
                    self.java_installations = installations;
                }
                Event::Game(id, event) => match event {
                    game::Event::Progress(s) => self.launch_status = s,
                    game::Event::Log(line) => {
                        if self.game_logs.len() >= if variant::LITE { 500 } else { 1500 } {
                            self.game_logs.pop_front();
                        }
                        self.game_logs.push_back(format!("[Сессия {id}] {line}"));
                    }
                    game::Event::Started => {
                        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                            session.running = true;
                        }
                        self.game_running = true;
                        self.launch_status = "Minecraft запущен".into();
                        match self.on_game_start() {
                            model::OnGameStart::KeepOpen => {}
                            model::OnGameStart::Minimize => {
                                #[cfg(windows)]
                                let hidden = self.tray.hide();
                                #[cfg(not(windows))]
                                let hidden = false;
                                if !hidden {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                                }
                            }
                            model::OnGameStart::Close => {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close)
                            }
                        }
                    }
                    game::Event::Finished(result) => {
                        let cancelled =
                            self.sessions.iter().find(|s| s.id == id).is_some_and(|s| {
                                s.cancel.load(std::sync::atomic::Ordering::Relaxed)
                            });
                        self.sessions.retain(|s| s.id != id);
                        self.game_cancel = self.sessions.first().map(|s| s.cancel.clone());
                        self.game_running = self.sessions.iter().any(|s| s.running);
                        #[cfg(windows)]
                        self.tray.remove();
                        if self.on_game_start() == model::OnGameStart::Minimize
                            && self.sessions.is_empty()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        }
                        self.launch_status.clear();
                        match result {
                            Ok(0) => self.result(Ok(()), "Игра закрыта"),
                            _ if cancelled => self.result(Ok(()), "Остановлено"),
                            Ok(code) => {
                                self.result(Err(anyhow::anyhow!("Minecraft завершился с кодом {code}. Откройте журнал запуска.")),"");
                                self.navigate(Page::Logs);
                            }
                            Err(error) => {
                                self.game_logs.push_back(error.clone());
                                self.result(Err(anyhow::anyhow!(error)), "");
                                self.navigate(Page::Logs);
                            }
                        }
                    }
                },
                Event::Categories(source, result) => {
                    self.categories_loading.remove(&source);
                    match result {
                        Ok(categories) => {
                            self.categories.insert(source, categories);
                            self.category_error.clear();
                        }
                        Err(e) => {
                            self.categories.insert(source, vec![]);
                            self.category_error = format!("{e:#}");
                        }
                    }
                }
                Event::Versions(result) => {
                    self.versions_loading = false;
                    match result {
                        Ok((list, status)) => {
                            self.version_list = list;
                            self.versions_status = status;
                        }
                        Err(e) => self.versions_status = format!("{e:#}"),
                    }
                }
                Event::Search(revision, offset, result) => {
                    if !self.mod_browse.finish(
                        revision,
                        offset,
                        result.as_ref().ok().map(|(_, projects)| projects.len()),
                    ) {
                        continue;
                    }
                    self.searched = true;
                    match result {
                        Ok((query, projects)) => {
                            self.mod_browse.resolved_query = Some(query);
                            self.thumbnails.request(
                                projects.iter().map(|p| p.icon_url.clone()).collect(),
                                ctx,
                            );
                            if offset == 0 {
                                self.projects.clear();
                            }
                            for project in projects {
                                if !self
                                    .projects
                                    .iter()
                                    .any(|p| p.source == project.source && p.id == project.id)
                                {
                                    self.projects.push(project);
                                }
                            }
                            self.status.clear();
                            self.error = false;
                        }
                        Err(e) => self.result(Err(e), ""),
                    }
                }
                Event::Progress(s) => self.status = s,
                Event::Installed(result) => {
                    self.busy = false;
                    self.mod_cancel = None;
                    match result {
                        Ok(n) => self.result(Ok(()), &format!("Установлено: {n}")),
                        Err(e) => self.result(Err(e), ""),
                    };
                    self.refresh_installed();
                }
                Event::Updated(result) => {
                    self.busy = false;
                    self.mod_cancel = None;
                    match result {
                        Ok(true) => self.result(Ok(()), "Мод обновлён"),
                        Ok(false) => self.result(Ok(()), "Обновлений нет"),
                        Err(e) => self.result(Err(e), ""),
                    }
                    self.refresh_installed();
                    self.updates.clear();
                }
                Event::Updates(result) => {
                    self.busy = false;
                    self.mod_cancel = None;
                    match result {
                        Ok(list) => {
                            self.updates.clear();
                            let mut count = 0;
                            for (source, id, version) in list {
                                if version.is_some() {
                                    count += 1;
                                }
                                self.updates.insert(Self::mod_key(source, &id), version);
                            }
                            self.result(Ok(()), &format!("Доступно обновлений: {count}"));
                        }
                        Err(e) => self.result(Err(e), ""),
                    }
                }
                Event::Auth(result) => {
                    self.auth_busy = false;
                    self.cancel_auth = None;
                    self.device = None;
                    match result {
                        Ok(account) => self.add_account(account),
                        Err(e) => self.auth_status = format!("{e:#}"),
                    }
                }
                Event::AuthProgress(s) => self.auth_status = s,
                Event::SessionRefreshed(kind, uuid, result) => match result {
                    Ok(account) => self.store_account(account, false),
                    Err(_) => {
                        let name = self
                            .config
                            .accounts
                            .iter_mut()
                            .find(|a| a.uuid == uuid && a.kind == kind)
                            .map(|account| {
                                account.session = None;
                                account.name.clone()
                            });
                        if let Some(name) = name {
                            self.auth_status = format!("Сессия {name} истекла. Войдите повторно.");
                        }
                        let _ = secrets::remove(&self.root, &secrets::key(kind, &uuid));
                        self.save();
                    }
                },
                Event::Device(prompt) => {
                    self.auth_status = "Введите код на сайте Microsoft".into();
                    let _ = open::that(&prompt.url);
                    self.device = Some(prompt);
                }
                Event::Logout(result) => {
                    self.auth_busy = false;
                    self.auth_status = if result.is_ok() {
                        "Аккаунт удалён".into()
                    } else {
                        "Аккаунт удалён локально. Отзовите сессию в Ely.by.".into()
                    };
                }
            }
        }
    }

    fn launch_panel(&mut self, ui: &mut egui::Ui) {
        let account = self
            .config
            .accounts
            .get(self.config.selected_account)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "Добавить аккаунт".into());
        ui.horizontal(|ui| {
            let response = if let Some(texture) = self
                .config
                .accounts
                .get(self.config.selected_account)
                .and_then(|a| self.avatars.get(a))
            {
                ui.add(egui::ImageButton::new(
                    egui::Image::new(texture).fit_to_exact_size(Vec2::splat(24.)),
                ))
                .on_hover_text("Аккаунты")
            } else {
                icon_button(ui, Icon::User, "Аккаунты")
            };
            if response.clicked() {
                self.toggle_page(Page::Accounts);
            }
            egui::ComboBox::from_id_salt("account")
                .width(ui.available_width())
                .truncate()
                .selected_text(account)
                .show_ui(ui, |ui| {
                    let old = self.config.selected_account;
                    for (i, a) in self.config.accounts.iter().enumerate() {
                        ui.selectable_value(&mut self.config.selected_account, i, &a.name);
                    }
                    if old != self.config.selected_account {
                        self.save();
                    }
                    if ui.button("+ Добавить аккаунт").clicked() {
                        self.navigate(Page::Accounts);
                    }
                });
        });
        let old = self.config.selected;
        ui.add_enabled_ui(
            !self.busy
                && (self.game_cancel.is_none() || self.config.multi_session.enabled)
                && !self.content_mutation_busy(),
            |ui| {
                egui::ComboBox::from_id_salt("instance")
                    .width(ui.available_width())
                    .truncate()
                    .selected_text(&self.config.profiles[self.config.selected].name)
                    .show_ui(ui, |ui| {
                        for (i, p) in self.config.profiles.iter().enumerate() {
                            ui.selectable_value(&mut self.config.selected, i, &p.name);
                        }
                        ui.separator();
                        if ui.button("Все версии…").clicked() {
                            self.navigate(Page::Versions);
                        }
                    });
            },
        );
        if old != self.config.selected {
            self.changed_instance();
        }
        ui.separator();
        ui.add_space((ui.available_height() - 88.).max(0.));
        if ui
            .add_enabled_ui(!self.busy, |ui| {
                ui.add_sized(
                    [ui.available_width(), 48.],
                    egui::Button::new(
                        RichText::new(
                            if self.game_cancel.is_some() && !self.config.multi_session.enabled {
                                if self.game_running {
                                    "Остановить"
                                } else {
                                    "Отменить"
                                }
                            } else {
                                "Запустить"
                            },
                        )
                        .size(19.),
                    ),
                )
            })
            .inner
            .clicked()
        {
            if let Some(cancel) = self
                .game_cancel
                .as_ref()
                .filter(|_| !self.config.multi_session.enabled)
            {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            } else {
                self.start_game(ui.ctx());
            }
        }
        ui.columns(5, |columns| {
            let click = |ui: &mut egui::Ui, icon, label| {
                let width = ui.available_width();
                ui::icon_button_size(ui, icon, label, Vec2::new(width, layout::ROW)).clicked()
            };
            if click(&mut columns[0], Icon::MyBuilds, "Мои сборки") {
                self.toggle_library(VersionTab::Builds);
            }
            if click(&mut columns[1], Icon::Folder, "Папка игры") {
                self.open_folder();
            }
            if click(&mut columns[2], Icon::Versions, "Каталог версий") {
                self.toggle_library(VersionTab::Catalog);
            }
            if click(&mut columns[3], Icon::Mods, "Моды и ресурспаки") {
                self.toggle_page(Page::Mods);
            }
            if click(&mut columns[4], Icon::Settings, "Настройки") {
                self.toggle_page(Page::Settings);
            }
        });
    }

    fn accounts(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut remove = None;
        let previous_account = self.config.selected_account;
        for (i, a) in self.config.accounts.iter().enumerate() {
            ui.horizontal(|ui| {
                let response = if let Some(texture) = self.avatars.get(a) {
                    ui.add(egui::ImageButton::new(
                        egui::Image::new(texture).fit_to_exact_size(Vec2::splat(32.)),
                    ))
                    .on_hover_text(&a.name)
                } else {
                    icon_button(ui, Icon::User, &a.name)
                };
                if response.clicked() {
                    self.config.selected_account = i;
                }
                ui.label(&a.name).on_hover_text(
                    if a.kind != auth::AccountKind::Offline && a.session.is_none() {
                        "Требуется повторный вход"
                    } else {
                        "Аккаунт выбран"
                    },
                );
                if ui
                    .add_enabled_ui(!self.auth_busy, |ui| {
                        icon_button(ui, Icon::Trash, "Удалить аккаунт")
                    })
                    .inner
                    .clicked()
                {
                    remove = Some(i);
                }
            });
        }
        if previous_account != self.config.selected_account {
            self.save();
        }
        if let Some(i) = remove {
            let mut account = self.config.accounts.remove(i);
            self.config.selected_account = self
                .config
                .selected_account
                .min(self.config.accounts.len().saturating_sub(1));
            self.save();
            let _ = secrets::remove(&self.root, &secrets::key(account.kind, &account.uuid));
            if account.kind == auth::AccountKind::ElyBy {
                if let Some(session) = account.session.take() {
                    self.auth_busy = true;
                    let (tx, ctx) = (self.tx.clone(), ctx.clone());
                    std::thread::spawn(move || {
                        let _ = tx.send(Event::Logout(auth::invalidate(session)));
                        ctx.request_repaint();
                    });
                }
            }
        }
        ui.separator();
        if self.account_kind.is_none() {
            if ui::provider_button(ui, Icon::Ely, "Ely.by (система скинов)").clicked()
            {
                self.account_kind = Some(auth::AccountKind::ElyBy);
                self.auth_status.clear();
            }
            if ui::provider_button(ui, Icon::Microsoft, "Microsoft").clicked() {
                self.account_kind = Some(auth::AccountKind::Microsoft);
                self.auth_status.clear();
            }
            if ui::provider_button(ui, Icon::User, "Обычный (без пароля)").clicked()
            {
                self.account_kind = Some(auth::AccountKind::Offline);
                self.auth_status.clear();
            }
        } else {
            ui.horizontal(|ui| {
                if icon_button(ui, Icon::Back, "Назад").clicked() && !self.auth_busy {
                    self.account_kind = None;
                }
                ui.label(match self.account_kind {
                    Some(auth::AccountKind::Microsoft) => "Microsoft",
                    Some(auth::AccountKind::ElyBy) => "Ely.by",
                    _ => "Обычный аккаунт",
                });
            });
            match self.account_kind {
                Some(auth::AccountKind::Offline) => {
                    ui.add_sized(
                        [ui.available_width(), layout::ROW],
                        egui::TextEdit::singleline(&mut self.account_name).hint_text("Ваш ник"),
                    );
                    ui.checkbox(&mut self.offline_skins, "Использовать скины Ely.by")
                        .on_hover_text("Скины внутри игры ещё не подключены");
                    if ui
                        .add_enabled(!self.auth_busy, egui::Button::new("Добавить"))
                        .clicked()
                    {
                        match auth::Account::offline(self.account_name.trim(), self.offline_skins) {
                            Ok(a) => self.add_account(a),
                            Err(e) => self.auth_status = e.to_string(),
                        }
                    }
                }
                Some(auth::AccountKind::ElyBy) => {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.account_name)
                            .min_size(Vec2::new(0., layout::ROW))
                            .hint_text("Логин или email"),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.account_password)
                            .min_size(Vec2::new(0., layout::ROW))
                            .hint_text("Пароль")
                            .password(true),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.account_totp)
                            .min_size(Vec2::new(0., layout::ROW))
                            .hint_text("2FA, если включена")
                            .password(true),
                    );
                    if ui
                        .add_enabled(!self.auth_busy, egui::Button::new("Войти"))
                        .clicked()
                    {
                        let (login, password, totp) = (
                            self.account_name.clone(),
                            zeroize::Zeroizing::new(std::mem::take(&mut self.account_password)),
                            zeroize::Zeroizing::new(std::mem::take(&mut self.account_totp)),
                        );
                        self.auth_busy = true;
                        self.auth_status = "Вход…".into();
                        let (tx, ctx) = (self.tx.clone(), ctx.clone());
                        std::thread::spawn(move || {
                            let _ = tx.send(Event::Auth(auth::authenticate(login, password, totp)));
                            ctx.request_repaint();
                        });
                    }
                }
                Some(auth::AccountKind::Microsoft) => {
                    if let Some(prompt) = &self.device {
                        ui.label("Введите код в браузере:");
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&prompt.code).size(23.).strong());
                            if icon_button(ui, Icon::Copy, "Скопировать код").clicked()
                            {
                                ctx.copy_text(prompt.code.clone());
                            }
                        });
                        ui.hyperlink_to("Открыть Microsoft", &prompt.url)
                            .on_hover_text(format!(
                                "Код действует {} минут",
                                prompt.expires_in / 60
                            ));
                    } else if !self.auth_busy {
                        ui.label("Вход на сайте Microsoft, без пароля в лаунчере.");
                        if ui.button("Войти через браузер").clicked() {
                            self.start_microsoft(ctx);
                        }
                        if self.config.microsoft_client_id.is_empty()
                            && ui.link("Настроить Client ID").clicked()
                        {
                            self.navigate(Page::Settings);
                        }
                    }
                    if self.auth_busy && ui.button("Отмена").clicked() {
                        if let Some(cancel) = &self.cancel_auth {
                            let _ = cancel.send(());
                        }
                        self.auth_status = "Отмена…".into();
                    }
                }
                None => {}
            }
        }
        if self.auth_busy {
            ui.spinner();
        }
        if !self.auth_status.is_empty() {
            ui.label(&self.auth_status);
        }
    }

    fn on_game_start(&self) -> model::OnGameStart {
        if self.config.multi_session.enabled || self.sessions.len() > 1 {
            model::OnGameStart::KeepOpen
        } else {
            self.config.on_game_start
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            if ui::icon_tab(ui, Icon::Info, "О проекте", false).clicked() {
                self.toggle_page(Page::About);
            }
        });
        ui.collapsing("Расположение вкладок", |ui| {
            use model::LibraryGroup;
            let previous = self.config.library_tabs;
            let tabs = &mut self.config.library_tabs;
            for (label, group) in [
                ("Скачать сборку", &mut tabs.packs),
                ("Импорт / экспорт", &mut tabs.exchange),
            ] {
                egui::ComboBox::from_label(label)
                    .selected_text(match group {
                        LibraryGroup::Catalog => "Каталог версий",
                        LibraryGroup::MyBuilds => "Мои сборки",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(group, LibraryGroup::Catalog, "Каталог версий");
                        ui.selectable_value(group, LibraryGroup::MyBuilds, "Мои сборки");
                    });
            }
            if previous != *tabs {
                let result = self.config.save(&self.root);
                self.result(result, "");
            }
        });
        if !variant::LITE {
            let old = serde_json::to_string(&self.config.multi_session).unwrap_or_default();
            let old_manual = self.config.curseforge_manual;
            ui.collapsing("Мульти-Сессии",|ui|{
            ui.checkbox(&mut self.config.multi_session.enabled,"Разрешить несколько Minecraft-сессий");
            ui.add(egui::Slider::new(&mut self.config.multi_session.limit,1..=8).text("Лимит сессий"));
            ui.add(egui::Slider::new(&mut self.config.multi_session.memory_mb,1024..=16384).text("МиБ на сессию"));
            ui.label("Профиль экономии: минимум 256 МиБ, окно 854×480, без полноэкранного режима. Для тяжёлых сборок увеличьте память. Каждая сессия требует отдельную сборку; миры и настройки не разделяются.");
            ui.label(format!("Лимит Java heap: {} МиБ суммарно (+ память JVM и системы).",self.config.multi_session.memory_mb as usize*self.config.multi_session.limit));
            for session in &self.sessions {
                ui.horizontal(|ui|{ui.label(format!("#{} · {} · {}",session.id,session.name,if session.running{"Работает"}else{"Подготовка"}));
                    if ui.button("Остановить").clicked(){session.cancel.store(true,std::sync::atomic::Ordering::Relaxed);}
                });
            }
        });
            ui.checkbox(&mut self.config.multi_session.native_overlay,"Внутриигровой оверлей · Shift+Tab (экспериментально)").on_hover_text("Для следующих запусков: внедрение DLL в запускаемую Java, без Minecraft-мода. Только Windows x64 / OpenGL. Vulkan не поддерживается. Возможны конфликты с античитами и другими оверлеями. При сбое отключите настройку и перезапустите игру.");
            let dll = std::env::current_exe()
                .unwrap_or_default()
                .with_file_name("mlauncher-overlay.dll");
            ui.label(
                RichText::new(if dll.is_file() {
                    "DLL оверлея найдена рядом с EXE; хеш проверяется при запуске."
                } else {
                    "DLL оверлея не найдена рядом с EXE — запуск игры пройдёт без оверлея."
                })
                .small(),
            );
            ui.checkbox(&mut self.config.curseforge_manual,"CurseForge: ручная загрузка (экспериментально)").on_hover_text("Официальный сайт в вашем браузере; затем выберите скачанный JAR. Не обходит запреты скачивания и не использует чужие API-ключи.");
            if old != serde_json::to_string(&self.config.multi_session).unwrap_or_default()
                || old_manual != self.config.curseforge_manual
            {
                let result = self.config.save(&self.root);
                self.result(result, "");
            }
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.show_appearance, true, "Оформление");
            ui.selectable_value(&mut self.show_appearance, false, "Аккаунты и сборка");
        });
        ui.separator();
        if self.show_appearance {
            self.appearance_settings(ui, ctx);
            return;
        }
        self.java_settings(ui, ctx);
        ui.separator();
        if !variant::LITE {
            self.ai_settings(ui);
        }
        ui.separator();
        let field_width = (ui.available_width() - 144.).max(120.);
        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing(Vec2::new(16., 8.))
            .show(ui, |ui| {
                ui.add_sized(
                    [128., layout::ROW],
                    egui::Label::new("Название сборки").wrap(),
                );
                ui.add_sized(
                    [field_width, layout::ROW],
                    egui::TextEdit::singleline(
                        &mut self.config.profiles[self.config.selected].name,
                    ),
                );
                ui.end_row();
                ui.label("");
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "Папка: {}",
                            self.config.profiles[self.config.selected].directory
                        ))
                        .small(),
                    )
                    .truncate(),
                )
                .on_hover_text(self.config.instance_dir(&self.root).display().to_string());
                ui.end_row();
                ui.add_sized(
                    [128., layout::ROW],
                    egui::Label::new("Microsoft Client ID").wrap(),
                );
                ui.add_sized(
                    [field_width, layout::ROW],
                    egui::TextEdit::singleline(&mut self.config.microsoft_client_id)
                        .hint_text("ID приложения"),
                );
                ui.end_row();
                ui.add_sized(
                    [128., layout::ROW],
                    egui::Label::new("CurseForge API key").wrap(),
                );
                ui.add_sized(
                    [field_width, layout::ROW],
                    egui::TextEdit::singleline(&mut self.key).password(true),
                );
                ui.end_row();
            });
        ui.hyperlink_to(
            "Регистрация приложения",
            "https://learn.microsoft.com/entra/identity-platform/quickstart-register-app",
        )
        .on_hover_text(
            "Personal Microsoft accounts, public client/device flow; нужен доступ к Minecraft API",
        );
        ui.horizontal(|ui| {
            if ui.button("Сохранить").clicked() {
                if self.game_cancel.is_none()
                    && self.config.sync_directory(self.config.selected, &self.root)
                {
                    self.refresh_installed();
                }
                self.save();
                self.navigate(Page::Home);
            }
            if icon_button(ui, Icon::Folder, "Папка сборки").clicked() {
                self.open_folder();
            }
        });
        if ui.button("Сбросить размеры панелей").clicked() {
            for id in layout::PanelId::ALL {
                self.config.panels.set(id, id.default_size());
            }
            self.layout_dirty_at = Some(ctx.input(|i| i.time));
            self.persist_layout();
            ctx.request_repaint();
        }
    }
    fn java_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        use model::OnGameStart;
        let modes = [
            (OnGameStart::KeepOpen, "Оставаться открытым"),
            (OnGameStart::Minimize, "Свернуться в трей"),
            (OnGameStart::Close, "Закрыться"),
        ];
        let mode = self.on_game_start();
        ui.label("Лаунчер после запуска игры");
        ui.add_enabled_ui(!self.config.multi_session.enabled, |ui| {
            let response = egui::ComboBox::from_id_salt("on_game_start")
                .selected_text(modes.iter().find(|(value, _)| *value == mode).unwrap().1)
                .show_ui(ui, |ui| {
                    for (value, label) in modes {
                        ui.selectable_value(&mut self.config.on_game_start, value, label);
                    }
                });
            response
                .response
                .on_hover_text("При закрытии лаунчера оверлей отключается.");
        });
        ui.strong("Minecraft и Java");
        ui.add_enabled_ui(self.game_cancel.is_none(),|ui|{
            ui.label("Папка установки Minecraft");
            ui.horizontal(|ui|{
                let width=(ui.available_width()-48.).max(100.);ui.add_sized([width,layout::ROW],egui::TextEdit::singleline(&mut self.config.game_directory).hint_text(self.root.to_string_lossy()));
                if icon_button(ui,Icon::Folder,"Выбрать папку установки").clicked(){if let Some(path)=rfd::FileDialog::new().pick_folder(){self.config.game_directory=path.to_string_lossy().into_owned();self.refresh_installed();}}
            });
            ui.label(RichText::new("Старые файлы не перемещаются и не удаляются. Верните прежний путь, чтобы открыть их снова.").small());
            ui.label("Java (пусто — автоматический подбор)");
            ui.horizontal(|ui|{let width=(ui.available_width()-48.).max(100.);ui.add_sized([width,layout::ROW],egui::TextEdit::singleline(&mut self.config.java.executable).hint_text("Авто"));if icon_button(ui,Icon::Folder,"Выбрать java.exe").clicked(){if let Some(path)=rfd::FileDialog::new().add_filter("Java",&["exe"]).pick_file(){self.config.java.executable=path.to_string_lossy().into_owned();}}});
            ui.horizontal(|ui|{if ui.add_enabled(!self.java_searching,egui::Button::new("Найти установленные Java")).clicked(){
                    self.find_java(ctx);
                }
                if ui.button("Авто").clicked(){
                    self.config.java.executable.clear();
                }
                if self.java_searching{
                    ui.spinner();
                }});
            for installation in &self.java_installations{if ui.selectable_label(self.config.java.executable==installation.path.to_string_lossy(),format!("Java {} · {}",installation.major,installation.path.display())).on_hover_text(&installation.description).clicked(){self.config.java.executable=installation.path.to_string_lossy().into_owned();}}
            ui.checkbox(&mut self.config.java.auto_download,"Автоматически скачать нужную Java, если она не найдена");
            egui::Grid::new("java_settings_grid").num_columns(2).spacing(Vec2::new(16.,8.)).show(ui,|ui|{
                ui.label("Память минимум (МиБ)");ui.add(egui::DragValue::new(&mut self.config.java.min_memory).range(128..=65536).speed(128));ui.end_row();
                ui.label("Память максимум (МиБ)");ui.add(egui::DragValue::new(&mut self.config.java.max_memory).range(512..=65536).speed(128));ui.end_row();
                ui.label("Размер окна игры");ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut self.config.java.window_width).range(320..=7680));ui.label("×");ui.add(egui::DragValue::new(&mut self.config.java.window_height).range(240..=4320));});ui.end_row();
            });
            ui.checkbox(&mut self.config.java.fullscreen,"Полноэкранный режим");
            egui::ComboBox::from_id_salt("collector").selected_text(match self.config.java.collector{java::Collector::Default=>"GC: по умолчанию",java::Collector::G1=>"GC: G1",java::Collector::Zgc=>"GC: ZGC (17+)",java::Collector::Shenandoah=>"GC: Shenandoah (17+, зависит от Java)"}).show_ui(ui,|ui|{
                ui.selectable_value(&mut self.config.java.collector,java::Collector::Default,"По умолчанию");ui.selectable_value(&mut self.config.java.collector,java::Collector::G1,"G1GC");ui.selectable_value(&mut self.config.java.collector,java::Collector::Zgc,"ZGC (Java 17+)");ui.selectable_value(&mut self.config.java.collector,java::Collector::Shenandoah,"Shenandoah (Java 17+)");
            });
            ui.collapsing("Дополнительные JVM-аргументы",|ui|{ui.label("Один аргумент на строку. Память задаётся выше; PermGen в Java 8+ отсутствует.");ui.add(egui::TextEdit::multiline(&mut self.config.java.extra_args).desired_width(f32::INFINITY).desired_rows(4));});
        });
    }

    fn appearance_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if variant::LITE {
            self.lite_appearance(ui, ctx);
            return;
        }
        ui.label("Стиль");
        ui.columns(4, |columns| {
            for (theme, column) in appearance::Theme::ALL.into_iter().zip(columns) {
                if column
                    .add_sized(
                        [column.available_width(), layout::ROW],
                        egui::Button::new(theme.label())
                            .selected(self.config.appearance.theme == theme),
                    )
                    .clicked()
                {
                    self.config.appearance.theme = theme;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Акцент");
            ui.color_edit_button_srgb(&mut self.config.appearance.accent);
            for color in [
                [120, 186, 170],
                [87, 155, 244],
                [181, 132, 240],
                [244, 154, 80],
                [234, 112, 141],
            ] {
                let response = ui.add(
                    egui::Button::new("  ")
                        .fill(Color32::from_rgb(color[0], color[1], color[2]))
                        .min_size(Vec2::splat(22.)),
                );
                if response.clicked() {
                    self.config.appearance.accent = color;
                }
            }
        });
        ui.label("Встроенный фон");
        egui::ComboBox::from_id_salt("background_effect")
            .selected_text(self.config.appearance.effect.label())
            .show_ui(ui, |ui| {
                for effect in effects::Effect::ALL {
                    ui.selectable_value(&mut self.config.appearance.effect, effect, effect.label());
                }
            });
        ui.checkbox(
            &mut self.config.appearance.light_background,
            "Светлый фон эффектов",
        );
        ui.checkbox(
            &mut self.config.appearance.transitions,
            "Плавные переходы панелей",
        );
        let field_width = (ui.available_width() - 144.).max(120.);
        egui::Grid::new("appearance_grid")
            .num_columns(2)
            .spacing(Vec2::new(16., 8.))
            .show(ui, |ui| {
                ui.add_sized(
                    [128., layout::ROW],
                    egui::Label::new("Плотность панелей").wrap(),
                );
                ui.scope(|ui| {
                    ui.spacing_mut().slider_width = (field_width - 64.).max(64.);
                    ui.add_sized(
                        [field_width, layout::ROW],
                        egui::Slider::new(&mut self.config.appearance.panel_opacity, 140..=255),
                    );
                });
                ui.end_row();
                ui.add_sized(
                    [128., layout::ROW],
                    egui::Label::new("Затемнение фона").wrap(),
                );
                ui.scope(|ui| {
                    ui.spacing_mut().slider_width = (field_width - 64.).max(64.);
                    ui.add_sized(
                        [field_width, layout::ROW],
                        egui::Slider::new(&mut self.config.appearance.dim, 0..=220),
                    );
                });
                ui.end_row();
            });
        ui.checkbox(&mut self.config.appearance.animate, "Анимация фона")
            .on_hover_text("Выключите для неподвижного кадра и меньшей нагрузки");
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Выбрать фон…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter(
                        "Фон",
                        &[
                            "png", "jpg", "jpeg", "webp", "bmp", "gif", "mp4", "webm", "mkv",
                            "mov", "avi", "m4v",
                        ],
                    )
                    .pick_file()
                {
                    self.import_wallpaper(path, ctx);
                }
            }
            if ui.button("Стандартный").clicked() {
                self.config.appearance.background.clear();
                self.background_path_input.clear();
            }
        });
        if !self.config.appearance.background.is_empty() {
            let name = std::path::Path::new(&self.config.appearance.background)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            ui.add(egui::Label::new(name).truncate())
                .on_hover_text(&self.config.appearance.background);
        }
        ui.collapsing("Путь к файлу", |ui| {
            ui.add_sized(
                [ui.available_width(), layout::ROW],
                egui::TextEdit::singleline(&mut self.background_path_input),
            );
            if ui.button("Применить путь").clicked() {
                self.import_wallpaper(PathBuf::from(&self.background_path_input), ctx);
            }
        });
        if !self.background.status.is_empty() {
            ui.label(&self.background.status);
        }
        if self.wallpaper_busy {
            ui.label("Копирование фона в кеш…");
            ui.spinner();
        }
        ui.label(RichText::new("PNG, JPG, WebP, GIF · видео через FFmpeg, без звука").small());
        ui.horizontal_wrapped(|ui| {
            if ui.button("Сохранить").clicked() {
                self.save();
            }
            if ui.button("Сбросить оформление").clicked() {
                self.config.appearance = Default::default();
                self.background_path_input.clear();
                self.save();
            }
            if ui.button("Перезагрузить фон").clicked() {
                self.background.load(
                    &self.config.appearance.background,
                    self.config.appearance.animate,
                    ctx,
                );
            }
        });
    }

    fn lite_appearance(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let before = serde_json::to_string(&self.config.appearance).unwrap_or_default();
        ui.strong("MLauncher Lite · оформление");
        ui.label("Простые эффекты и анимации без видео и тяжёлых сцен.");
        ui.horizontal(|ui| {
            ui.label("Акцент");
            ui.color_edit_button_srgb(&mut self.config.appearance.accent);
            for color in [
                [120, 186, 170],
                [87, 155, 244],
                [181, 132, 240],
                [244, 154, 80],
                [234, 112, 141],
            ] {
                if ui
                    .add(
                        egui::Button::new("  ")
                            .fill(Color32::from_rgb(color[0], color[1], color[2]))
                            .min_size(Vec2::splat(22.)),
                    )
                    .clicked()
                {
                    self.config.appearance.accent = color;
                }
            }
        });
        egui::ComboBox::from_id_salt("lite_effect")
            .selected_text(self.config.appearance.effect.label())
            .show_ui(ui, |ui| {
                for effect in effects::Effect::LITE {
                    ui.selectable_value(&mut self.config.appearance.effect, effect, effect.label());
                }
            });
        ui.checkbox(&mut self.config.appearance.animate, "Анимация фона / GIF")
            .on_hover_text(
                "Волны, пузыри и сияние анимируются; без фона перерисовка не запрашивается",
            );
        ui.checkbox(
            &mut self.config.appearance.transitions,
            "Анимация открытия и закрытия вкладок",
        );
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Выбрать фото / GIF…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Фото и GIF", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
                    .pick_file()
                {
                    self.import_wallpaper(path, ctx);
                }
            }
            if ui.button("Убрать фон").clicked() {
                self.config.appearance.background.clear();
                self.background_path_input.clear();
            }
        });
        if !self.config.appearance.background.is_empty() {
            ui.add(egui::Label::new(&self.config.appearance.background).wrap());
        }
        if self.wallpaper_busy {
            ui.spinner();
        }
        if !self.background.status.is_empty() {
            ui.label(&self.background.status);
        }
        if before != serde_json::to_string(&self.config.appearance).unwrap_or_default() {
            self.save();
        }
    }

    fn open_build_folder(&mut self, index: usize) {
        let Some(profile) = self.config.profiles.get(index) else {
            return;
        };
        let path = self
            .config
            .game_root(&self.root)
            .join("instances")
            .join(&profile.directory);
        let result = std::fs::create_dir_all(&path)
            .map_err(anyhow::Error::from)
            .and_then(|_| open::that(path).map_err(Into::into));
        self.result(result, "");
    }

    fn rename_build(&mut self, index: usize) {
        let name = self.edit_build_name.trim().to_owned();
        if name.is_empty() {
            self.result(
                Err(anyhow::anyhow!("Название сборки не может быть пустым")),
                "",
            );
            return;
        }
        let Some(profile) = self.config.profiles.get_mut(index) else {
            return;
        };
        profile.name = name;
        let synced = self.game_cancel.is_none() && self.config.sync_directory(index, &self.root);
        self.editing_build = None;
        self.changed_instance();
        if synced {
            self.result(Ok(()), "Сборка переименована");
        } else {
            self.result(
                Ok(()),
                "Название сохранено, но папку не удалось переименовать. Закройте её в Проводнике.",
            );
        }
    }

    fn delete_build_now(&mut self, index: usize) {
        if self.game_cancel.is_some() {
            self.result(
                Err(anyhow::anyhow!("Остановите игру перед удалением сборки")),
                "",
            );
            self.delete_build = None;
            return;
        }
        let Some(profile) = self.config.profiles.get(index).cloned() else {
            self.delete_build = None;
            return;
        };
        let remove_folder = self.delete_build_folder;
        if remove_folder {
            let base = self.config.game_root(&self.root).join("instances");
            let folder = base.join(&profile.directory);
            let safe = base.canonicalize().ok().zip(folder.canonicalize().ok());
            match safe {
                Some((base, folder)) if folder.starts_with(&base) && folder != base => {
                    if let Err(error) = std::fs::remove_dir_all(&folder) {
                        self.result(Err(error.into()), "");
                        self.delete_build = None;
                        return;
                    }
                }
                _ => {
                    self.result(
                        Err(anyhow::anyhow!(
                            "Папка сборки не найдена или путь небезопасен"
                        )),
                        "",
                    );
                    self.delete_build = None;
                    return;
                }
            }
        }
        self.config.profiles.remove(index);
        if index < self.config.selected {
            self.config.selected -= 1;
        }
        self.config.selected = self
            .config
            .selected
            .min(self.config.profiles.len().saturating_sub(1));
        self.config.ensure_default();
        let _ = std::fs::create_dir_all(self.config.instance_dir(&self.root));
        self.delete_build = None;
        self.delete_build_folder = false;
        self.editing_build = None;
        self.changed_instance();
        self.result(
            Ok(()),
            if remove_folder {
                "Сборка и её папка удалены"
            } else {
                "Сборка убрана из лаунчера, папка сохранена"
            },
        );
    }

    fn edit_instance(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(mut editor) = self.build_editor.take() else {
            ui.label("Выберите сборку в разделе «Мои сборки».");
            if ui.button("Мои сборки").clicked() {
                self.version_tab = VersionTab::Builds;
                self.navigate(Page::Versions);
            }
            return;
        };
        let path = self
            .config
            .game_root(&self.root)
            .join("instances")
            .join(&editor.directory);
        let blocked = self.sessions.iter().any(|s| s.instance == path)
            || self.busy
            || self.content_mutation_busy();
        match editor.ui(ui, ctx, &path, blocked) {
            Some(instance::Action::Save) => {
                let result = editor.save(&mut self.config, &self.root);
                if result.is_err() {
                    editor.report(result);
                }
                self.refresh_installed();
            }
            Some(instance::Action::Close) => {
                self.version_tab = VersionTab::Builds;
                self.navigate(Page::Versions);
            }
            Some(instance::Action::Changed) => {
                self.refresh_installed();
                self.resource_catalog.invalidate_resources();
            }
            Some(instance::Action::Catalog(resources)) => {
                if let Some(index) = self
                    .config
                    .profiles
                    .iter()
                    .position(|p| p.directory == editor.directory)
                {
                    self.config.selected = index;
                    self.changed_instance();
                    self.resource_tab = resources;
                    self.maps_tab = false;
                    self.installed_tab = !resources;
                    self.navigate(Page::Mods);
                }
            }
            None => {}
        }
        self.build_editor = Some(editor);
    }

    fn my_builds(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let mut select = None;
        let mut start_edit = None;
        let mut open_editor = None;
        let mut open_folder = None;
        let mut request_delete = None;
        let mut confirm_delete = None;
        let mut cancel_delete = false;
        let mut save_edit = None;
        let mut cancel_edit = false;
        ui.horizontal(|ui| {
            if ui.button("+ Создать сборку").clicked() {
                self.version_tab = VersionTab::Catalog;
                self.instance_version = None;
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new("Проверить папки"))
                .on_hover_text("Убрать сборки, чьи папки были удалены")
                .clicked()
            {
                self.prune_missing_builds();
            }
            ui.label(RichText::new(format!("Всего: {}", self.config.profiles.len())).small());
        });
        ui.separator();
        let selected = self.config.selected;
        let count = self.config.profiles.len();
        egui::ScrollArea::vertical()
            .id_salt("my_builds")
            .max_height(ui.available_height().max(64.))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if count == 0 {
                    ui.label("Сборок нет. Создайте новую в каталоге версий.");
                }
                for index in 0..count {
                    let Some(profile) = self.config.profiles.get(index) else {
                        break;
                    };
                    let name = profile.name.clone();
                    let version = profile.game_version.clone();
                    let loader = profile.loader.clone();
                    let directory = profile.directory.clone();
                    ui.push_id(("build", index), |ui| {
                        if self.editing_build == Some(index) {
                            ui.add_sized(
                                [ui.available_width(), layout::ROW],
                                egui::TextEdit::singleline(&mut self.edit_build_name),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("Сохранить").clicked() {
                                    save_edit = Some(index);
                                }
                                if ui.button("Отмена").clicked() {
                                    cancel_edit = true;
                                }
                            });
                        } else {
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(selected == index, &name)
                                    .on_hover_text("Сделать активной")
                                    .clicked()
                                {
                                    select = Some(index);
                                }
                                ui.label(RichText::new(format!("{version} · {loader}")).small());
                            });
                            ui.horizontal(|ui| {
                                if icon_button(ui, Icon::Check, "Сделать активной").clicked()
                                {
                                    select = Some(index);
                                }
                                let edit = icon_button(ui, Icon::Edit, "Редактировать сборку");
                                if edit.clicked() {
                                    open_editor = Some(index);
                                }
                                edit.context_menu(|ui| {
                                    if ui.button("Переименовать").clicked() {
                                        start_edit = Some(index);
                                        ui.close_menu();
                                    }
                                });
                                if icon_button(ui, Icon::Folder, "Открыть папку").clicked()
                                {
                                    open_folder = Some(index);
                                }
                                if ui
                                    .add_enabled_ui(!self.busy, |ui| {
                                        icon_button(ui, Icon::Trash, "Удалить сборку")
                                    })
                                    .inner
                                    .clicked()
                                {
                                    request_delete = Some(index);
                                }
                                ui.add(
                                    egui::Label::new(RichText::new(&directory).small()).truncate(),
                                );
                            });
                        }
                        if self.delete_build == Some(index) {
                            ui.group(|ui| {
                                ui.label(format!("Удалить сборку «{name}»?"));
                                ui.checkbox(
                                    &mut self.delete_build_folder,
                                    "Удалить папку с файлами безвозвратно",
                                );
                                ui.horizontal(|ui| {
                                    if ui.button("Удалить").clicked() {
                                        confirm_delete = Some(index);
                                    }
                                    if ui.button("Отмена").clicked() {
                                        cancel_delete = true;
                                    }
                                });
                            });
                        }
                    });
                    ui.separator();
                }
            });
        if let Some(index) = open_editor {
            self.build_editor = Some(instance::Editor::new(
                &self.config.profiles[index],
                &self.config.java,
            ));
            self.navigate(Page::BuildEdit);
        }
        if let Some(index) = start_edit {
            if let Some(profile) = self.config.profiles.get(index) {
                self.edit_build_name = profile.name.clone();
                self.editing_build = Some(index);
                self.delete_build = None;
            }
        }
        if let Some(index) = select {
            self.config.selected = index;
            self.save();
            self.refresh_installed();
            self.projects.clear();
            self.searched = false;
            self.navigate(Page::Home);
        }
        if let Some(index) = open_folder {
            self.open_build_folder(index);
        }
        if let Some(index) = request_delete {
            self.delete_build = Some(index);
            self.delete_build_folder = false;
            self.editing_build = None;
        }
        if cancel_delete {
            self.delete_build = None;
        }
        if cancel_edit {
            self.editing_build = None;
        }
        if let Some(index) = save_edit {
            self.rename_build(index);
        }
        if let Some(index) = confirm_delete {
            self.delete_build_now(index);
        }
    }

    fn version_picker(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let previous = self.version_tab;
        let tabs = self.config.library_tabs;
        ui.add_enabled_ui(!self.content.busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (tab, icon, label, tooltip) in [
                    (
                        VersionTab::Catalog,
                        Icon::Versions,
                        "Версии",
                        "Каталог Minecraft",
                    ),
                    (
                        VersionTab::Builds,
                        Icon::MyBuilds,
                        "Мои сборки",
                        "Управление установленными сборками",
                    ),
                    (VersionTab::Packs, Icon::Builds, "Скачать сборку", "Каталог сборок"),
                    (
                        VersionTab::Exchange,
                        Icon::Exchange,
                        "Импорт / экспорт",
                        "Импорт / экспорт сборок",
                    ),
                ] {
                    if tab.group(tabs) != previous.group(tabs) {
                        continue;
                    }
                    if ui::icon_tab(ui, icon, label, self.version_tab == tab)
                        .on_hover_text(tooltip)
                        .clicked()
                    {
                        self.version_tab = tab;
                    }
                }
            });
        });
        if previous != self.version_tab {
            self.instance_version = None;
        }
        ui.separator();
        let section = match self.version_tab {
            VersionTab::Packs => Some(content::Section::Packs),
            VersionTab::Exchange => Some(content::Section::Exchange),
            _ => None,
        };
        if let Some(section) = section {
            self.content.set_section(section);
            if let Some(profile) = self.content.ui(
                ui,
                ctx,
                &self.root,
                &self.config,
                &self.key,
                self.busy || self.game_cancel.is_some(),
            ) {
                self.config.profiles.push(profile);
                self.config.selected = self.config.profiles.len() - 1;
                self.changed_instance();
            }
            return;
        }
        if let Some(version) = self.instance_version.clone() {
            ui.horizontal(|ui| {
                if icon_button(ui, Icon::Back, "К версиям").clicked() {
                    self.instance_version = None;
                }
                ui.strong(&version);
            });
            egui::ComboBox::from_id_salt("loader")
                .width(ui.available_width())
                .truncate()
                .selected_text(&self.instance_loader)
                .show_ui(ui, |ui| {
                    for loader in ["fabric", "forge", "neoforge", "quilt", "vanilla"] {
                        ui.selectable_value(&mut self.instance_loader, loader.into(), loader);
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut self.instance_name)
                    .min_size(Vec2::new(0., layout::ROW))
                    .hint_text("Название сборки (необязательно)"),
            );
            ui.checkbox(&mut self.instance_force_new, "Отдельная копия сборки");
            if ui
                .add_enabled(!self.busy, egui::Button::new("Добавить"))
                .on_hover_text("Игра и загрузчик пока не устанавливаются")
                .clicked()
            {
                let (loader, name) = (self.instance_loader.clone(), self.instance_name.clone());
                self.select_instance(&version, &loader, &name, self.instance_force_new);
                self.instance_version = None;
                self.instance_force_new = false;
                self.navigate(if loader == "vanilla" {
                    Page::Home
                } else {
                    Page::Mods
                });
            }
            return;
        }
        if self.version_tab == VersionTab::Builds {
            if !self.busy && self.editing_build.is_none() && self.delete_build.is_none() {
                self.prune_missing_builds();
            }
            self.my_builds(ui, ctx);
            return;
        }
        ui.horizontal(|ui| {
            for kind in versions::VersionKind::ALL {
                if ui
                    .checkbox(&mut self.config.version_filters[kind.index()], kind.label())
                    .changed()
                {
                    let result = self.config.save(&self.root);
                    self.result(result, "");
                }
            }
        });
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 48.).max(64.);
            ui.add_sized(
                [width, layout::ROW],
                egui::TextEdit::singleline(&mut self.version_query).hint_text("Найти версию"),
            );
            if ui
                .add_enabled_ui(!self.versions_loading, |ui| {
                    icon_button(ui, Icon::Refresh, "Обновить")
                })
                .inner
                .clicked()
            {
                self.load_versions(ctx);
            }
        });
        if self.versions_loading {
            ui.spinner();
        }
        let mut selected = None;
        let mut modded = None;
        let visible: Vec<_> = self
            .version_list
            .iter()
            .filter(|v| versions::matches(v, &self.config.version_filters, &self.version_query))
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("versions")
            .max_height((ui.available_height() - 40.).max(64.))
            .auto_shrink([false, false])
            .show_rows(ui, layout::ROW, visible.len(), |ui, rows| {
                for i in rows {
                    let v = visible[i];
                    ui.push_id(&v.id, |ui| {
                        let version_width = (ui.available_width() - 128.).max(100.);
                        egui::Grid::new("version_grid")
                            .num_columns(3)
                            .spacing(Vec2::splat(layout::GRID))
                            .show(ui, |ui| {
                                if ui
                                    .add_enabled_ui(!self.busy, |ui| {
                                        ui.add_sized(
                                            [version_width, layout::ROW],
                                            egui::Button::new(&v.id),
                                        )
                                    })
                                    .inner
                                    .on_hover_text("Выбрать Vanilla")
                                    .clicked()
                                {
                                    selected = Some(v.id.clone());
                                }
                                ui.add_sized(
                                    [72., layout::ROW],
                                    egui::Label::new(RichText::new(v.kind.label()).small()),
                                );
                                if ui
                                    .add_enabled_ui(!self.busy, |ui| {
                                        icon_button(ui, Icon::Plus, "Создать сборку с модами")
                                    })
                                    .inner
                                    .clicked()
                                {
                                    modded = Some(v.id.clone());
                                }
                            });
                    });
                }
            });
        if self.version_list.is_empty() {
            ui.label(&self.versions_status);
        } else {
            ui.add_sized(
                [ui.available_width(), layout::ROW],
                egui::Label::new(RichText::new(format!("{} версий", visible.len())).small()),
            )
            .on_hover_text(&self.versions_status);
        }
        if let Some(v) = selected {
            self.select_instance(&v, "vanilla", &format!("{v} · Vanilla"), false);
            self.navigate(Page::Home);
        }
        if let Some(v) = modded {
            self.instance_version = Some(v);
            self.instance_name.clear();
        }
    }

    fn mods(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !variant::LITE && self.config.curseforge_manual {
            ui.collapsing("CurseForge · ручная загрузка",|ui|{
                ui.label("Скачайте мод на официальном сайте через свой браузер, затем импортируйте JAR. Версию Minecraft, загрузчик и зависимости проверяет пользователь. Потребуется перезапуск игры.");
                if ui.button("Открыть CurseForge").clicked(){ui.ctx().open_url(egui::OpenUrl::new_tab("https://www.curseforge.com/minecraft/search?class=mc-mods"));}
                ui.checkbox(&mut self.manual_trust,"Доверяю выбранному JAR и проверил совместимость");
                if ui.add_enabled(self.manual_trust&&!self.busy&&self.sessions.is_empty()&&!self.content_mutation_busy()&&self.config.profiles[self.config.selected].loader!="vanilla",egui::Button::new("Импортировать скачанный JAR…")).clicked(){
                    if let Some(path)=rfd::FileDialog::new().add_filter("Java mod",&["jar"]).pick_file(){
                        self.manual_trust=false;
                        self.busy=true;
                        let(tx,ctx,instance)=(self.tx.clone(),ctx.clone(),self.config.instance_dir(&self.root));
                        std::thread::spawn(move||{let result=manual::import(&path,&instance).map(|_|1);let _=tx.send(Event::Installed(result));ctx.request_repaint();});
                    }
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            if ui::icon_tab(
                ui,
                Icon::Search,
                "Поиск",
                !self.resource_tab && !self.installed_tab,
            )
            .on_hover_text("Найти моды")
            .clicked()
            {
                self.resource_tab = false;
                self.installed_tab = false;
            }
            if ui::icon_tab(
                ui,
                Icon::Mods,
                "Мои",
                !self.resource_tab && self.installed_tab,
            )
            .on_hover_text(format!("Мои моды ({})", self.installed.len()))
            .clicked()
            {
                self.resource_tab = false;
                self.installed_tab = true;
            }
            if ui::icon_tab(
                ui,
                Icon::Resources,
                "Ресурспаки",
                self.resource_tab && !self.maps_tab,
            )
            .clicked()
                && !self.resource_catalog.busy
            {
                self.resource_tab = true;
                self.maps_tab = false;
            }
            if ui::icon_tab(
                ui,
                Icon::Builds,
                "Карты",
                self.resource_tab && self.maps_tab,
            )
            .clicked()
                && !self.resource_catalog.busy
            {
                self.resource_tab = true;
                self.maps_tab = true;
            }
        });
        if self.resource_tab {
            self.resource_catalog.set_section(if self.maps_tab {
                content::Section::Maps
            } else {
                content::Section::Resources
            });
            self.resource_catalog
                .ui(ui, ctx, &self.root, &self.config, &self.key, self.busy);
            return;
        }
        if self.config.profiles[self.config.selected].loader == "vanilla" {
            ui.label("Для модов нужна отдельная сборка.");
            if ui.button("+ Добавить сборку").clicked() {
                self.version_tab = VersionTab::Catalog;
                self.instance_version = Some(
                    self.config.profiles[self.config.selected]
                        .game_version
                        .clone(),
                );
                self.instance_name.clear();
                self.navigate(Page::Versions);
            }
        }
        if let Some(item) = self.remove_candidate.clone() {
            ui.group(|ui| {
                ui.label(format!("Удалить {}?", item.name));
                ui.horizontal(|ui| {
                    if ui
                        .button("Удалить")
                        .on_hover_text("Другие моды могут зависеть от него")
                        .clicked()
                    {
                        let result = api::remove(&self.config.instance_dir(&self.root), &item);
                        self.result(result, "Мод удалён");
                        self.refresh_installed();
                        self.remove_candidate = None;
                    }
                    if ui.button("Отмена").clicked() {
                        self.remove_candidate = None;
                    }
                });
            });
        }
        if self.installed_tab {
            if !self.installed_icons_requested {
                self.thumbnails.request(
                    self.installed.iter().map(|m| m.icon_url.clone()).collect(),
                    ctx,
                );
                self.installed_icons_requested = true;
            }
            let mut remove = None;
            let mut update = None;
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.busy && !self.installed.is_empty(),
                        egui::Button::new("Проверить обновления"),
                    )
                    .clicked()
                {
                    self.check_updates(ctx);
                }
                if self.busy && self.mod_cancel.is_some() {
                    ui.spinner();
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("installed")
                .max_height((ui.available_height() - 40.).max(64.))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let width = ui.available_width();
                    for item in &self.installed {
                        let disabled =
                            model::mod_path(&self.config.instance_dir(&self.root), &item.filename)
                                .is_ok_and(|path| {
                                    path.extension().is_some_and(|ext| ext == "disabled")
                                });
                        let has_update = self
                            .updates
                            .get(&Self::mod_key(item.source, &item.project_id))
                            .and_then(|value| value.as_ref())
                            .is_some_and(|version| *version != item.version_id);
                        // Anchor actions to the list edge before allocating the text column.
                        ui.allocate_ui_with_layout(
                            Vec2::new(width, layout::ROW),
                            egui::Layout::right_to_left(egui::Align::Min),
                            |ui| {
                                if ui
                                    .add_enabled_ui(!self.busy, |ui| {
                                        icon_button(ui, Icon::Trash, "Удалить")
                                    })
                                    .inner
                                    .clicked()
                                {
                                    remove = Some(item.clone());
                                }
                                if has_update {
                                    if ui
                                        .add_enabled_ui(!self.busy, |ui| {
                                            icon_button(ui, Icon::Refresh, "Доступно обновление")
                                        })
                                        .inner
                                        .clicked()
                                    {
                                        update = Some(item.clone());
                                    }
                                } else {
                                    ui.allocate_space(Vec2::new(40., layout::ROW));
                                }
                                let text_width =
                                    (ui.available_width() - 42. - ui.spacing().item_spacing.x)
                                        .max(0.);
                                ui.allocate_ui_with_layout(
                                    Vec2::new(text_width, 0.),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_width(text_width);
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(if disabled {
                                                    format!("{} (отключён)", item.name)
                                                } else {
                                                    item.name.clone()
                                                })
                                                .strong(),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(format!("{}\n{}", item.name, item.filename));
                                        if !item.description.is_empty() {
                                            ui.add(egui::Label::new(&item.description).wrap());
                                        }
                                        if !item.categories.is_empty() {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(item.categories.join(" · "))
                                                        .small(),
                                                )
                                                .wrap(),
                                            );
                                        }
                                    },
                                );
                                if let Some(texture) = self.thumbnails.get(&item.icon_url) {
                                    ui.add(
                                        egui::Image::new(texture)
                                            .fit_to_exact_size(Vec2::splat(42.))
                                            .corner_radius(4.),
                                    );
                                } else {
                                    let rect = ui
                                        .allocate_exact_size(Vec2::splat(42.), egui::Sense::hover())
                                        .0;
                                    ui::paint_icon(ui, Icon::Mods, rect, ui.visuals().text_color());
                                }
                            },
                        );
                    }
                    if self.installed.is_empty() {
                        ui.label("Пока пусто");
                    }
                });
            if let Some(item) = update {
                self.update_mod(item, ctx);
            }
            if let Some(item) = remove {
                self.remove_candidate = Some(item);
            }
            return;
        }
        ui.add_enabled_ui(!self.busy, |ui| {
            ui.horizontal(|ui| {
                let old = self.source;
                ui.selectable_value(&mut self.source, Source::Modrinth, "Modrinth");
                ui.selectable_value(&mut self.source, Source::CurseForge, "CurseForge");
                if old != self.source {
                    self.projects.clear();
                    self.searched = false;
                    self.search_options.category = None;
                    self.category_error.clear();
                }
            });
        });
        self.load_categories(ctx);
        let mut filter_changed = false;
        ui.add_enabled_ui(!self.busy, |ui| {
            ui.columns(2, |columns| {
                let previous = self.search_options.sort;
                let ui = &mut columns[0];
                egui::ComboBox::from_id_salt("mod_sort")
                    .width(ui.available_width())
                    .truncate()
                    .selected_text(self.search_options.sort.label())
                    .show_ui(ui, |ui| {
                        for sort in SortOrder::ALL {
                            ui.selectable_value(&mut self.search_options.sort, sort, sort.label());
                        }
                    });
                filter_changed |= previous != self.search_options.sort;
                let previous = self.search_options.category.clone();
                let ui = &mut columns[1];
                let selected = self
                    .categories
                    .get(&self.source)
                    .and_then(|categories| {
                        categories
                            .iter()
                            .find(|c| Some(&c.id) == self.search_options.category.as_ref())
                    })
                    .map(|c| c.name.as_str())
                    .unwrap_or("Все категории");
                egui::ComboBox::from_id_salt("mod_category")
                    .width(ui.available_width())
                    .truncate()
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.search_options.category,
                            None,
                            "Все категории",
                        );
                        if let Some(categories) = self.categories.get(&self.source) {
                            for category in categories {
                                ui.selectable_value(
                                    &mut self.search_options.category,
                                    Some(category.id.clone()),
                                    &category.name,
                                );
                            }
                        }
                    });
                filter_changed |= previous != self.search_options.category;
            });
        });
        if self.categories_loading.contains(&self.source) {
            ui.spinner();
        }
        if !self.category_error.is_empty() {
            ui.horizontal(|ui| {
                ui.label("Категории недоступны")
                    .on_hover_text(&self.category_error);
                if ui.small_button("Повторить").clicked() {
                    self.categories.remove(&self.source);
                    self.load_categories(ctx);
                }
            });
        }
        let mut search = false;
        ui.add_enabled_ui(!self.busy, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 48.).max(64.);
                let r = ui.add_sized(
                    [width, layout::ROW],
                    egui::TextEdit::singleline(&mut self.query).hint_text("Название мода"),
                );
                #[cfg(test)]
                ctx.data_mut(|data| data.insert_temp(egui::Id::new("mods_search_bounds"), r.rect));
                if r.changed() {
                    ctx.request_repaint_after(std::time::Duration::from_millis(360));
                }
                search = icon_button(ui, Icon::Search, "Найти").clicked()
                    || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            });
        });
        let now = ctx.input(|i| i.time);
        let profile = &self.config.profiles[self.config.selected];
        let signature = format!(
            "{}|{}|{}|{}|{}|{:?}",
            self.source,
            self.query.trim(),
            profile.game_version,
            profile.loader,
            self.search_options.sort.modrinth(),
            self.search_options.category
        );
        if self
            .mod_browse
            .sync(signature, now, self.searched && !self.projects.is_empty())
        {
            self.projects.clear();
            self.searched = false;
        }
        if search || filter_changed {
            self.mod_browse.reset(now);
            self.projects.clear();
        }
        if self.mod_browse.pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.mod_browse.due(now) && !self.busy {
            self.search(ctx);
        }
        if self.source == Source::CurseForge
            && self.key.is_empty()
            && ui.link("Укажите ключ CurseForge в настройках").clicked()
        {
            self.navigate(Page::Settings);
        }
        let mut install = None;
        let mut link = None;
        let mut list = egui::ScrollArea::vertical();
        if std::mem::take(&mut self.mod_browse.reset_scroll) {
            list = list.vertical_scroll_offset(0.);
        }
        let _list_area = list
            .id_salt("mod_results")
            .max_height((ui.available_height() - 40.).max(64.))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.projects.is_empty()
                    && !self.busy
                    && !self.mod_browse.loading
                    && !self.mod_browse.pending()
                {
                    if self.error && !self.status.is_empty() {
                        ui.colored_label(ui.visuals().error_fg_color, &self.status);
                    } else {
                        ui.label(if self.searched {
                            "Ничего не найдено"
                        } else {
                            "Введите название или нажмите поиск"
                        });
                    }
                }
                for p in &self.projects {
                    ui.push_id(&p.id, |ui| {
                        ui.group(|ui| {
                            ui.set_min_width(ui.available_width());
                            // Reserve 8 px per gap so the actions never spill out of the card.
                            let width = ui.available_width();
                            let text_width =
                                (width - 56. - 40. - 40. - 3. * layout::GRID - layout::GRID)
                                    .max(64.);
                            ui.horizontal(|ui| {
                                ui::catalog_thumbnail(
                                    ui,
                                    self.thumbnails.get(&p.icon_url),
                                    Icon::Mods,
                                    56.,
                                );
                                ui.vertical(|ui| {
                                    ui.set_width(text_width);
                                    ui.add(
                                        egui::Label::new(RichText::new(&p.name).strong())
                                            .truncate(),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "{} · ↓ {}",
                                                p.author,
                                                compact_number(p.downloads)
                                            ))
                                            .small(),
                                        )
                                        .truncate(),
                                    );
                                });
                                let exists = self
                                    .installed
                                    .iter()
                                    .any(|m| m.source == p.source && m.project_id == p.id);
                                if ui
                                    .add_enabled_ui(
                                        !self.busy
                                            && !exists
                                            && self.config.profiles[self.config.selected].loader
                                                != "vanilla",
                                        |ui| {
                                            icon_button(
                                                ui,
                                                if exists { Icon::Check } else { Icon::Download },
                                                if exists {
                                                    "Установлено"
                                                } else {
                                                    "Установить с зависимостями"
                                                },
                                            )
                                        },
                                    )
                                    .inner
                                    .clicked()
                                {
                                    install = Some(p.clone());
                                }
                                if icon_button(ui, Icon::Link, "Страница проекта").clicked()
                                {
                                    link = Some(p.website.clone());
                                }
                            });
                            ui.add(egui::Label::new(&p.description).wrap());
                            ui.horizontal_wrapped(|ui| {
                                for category in p
                                    .categories
                                    .iter()
                                    .filter(|c| {
                                        !matches!(
                                            c.as_str(),
                                            "fabric" | "forge" | "quilt" | "neoforge"
                                        )
                                    })
                                    .take(6)
                                {
                                    egui::Frame::new()
                                        .fill(ui.visuals().selection.bg_fill)
                                        .inner_margin(egui::Margin::symmetric(6, 2))
                                        .corner_radius(4.)
                                        .show(ui, |ui| {
                                            ui.label(RichText::new(category).size(10.));
                                        });
                                }
                            });
                        });
                    });
                }
            });
        #[cfg(test)]
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("mods_list_bounds"), _list_area.inner_rect)
        });
        if let Some(p) = install {
            self.install(p, ctx);
        }
        if let Some(link) = link {
            if let Ok(url) = reqwest::Url::parse(&link) {
                if url.scheme() == "https"
                    && matches!(
                        url.host_str(),
                        Some("modrinth.com" | "www.curseforge.com" | "curseforge.com")
                    )
                {
                    let result = open::that(link).map_err(Into::into);
                    self.result(result, "");
                }
            }
        }
        if !self.busy
            && !self.mod_browse.loading
            && !self.mod_browse.pending()
            && self.mod_browse.more
            && !self.mod_browse.failed
            && browse::near_end(
                _list_area.state.offset.y,
                _list_area.inner_rect.height(),
                _list_area.content_size.y,
            )
        {
            self.search(ctx);
        }
        let _footer = ui::catalog_footer(
            ui,
            self.projects.len(),
            self.mod_browse.loading,
            self.mod_browse.failed,
        );
        if _footer.inner {
            self.search(ctx);
        }
        #[cfg(test)]
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("mods_footer_bounds"), _footer.response.rect)
        });
    }

    fn detail_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let title = match self.page {
            Page::Accounts => "Аккаунты",
            Page::Settings => "Настройки",
            Page::Versions => match self.version_tab.group(self.config.library_tabs) {
                model::LibraryGroup::Catalog => "Каталог версий",
                model::LibraryGroup::MyBuilds => "Мои сборки",
            },
            Page::Mods => "Моды и ресурспаки",
            Page::About => "О проекте",
            Page::Logs => "Журнал запуска",
            Page::Help => "ИИ-помощник",
            Page::BuildEdit => "Редактирование сборки",
            Page::Home => "",
        };
        ui.horizontal(|ui| {
            ui.strong(title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, Icon::Home, "На главную").clicked() {
                    self.navigate(Page::Home);
                }
                if icon_button(ui, Icon::Refresh, "Сбросить размер панели").clicked()
                {
                    let id = self.panel_id();
                    self.config.panels.set(id, id.default_size());
                    self.layout_dirty_at = Some(ctx.input(|i| i.time));
                    self.persist_layout();
                    ctx.request_repaint();
                }
            });
        });
        ui.separator();
        if !variant::LITE && self.page == Page::Help {
            self.ai_ui(ui, ctx);
        } else if self.page == Page::BuildEdit {
            self.edit_instance(ui, ctx);
        } else if self.page == Page::Mods {
            self.mods(ui, ctx);
        } else if self.page == Page::Versions && self.instance_version.is_none() {
            self.version_picker(ui, ctx);
        } else {
            egui::ScrollArea::vertical().id_salt(("detail",self.panel_id())).auto_shrink([false,false]).max_height(ui.available_height()).show(ui,|ui|{match self.page{
            Page::Accounts=>self.accounts(ui,ctx),Page::Settings=>self.settings(ui,ctx),Page::Versions=>self.version_picker(ui,ctx),
            Page::About=>{
                ui.label(format!("{} {} · игровая бета",variant::NAME,env!("CARGO_PKG_VERSION")));
                ui.label("Vanilla, Fabric, Quilt, Forge и NeoForge. Аккаунты Ely.by, Microsoft и без пароля. Сессии хранятся зашифрованными через Windows DPAPI.");
                if !variant::LITE {
                    ui.label("Оверлей Shift+Tab: экспериментальная поддержка Windows x64 / OpenGL (GL 3+). Vulkan и VulkanMod НЕ поддерживаются. Возможны конфликты с Discord/Steam и античитами. Отключите оверлей и перезапустите игру при проблемах. Minecraft-мод не устанавливается.");
                }
                if ui.button("Журнал запуска").clicked(){self.navigate(Page::Logs);}
            },
            Page::Logs=>{
                if !variant::LITE&&ui.button("Разобрать ошибку с ИИ").clicked(){self.prepare_ai_report();self.navigate(Page::Help);}
                if ui.button("Открыть папку журналов").clicked(){let path=self.config.instance_dir(&self.root).join("launcher/logs");let result=std::fs::create_dir_all(&path).map_err(anyhow::Error::from).and_then(|_|open::that(path).map_err(Into::into));self.result(result,"");}
                for line in &self.game_logs{ui.add(egui::Label::new(RichText::new(line).monospace().size(11.)).wrap());}
            },_=>{}
        }});
        }
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        self.persist_layout();
        use zeroize::Zeroize;
        self.account_password.zeroize();
        self.account_totp.zeroize();
        self.key.zeroize();
        if let Some(tx) = &self.cancel_auth {
            let _ = tx.send(());
        }
    }
}
impl Launcher {
    fn import_wallpaper(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.wallpaper_busy {
            return;
        }
        self.wallpaper_busy = true;
        let (root, tx, ctx) = (self.root.clone(), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Event::Wallpaper(media::import_background(&path, &root)));
            ctx.request_repaint();
        });
    }
    fn ai_settings(&mut self, ui: &mut egui::Ui) {
        ui.strong("ИИ-помощник · OpenAI-совместимый API");
        ui.label("URL запроса (полный, с /chat/completions)");
        ui.text_edit_singleline(&mut self.config.assistant.endpoint);
        ui.label("Модель");
        ui.text_edit_singleline(&mut self.config.assistant.model);
        ui.label("API-ключ (шифруется DPAPI)");
        ui.add(egui::TextEdit::singleline(&mut *self.ai_key).password(true));
        ui.small("Журналы не отправляются автоматически. Перед отправкой можно проверить и отредактировать отчёт. Ответ ИИ — совет, не автоматическое исправление.");
        if ui.button("Открыть помощника").clicked() {
            self.navigate(Page::Help);
        }
    }
    fn prepare_ai_report(&mut self) {
        let p = &self.config.profiles[self.config.selected];
        let lines = self
            .game_logs
            .iter()
            .rev()
            .take(450)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let report=format!("Вопрос: {}\nMinecraft: {}\nЗагрузчик: {}\nПамять: {}–{} МиБ\nПоследний результат: {}\nЖурнал (последние строки):\n{}",self.ai_question,p.game_version,p.loader,self.config.java.min_memory,self.config.java.max_memory,self.status,lines);
        self.ai_report = self.sanitize_ai_report(&report);
    }
    fn sanitize_ai_report(&self, report: &str) -> String {
        let mut secrets = vec![self.ai_key.as_str(), self.key.as_str()];
        for a in &self.config.accounts {
            if let Some(s) = &a.session {
                secrets.push(&s.access_token);
                secrets.push(&s.client_token);
                if let Some(t) = &s.refresh_token {
                    secrets.push(t);
                }
            }
        }
        assistant::sanitize(report, &secrets)
    }
    fn ai_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::ScrollArea::vertical()
            .id_salt("ai_helper")
            .show(ui, |ui| {
                ui.label("Опишите вылет, баг или проблему запуска");
                ui.add(
                    egui::TextEdit::multiline(&mut self.ai_question)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    if ui.button("Подготовить отчёт").clicked() {
                        self.prepare_ai_report();
                    }
                    if ui.button("Настройка API").clicked() {
                        self.show_appearance = false;
                        self.navigate(Page::Settings);
                    }
                });
                if ui.button("Добавить crash-report / latest.log…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Журнал", &["log", "txt"])
                        .pick_file()
                    {
                        use std::io::Read;
                        let result = std::fs::File::open(&path).and_then(|f| {
                            let mut bytes = vec![];
                            f.take(256 * 1024 + 1).read_to_end(&mut bytes)?;
                            Ok(bytes)
                        });
                        match result {
                            Ok(bytes) if bytes.len() <= 256 * 1024 => {
                                if self.ai_report.is_empty() {
                                    self.prepare_ai_report();
                                }
                                let report = format!(
                                    "{}\n\nДополнительный журнал:\n{}",
                                    self.ai_report,
                                    String::from_utf8_lossy(&bytes)
                                );
                                self.ai_report = self.sanitize_ai_report(&report);
                            }
                            Ok(_) => self.ai_answer =
                                "Файл больше 256 КиБ. Вставьте только фрагмент с ошибкой в отчёт."
                                    .into(),
                            Err(error) => {
                                self.ai_answer = format!("Не удалось прочитать журнал: {error}")
                            }
                        }
                    }
                }
                if !self.ai_report.is_empty() {
                    ui.label("Проверьте данные. Этот текст будет передан выбранному вами API:");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.ai_report)
                            .desired_rows(8)
                            .desired_width(f32::INFINITY),
                    );
                    ui.label(format!("Получатель: {}", self.config.assistant.endpoint));
                    if ui
                        .add_enabled(
                            !self.ai_busy,
                            egui::Button::new("Подтверждаю отправку — проанализировать"),
                        )
                        .clicked()
                    {
                        let settings = self.config.assistant.clone();
                        let key = zeroize::Zeroizing::new(self.ai_key.to_string());
                        let report = self.sanitize_ai_report(&self.ai_report);
                        let (tx, ctx) = (self.tx.clone(), ctx.clone());
                        self.ai_busy = true;
                        std::thread::spawn(move || {
                            let _ =
                                tx.send(Event::Analysis(assistant::analyze(settings, key, report)));
                            ctx.request_repaint();
                        });
                    }
                }
                if self.ai_busy {
                    ui.spinner();
                }
                if !self.ai_answer.is_empty() {
                    ui.separator();
                    ui.label(&self.ai_answer);
                    if ui.button("Скопировать ответ").clicked() {
                        ctx.copy_text(self.ai_answer.clone());
                    }
                }
            });
    }
    fn title_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("custom_titlebar")
            .exact_height(38.)
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.window_fill)
                    .inner_margin(egui::Margin::symmetric(10, 3)),
            )
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let drag = egui::Rect::from_min_max(
                    rect.min,
                    egui::pos2(rect.right() - 200., rect.bottom()),
                );
                let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ui.horizontal(|ui| {
                    if let Some(brand) = &self.brand {
                        ui.add(egui::Image::new(brand).fit_to_exact_size(Vec2::splat(24.)));
                    }
                    ui.label(variant::NAME);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui::icon_button_size(ui, Icon::Close, "Закрыть", Vec2::new(32., 28.))
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui::icon_button_size(
                            ui,
                            if maximized {
                                Icon::Restore
                            } else {
                                Icon::Maximize
                            },
                            "Развернуть / восстановить",
                            Vec2::new(32., 28.),
                        )
                        .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        }
                        if ui::icon_button_size(ui, Icon::Minimize, "Свернуть", Vec2::new(32., 28.))
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        if !variant::LITE
                            && ui.button("ИИ").on_hover_text("Помощь при вылете").clicked()
                        {
                            self.toggle_page(Page::Help);
                        }
                    });
                });
                // Register the drag area after the title text, otherwise labels can win hit-testing.
                let response = ui.interact(
                    drag,
                    ui.id().with("window_drag"),
                    egui::Sense::click_and_drag(),
                );
                if response.drag_started() {
                    if maximized {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.double_clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
            });
        // Native decorations are disabled; preserve edge/corner resize gestures.
        let screen = ctx.screen_rect();
        for (name, rect, direction, cursor) in [
            (
                "left",
                egui::Rect::from_min_size(screen.min, Vec2::new(4., screen.height())),
                egui::ResizeDirection::West,
                egui::CursorIcon::ResizeHorizontal,
            ),
            (
                "right",
                egui::Rect::from_min_size(
                    egui::pos2(screen.right() - 4., screen.top()),
                    Vec2::new(4., screen.height()),
                ),
                egui::ResizeDirection::East,
                egui::CursorIcon::ResizeHorizontal,
            ),
            (
                "top",
                egui::Rect::from_min_size(screen.min, Vec2::new(screen.width(), 4.)),
                egui::ResizeDirection::North,
                egui::CursorIcon::ResizeVertical,
            ),
            (
                "bottom",
                egui::Rect::from_min_size(
                    egui::pos2(screen.left(), screen.bottom() - 4.),
                    Vec2::new(screen.width(), 4.),
                ),
                egui::ResizeDirection::South,
                egui::CursorIcon::ResizeVertical,
            ),
            (
                "corner",
                egui::Rect::from_min_size(screen.max - Vec2::splat(12.), Vec2::splat(12.)),
                egui::ResizeDirection::SouthEast,
                egui::CursorIcon::ResizeNwSe,
            ),
        ] {
            egui::Area::new(egui::Id::new(("viewport_edge", name)))
                .fixed_pos(rect.min)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let r = ui
                        .allocate_response(rect.size(), egui::Sense::drag())
                        .on_hover_cursor(cursor);
                    if r.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
                    }
                });
        }
    }
    fn render(&mut self, ctx: &egui::Context) -> egui::Rect {
        self.process_events(ctx);
        if let Some(profile) = self.content.drain(ctx) {
            self.config.profiles.push(profile);
            self.config.selected = self.config.profiles.len() - 1;
            self.changed_instance();
        }
        self.resource_catalog.drain(ctx);
        self.avatars.refresh(&self.config.accounts, &self.root, ctx);
        self.avatars.drain(ctx);
        self.thumbnails.drain(ctx);
        self.title_bar(ctx);
        if self.config.appearance != self.applied_appearance {
            if self.config.appearance.background != self.applied_appearance.background {
                self.background.load(
                    &self.config.appearance.background,
                    self.config.appearance.animate,
                    ctx,
                );
            }
            self.background.animate(self.config.appearance.animate);
            self.config.appearance.apply(ctx);
            self.applied_appearance = self.config.appearance.clone();
        }
        let mut custom_background = false;
        let mut background_click = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                // A click on genuinely free workspace closes every open side panel.
                background_click = ui
                    .interact(
                        ui.max_rect(),
                        egui::Id::new("workspace-close-background"),
                        egui::Sense::click(),
                    )
                    .clicked();
                let time = if self.config.appearance.animate {
                    ctx.input(|i| i.time) as f32
                } else {
                    0.
                };
                custom_background = self.background.paint(ui);
                let effect_allowed =
                    !variant::LITE || self.config.appearance.effect.lite_supported();
                if !custom_background
                    && self.config.appearance.background.is_empty()
                    && effect_allowed
                {
                    effects::paint(ui, time, &self.config.appearance);
                } else if !custom_background {
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0., ui.visuals().window_fill);
                }
                ui.painter().rect_filled(
                    ui.max_rect(),
                    0.,
                    Color32::from_black_alpha(self.config.appearance.dim),
                );
            });
        if background_click {
            self.close_pages();
        }
        if self.config.appearance.animate
            && !custom_background
            && (!variant::LITE || !matches!(self.config.appearance.effect, effects::Effect::None))
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        let screen = egui::Rect::from_min_max(
            ctx.screen_rect().min + Vec2::new(0., 38.),
            ctx.screen_rect().max,
        );
        let time = ctx.input(|i| i.time);
        let real_page = self.page;
        let detail = (real_page != Page::Home).then(|| self.panel_id_of(real_page));
        let secondary = self
            .secondary_page
            .filter(|p| *p != Page::Home && Some(self.panel_id_of(*p)) != detail);
        let secondary_id = secondary.map(|p| self.panel_id_of(p));
        let geometry = layout::Geometry::new_two(screen, &self.config.panels, secondary_id, detail);
        let mut targets = vec![(layout::PanelId::Launcher, Page::Home, geometry.launcher)];
        if let (Some(page), Some(id), Some(rect)) = (secondary, secondary_id, geometry.left) {
            targets.push((id, page, rect));
        }
        if let (Some(id), Some(rect)) = (detail, geometry.detail) {
            targets.push((id, real_page, rect));
        }
        let transitions = self.config.appearance.transitions;
        for (id, page, rect) in &targets {
            let motion = self.panel_motion.entry(*id).or_insert_with(|| {
                PanelMotion::new(
                    *page,
                    *rect,
                    time,
                    transitions && *id != layout::PanelId::Launcher,
                )
            });
            motion.page = *page;
            motion.target(*rect, 1., time, transitions);
        }
        for (id, motion) in &mut self.panel_motion {
            if !targets.iter().any(|(target, _, _)| target == id) {
                motion.target(motion.to_rect, 0., time, transitions);
            }
        }
        let mut panels: Vec<_> = self
            .panel_motion
            .iter()
            .map(|(id, motion)| (*id, *motion))
            .collect();
        // The launcher is drawn first, followed by outgoing and current detail panels.
        panels.sort_by_key(|(id, motion)| {
            (
                *id != layout::PanelId::Launcher,
                motion.to_alpha > 0.,
                id.key(),
            )
        });
        let mut resize = vec![];
        for (id, motion) in panels {
            let (rect, opacity) = motion.sample(time);
            if motion.active(time) {
                ctx.request_repaint();
            }
            if opacity <= 0. {
                continue;
            }
            let outgoing = motion.to_alpha == 0.;
            let request = if id == layout::PanelId::Launcher {
                layout::panel(ctx, id, rect, |ui| self.launch_panel(ui))
            } else {
                let current_page = self.page;
                self.page = motion.page;
                let enabled = !outgoing
                    && (!self.content_mutation_busy()
                        || self.content.mutation_busy() && motion.page == Page::Versions
                        || self.resource_catalog.mutation_busy() && motion.page == Page::Mods);
                let result = layout::panel_animated(ctx, id, rect, opacity, |ui| {
                    ui.add_enabled_ui(enabled, |ui| self.detail_panel(ui, ctx));
                });
                if outgoing || self.page == motion.page {
                    self.page = current_page;
                }
                result
            };
            if !outgoing {
                if let Some(request) = request {
                    resize.push(request);
                }
            }
        }
        self.panel_motion
            .retain(|_, motion| motion.to_alpha > 0. || motion.active(time));
        for request in resize {
            self.resize_panel(request, screen, ctx);
        }
        // Save the normal outer viewport size too; maximizing is temporary, not a new preference.
        if let Some(size) = ctx.input(|i| {
            let viewport = i.viewport();
            if viewport.maximized == Some(true) || viewport.fullscreen == Some(true) {
                None
            } else {
                viewport.inner_rect.map(|rect| rect.size())
            }
        }) {
            let value = [
                layout::snap(size.x.max(720.)),
                layout::snap(size.y.max(560.)),
            ];
            if value != self.config.panels.viewport {
                self.config.panels.viewport = value;
                self.layout_dirty_at = Some(ctx.input(|i| i.time));
            }
        }
        if let Some(changed) = self.layout_dirty_at {
            if ctx.input(|i| i.time) - changed > 0.4 && !ctx.input(|i| i.pointer.any_down()) {
                self.persist_layout();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(450));
            }
        }
        if self.busy || self.error || !self.status.is_empty() || self.game_cancel.is_some() {
            egui::Area::new("status".into())
                .anchor(egui::Align2::LEFT_BOTTOM, [18., -15.])
                .show(ctx, |ui| {
                    ui.set_max_width(ctx.screen_rect().width() - 36.);
                    ui.horizontal(|ui| {
                        if self.busy || self.game_cancel.is_some() {
                            ui.spinner();
                        }
                        ui.label(
                            RichText::new(if self.game_cancel.is_some() {
                                &self.launch_status
                            } else {
                                &self.status
                            })
                            .size(12.)
                            .color(if self.error {
                                Color32::from_rgb(255, 180, 165)
                            } else {
                                Color32::from_gray(200)
                            }),
                        );
                        if self.game_cancel.is_some() && ui.small_button("Журнал").clicked() {
                            self.navigate(Page::Logs);
                        }
                        if let Some(cancel) = &self.mod_cancel {
                            if ui.small_button("Отмена").clicked() {
                                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        if !self.busy && ui.small_button("×").clicked() {
                            self.status.clear();
                            self.error = false;
                        }
                    });
                });
        }
        geometry.workspace
    }
}
impl eframe::App for Launcher {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        self.tray.bind(_frame);
        self.render(ctx);
    }
}

fn compact_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} млн", n as f64 / 1_000_000.)
    } else if n >= 1000 {
        format!("{:.1} тыс", n as f64 / 1000.)
    } else {
        n.to_string()
    }
}

fn smoke_test() -> anyhow::Result<()> {
    let all = versions::fetch()?;
    for kind in versions::VersionKind::ALL {
        assert!(all.iter().any(|v| v.kind == kind));
    }
    println!("Mojang: {} versions, all categories OK", all.len());
    let api = api::Api::new(std::env::var("CURSEFORGE_API_KEY").unwrap_or_default())?;
    let profile = Profile {
        java: None,
        name: "Test".into(),
        game_version: "1.21.1".into(),
        loader: "fabric".into(),
        directory: "Test".into(),
    };
    let projects = api.search(Source::Modrinth, "sodium", &profile, 0)?;
    let p = projects
        .iter()
        .find(|p| p.name == "Sodium")
        .ok_or_else(|| anyhow::anyhow!("Sodium not found"))?;
    let categories = api.categories(Source::Modrinth)?;
    assert!(!categories.is_empty());
    assert!(!p.categories.is_empty());
    let options = SearchOptions {
        sort: SortOrder::Downloads,
        category: Some("optimization".into()),
    };
    let popular = api.search_filtered(Source::Modrinth, "", &profile, 0, &options)?;
    assert!(!popular.is_empty());
    assert!(popular.windows(2).all(|p| p[0].downloads >= p[1].downloads));
    assert!(popular
        .iter()
        .all(|p| p.categories.iter().any(|c| c == "optimization")));
    println!("Categories and download sorting OK");
    media::smoke_test(&p.icon_url)?;
    let dir = tempfile::tempdir()?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    assert!(api.install(p, &profile, dir.path(), &cancel, |s| println!("{s}"))? > 0);
    assert_eq!(api.install(p, &profile, dir.path(), &cancel, |_| {})?, 0);
    let installed = model::installed(dir.path())?;
    assert_eq!(
        api.latest_version(Source::Modrinth, &p.id, &profile)?
            .as_deref(),
        Some(installed[0].version_id.as_str())
    );
    assert!(!api.update(&installed[0], &profile, dir.path(), &cancel, |_| {})?);
    println!("Mod update check OK");
    for item in model::installed(dir.path())? {
        api::remove(dir.path(), &item)?;
    }
    assert!(model::installed(dir.path())?.is_empty());
    let projects = api.search(Source::Modrinth, "iris", &profile, 0)?;
    let iris = projects
        .iter()
        .find(|p| p.id == "YL57xq9U")
        .ok_or_else(|| anyhow::anyhow!("Iris not found"))?;
    assert!(api.install(iris, &profile, dir.path(), &cancel, |s| println!("{s}"))? >= 2);
    println!("Search, verified install, dependencies, idempotency and removal OK");
    Ok(())
}
fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--overlay-smoke-test") {
        return overlay::smoke_test();
    }
    if std::env::args().any(|a| a == "--catalog-smoke-test") {
        return packs::catalog_smoke_test();
    }
    if std::env::args().any(|a| a == "--loader-smoke-test") {
        return game::loader_smoke_test();
    }
    if std::env::args().any(|a| a == "--content-smoke-test") {
        return packs::smoke_test();
    }
    if std::env::args().any(|a| a == "--smoke-test") {
        return smoke_test();
    }
    let root = directories::ProjectDirs::from("dev", variant::DATA_NAME, variant::DATA_NAME)
        .ok_or_else(|| anyhow::anyhow!("Нет каталога данных"))?
        .data_local_dir()
        .to_path_buf();
    let had_config = root.join("config.json").exists();
    let mut config = Config::load(&root)?;
    variant::configure(&mut config);
    if !had_config {
        let _ = std::fs::create_dir_all(config.instance_dir(&root));
    }
    let (migrated, warnings) = config.migrate_directories(&root);
    if migrated {
        let _ = config.save(&root);
    }
    let startup_warning = warnings.join(" ");
    let icon = icons::icon_data().ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(config.panels.viewport_size())
        .with_min_inner_size([720., 560.])
        .with_decorations(false)
        .with_resizable(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let title = format!("{} {}", variant::NAME, env!("CARGO_PKG_VERSION"));
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            Ok(Box::new(Launcher::new(
                &cc.egui_ctx,
                root,
                config,
                true,
                startup_warning,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    #[test]
    #[cfg(feature = "lite")]
    fn lite_idle_does_not_schedule_an_animated_background() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut app = Launcher::new(
            &ctx,
            root.path().into(),
            Config::default(),
            false,
            String::new(),
        );
        app.config.appearance.animate = false;
        app.config.appearance.effect = effects::Effect::None;
        for time in 0..6 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960., 640.),
                    )),
                    time: Some(time as f64),
                    ..Default::default()
                },
                |ctx| {
                    app.render(ctx);
                },
            );
            if time == 5 {
                assert!(
                    output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                        > std::time::Duration::from_millis(60)
                );
            }
        }
    }
    #[test]
    fn finishing_one_session_preserves_the_other() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut app = Launcher::new(
            &ctx,
            root.path().into(),
            Config::default(),
            false,
            String::new(),
        );
        for id in [1, 2] {
            app.sessions.push(GameSession {
                id,
                name: format!("Session {id}"),
                instance: root.path().join(format!("{id}")),
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                running: true,
            });
        }
        app.game_cancel = Some(app.sessions[0].cancel.clone());
        app.game_running = true;
        app.tx
            .send(Event::Game(2, game::Event::Log("Second session".into())))
            .unwrap();
        app.tx
            .send(Event::Game(1, game::Event::Finished(Ok(0))))
            .unwrap();
        app.process_events(&ctx);
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.sessions[0].id, 2);
        assert!(app.game_running && app.game_cancel.is_some());
        assert!(app.game_logs[0].contains("[Сессия 2]"));
        app.tx
            .send(Event::Game(2, game::Event::Finished(Ok(0))))
            .unwrap();
        app.process_events(&ctx);
        assert!(app.sessions.is_empty() && app.game_cancel.is_none() && !app.game_running);
    }
    #[test]
    fn download_and_exchange_categories_stay_in_versions_panel() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut app = Launcher::new(
            &ctx,
            root.path().into(),
            Config::default(),
            false,
            String::new(),
        );
        app.page = Page::Versions;
        for tab in [VersionTab::Packs, VersionTab::Exchange] {
            app.version_tab = tab;
            for i in 0..3 {
                frame(
                    &mut app,
                    &ctx,
                    Vec2::new(720., 560.),
                    i as f64 * 0.1,
                    vec![],
                );
            }
            assert!(app.page == Page::Versions);
            assert_eq!(app.panel_id(), layout::PanelId::Versions);
            let (inner, used) = layout::content_bounds(&ctx, layout::PanelId::Versions).unwrap();
            assert!(used.right() <= inner.right() + 1.);
            assert!(used.bottom() <= inner.bottom() + 1.);
        }
    }
    #[test]
    fn closing_two_panels_keeps_each_geometry_until_fade_finishes() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.appearance.transitions = true;
        let mut app = Launcher::new(&ctx, root.path().into(), config, false, String::new());
        app.page = Page::Accounts;
        app.secondary_page = Some(Page::Settings);
        let size = Vec2::new(1600., 1000.);
        frame(&mut app, &ctx, size, 0., vec![]);
        frame(&mut app, &ctx, size, 0.3, vec![]);
        let accounts = app.panel_motion[&layout::PanelId::Accounts].sample(0.3).0;
        let settings = app.panel_motion[&layout::PanelId::Settings].sample(0.3).0;
        app.close_pages();
        frame(&mut app, &ctx, size, 1., vec![]);
        for (id, rect) in [
            (layout::PanelId::Accounts, accounts),
            (layout::PanelId::Settings, settings),
        ] {
            let motion = app.panel_motion[&id];
            assert_eq!(motion.sample(1.).0, rect);
            assert_eq!(motion.sample(1.).1, 1.);
            assert!(motion.sample(1.1).1 > 0. && motion.sample(1.1).1 < 1.);
        }
        // Reopening halfway starts at the current position and opacity.
        let before = app.panel_motion[&layout::PanelId::Accounts].sample(1.1);
        app.toggle_page(Page::Accounts);
        frame(&mut app, &ctx, size, 1.1, vec![]);
        assert_eq!(
            app.panel_motion[&layout::PanelId::Accounts].sample(1.1),
            before
        );
        frame(&mut app, &ctx, size, 1.4, vec![]);
        assert!(!app.panel_motion.contains_key(&layout::PanelId::Settings));
        assert_eq!(
            app.panel_motion[&layout::PanelId::Accounts].sample(1.4).1,
            1.
        );
    }

    #[test]
    fn installed_rows_keep_actions_beside_their_text() {
        for width in [360., 640., 1128.] {
            for (updates, busy) in [(false, false), (true, false), (true, true)] {
                check_installed_rows(width, updates, busy);
            }
        }
    }

    fn check_installed_rows(width: f32, updates: bool, busy: bool) {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.profiles[0].loader = "fabric".into();
        std::fs::create_dir_all(
            root.path()
                .join("instances")
                .join(&config.profiles[0].directory),
        )
        .unwrap();
        let mut app = Launcher::new(&ctx, root.path().into(), config, false, String::new());
        app.page = Page::Mods;
        app.installed_tab = true;
        let description =
            "A long example description that wraps onto several lines inside the narrow panel.";
        for i in 0..3 {
            app.installed.push(InstalledMod {
                name: format!(
                    "Example {i} with a very long title that must fit in the text column"
                ),
                description: description.into(),
                filename: "example.jar".into(),
                categories: vec![
                    "fabric".into(),
                    "neoforge".into(),
                    "optimization".into(),
                    "decoration".into(),
                    "quilt".into(),
                ],
                icon_url: String::new(),
                project_id: format!("p{i}"),
                source: Source::Modrinth,
                version_id: "v".into(),
            });
        }
        if updates {
            for i in [0, 2] {
                app.updates.insert(
                    Launcher::mod_key(Source::Modrinth, &format!("p{i}")),
                    Some("new".into()),
                );
            }
        }
        app.busy = busy;
        let size = Vec2::new(1600., 1000.);
        let mut titles = Vec::new();
        let mut trashes = Vec::new();
        for i in 0..4 {
            titles.clear();
            trashes.clear();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    time: Some(i as f64 * 0.1),
                    ..Default::default()
                },
                |ctx| {
                    layout::panel(
                        ctx,
                        layout::PanelId::Mods,
                        egui::Rect::from_min_size(egui::pos2(16., 16.), Vec2::new(width, 800.)),
                        |ui| {
                            app.mods(ui, ctx);
                        },
                    );
                },
            );
            for clipped in &output.shapes {
                match &clipped.shape {
                    egui::epaint::Shape::Text(text) => {
                        if text.galley.job.text.starts_with("Example ") {
                            titles
                                .push((text.galley.job.text.clone(), text.visual_bounding_rect()));
                        }
                    }
                    // Action buttons are 40x32 widgets inside the list columns.
                    egui::epaint::Shape::Rect(shape)
                        if shape.rect.width().round() == 40.
                            && shape.rect.height().round() == 32. =>
                    {
                        assert!(
                            clipped.clip_rect.contains_rect(shape.rect),
                            "action must be fully visible: {:?} / {:?}",
                            shape.rect,
                            clipped.clip_rect
                        );
                        trashes.push(shape.rect);
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(titles.len(), 3, "every installed mod needs a title line");
        let (inner, _) = layout::content_bounds(&ctx, layout::PanelId::Mods).unwrap();
        let row_actions = trashes;
        assert_eq!(row_actions.len(), if updates { 5 } else { 3 });
        let delete_buttons: Vec<_> = row_actions
            .iter()
            .filter(|rect| (rect.right() - row_actions[0].right()).abs() < 1.)
            .collect();
        assert_eq!(
            delete_buttons.len(),
            3,
            "delete buttons must form one column"
        );
        for rect in &row_actions {
            assert!(
                inner.contains_rect(*rect),
                "row action {rect:?} must stay inside the list {inner:?}"
            );
            assert!(
                rect.right() > titles[0].1.right(),
                "row action {rect:?} must sit right of the text column"
            );
        }
        // Each action must be horizontally beside the row it belongs to.
        let mut row_tops: Vec<f32> = titles.iter().map(|t| t.1.top()).collect();
        row_tops.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for rect in &row_actions {
            assert!(
                row_tops.iter().any(|top| (rect.top() - top).abs() < 24.),
                "row action {rect:?} is not level with any row title {row_tops:?}"
            );
        }
        let mut previous = f32::NEG_INFINITY;
        for (label, rect) in &titles {
            // Rows must stack downwards, never jump back above the previous row.
            assert!(
                rect.top() > previous,
                "{label} at {rect:?} overlaps the previous row"
            );
            previous = rect.top();
            assert!(
                rect.right() <= inner.right(),
                "{label} at {rect:?} escapes the panel {inner:?}"
            );
        }
        // Text must start right next to the icon column, not far below it.
        let first = titles[0].1;
        assert!(
            first.top() - inner.top() < 200.,
            "first row is pushed down: {first:?} inside {inner:?}"
        );
    }
    #[test]
    fn resourcepack_category_stays_in_mods_panel() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut app = Launcher::new(
            &ctx,
            root.path().into(),
            Config::default(),
            false,
            String::new(),
        );
        app.page = Page::Mods;
        app.resource_tab = true;
        for i in 0..3 {
            frame(
                &mut app,
                &ctx,
                Vec2::new(720., 560.),
                i as f64 * 0.1,
                vec![],
            );
        }
        assert!(app.page == Page::Mods);
        assert_eq!(app.panel_id(), layout::PanelId::Mods);
        let (inner, used) = layout::content_bounds(&ctx, layout::PanelId::Mods).unwrap();
        assert!(used.right() <= inner.right() + 1.);
        assert!(used.bottom() <= inner.bottom() + 1.);
    }
    #[test]
    fn two_pages_render_side_by_side() {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = Launcher::new(
            &ctx,
            root.path().into(),
            Config::default(),
            false,
            String::new(),
        );
        app.toggle_page(Page::Settings);
        app.toggle_page(Page::Versions);
        for i in 0..4 {
            frame(
                &mut app,
                &ctx,
                Vec2::new(1600., 900.),
                i as f64 * 0.1,
                vec![],
            );
        }
        assert!(layout::content_bounds(&ctx, layout::PanelId::Settings).is_some());
        assert!(layout::content_bounds(&ctx, layout::PanelId::Versions).is_some());
    }
    #[test]
    fn two_pages_can_be_open_and_each_toggle_closes_one() {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = Launcher::new(
            &ctx,
            root.path().into(),
            Config::default(),
            false,
            String::new(),
        );
        app.toggle_page(Page::Settings);
        app.toggle_page(Page::Versions);
        assert_eq!(app.page, Page::Versions);
        assert_eq!(app.secondary_page, Some(Page::Settings));
        app.toggle_page(Page::Settings);
        assert_eq!(app.secondary_page, None);
        assert_eq!(app.page, Page::Versions);
        app.close_pages();
        assert_eq!(app.page, Page::Home);
    }
    #[test]
    fn clicking_free_workspace_closes_open_pages() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut app = Launcher::new(
            &ctx,
            root.path().into(),
            Config::default(),
            false,
            String::new(),
        );
        app.page = Page::Settings;
        let size = Vec2::new(1206., 760.);
        frame(&mut app, &ctx, size, 0., vec![]);
        let point = egui::pos2(32., 100.);
        let button = |pressed| egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &mut app,
            &ctx,
            size,
            0.1,
            vec![egui::Event::PointerMoved(point)],
        );
        frame(&mut app, &ctx, size, 0.2, vec![button(true)]);
        frame(&mut app, &ctx, size, 0.3, vec![button(false)]);
        frame(&mut app, &ctx, size, 0.4, vec![]);
        assert_eq!(app.page, Page::Home);
    }
    #[test]
    fn clicking_a_panel_does_not_close_it() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut app = Launcher::new(
            &ctx,
            root.path().into(),
            Config::default(),
            false,
            String::new(),
        );
        app.page = Page::Settings;
        let size = Vec2::new(1206., 760.);
        frame(&mut app, &ctx, size, 0., vec![]);
        let screen = egui::Rect::from_min_max(egui::pos2(0., 38.), egui::pos2(size.x, size.y));
        let launcher =
            layout::Geometry::new(screen, &app.config.panels, Some(layout::PanelId::Settings))
                .launcher;
        let point = launcher.center();
        let button = |pressed| egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &mut app,
            &ctx,
            size,
            0.1,
            vec![egui::Event::PointerMoved(point)],
        );
        frame(&mut app, &ctx, size, 0.2, vec![button(true)]);
        frame(&mut app, &ctx, size, 0.3, vec![button(false)]);
        frame(&mut app, &ctx, size, 0.4, vec![]);
        assert_eq!(app.page, Page::Settings);
    }
    #[test]
    fn repeated_navigation_button_closes_its_page() {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = Launcher::new(
            &ctx,
            root.path().into(),
            Config::default(),
            false,
            String::new(),
        );
        for page in [
            Page::Versions,
            Page::Mods,
            Page::Accounts,
            Page::Settings,
            Page::About,
            Page::Help,
        ] {
            app.toggle_page(page);
            assert!(app.page == page);
            app.toggle_page(page);
            assert!(app.page == Page::Home);
        }
    }
    fn frame(
        app: &mut Launcher,
        ctx: &egui::Context,
        size: Vec2,
        time: f64,
        events: Vec<egui::Event>,
    ) -> egui::Rect {
        let mut rect = egui::Rect::NOTHING;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                rect = app.render(ctx);
            },
        );
        rect
    }
    fn drag(
        app: &mut Launcher,
        ctx: &egui::Context,
        size: Vec2,
        start: egui::Pos2,
        delta: Vec2,
        time: f64,
    ) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(app, ctx, size, time, vec![egui::Event::PointerMoved(start)]);
        frame(app, ctx, size, time + 0.1, vec![button(start, true)]);
        frame(
            app,
            ctx,
            size,
            time + 0.2,
            vec![egui::Event::PointerMoved(start + delta)],
        );
        frame(
            app,
            ctx,
            size,
            time + 0.3,
            vec![button(start + delta, false)],
        );
        frame(app, ctx, size, time + 0.4, vec![]);
    }
    #[test]
    fn edges_resize_inline_and_persist_each_page_independently() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.profiles[0].loader = "fabric".into();
        std::fs::create_dir_all(
            root.path()
                .join("instances")
                .join(&config.profiles[0].directory),
        )
        .unwrap();
        let mut app = Launcher::new(&ctx, root.path().into(), config, false, String::new());
        app.page = Page::Mods;
        app.searched = true;
        app.categories.insert(Source::Modrinth, vec![]);
        let screen = Vec2::new(1280., 800.);
        for i in 0..4 {
            frame(&mut app, &ctx, screen, i as f64 * 0.1, vec![]);
        }
        let geometry = layout::Geometry::new(
            egui::Rect::from_min_max(egui::pos2(0., 38.), screen.to_pos2()),
            &app.config.panels,
            Some(layout::PanelId::Mods),
        );
        let right = geometry.detail.unwrap();
        drag(
            &mut app,
            &ctx,
            screen,
            egui::pos2(right.right() - 2., right.center().y),
            Vec2::new(32., 0.),
            1.,
        );
        assert_eq!(app.config.panels.get(layout::PanelId::Mods).x, 704.);
        let geometry = layout::Geometry::new(
            egui::Rect::from_min_max(egui::pos2(0., 38.), screen.to_pos2()),
            &app.config.panels,
            Some(layout::PanelId::Mods),
        );
        let right = geometry.detail.unwrap();
        drag(
            &mut app,
            &ctx,
            screen,
            egui::pos2(right.center().x, right.bottom() - 2.),
            Vec2::new(0., 16.),
            2.,
        );
        assert_eq!(
            app.config.panels.get(layout::PanelId::Mods),
            Vec2::new(704., 512.)
        );
        let restored = Config::load(root.path()).unwrap();
        assert_eq!(
            restored.panels.get(layout::PanelId::Mods),
            Vec2::new(704., 512.)
        );
        assert_eq!(
            restored.panels.get(layout::PanelId::Versions),
            layout::PanelId::Versions.default_size()
        );
        app.page = Page::Versions;
        frame(&mut app, &ctx, screen, 3., vec![]);
        app.page = Page::Mods;
        frame(&mut app, &ctx, Vec2::new(720., 560.), 3.1, vec![]);
        assert_eq!(
            app.config.panels.get(layout::PanelId::Mods),
            Vec2::new(704., 512.)
        );
        frame(&mut app, &ctx, screen, 3.2, vec![]);
        let restored_geometry = layout::Geometry::new(
            egui::Rect::from_min_max(egui::pos2(0., 38.), screen.to_pos2()),
            &app.config.panels,
            Some(layout::PanelId::Mods),
        );
        assert_eq!(
            restored_geometry.detail.unwrap().size(),
            Vec2::new(704., 512.)
        );
        let right = restored_geometry.detail.unwrap();
        drag(
            &mut app,
            &ctx,
            screen,
            egui::pos2(right.center().x, right.top() + 2.),
            Vec2::new(0., -16.),
            4.,
        );
        assert_eq!(
            app.config.panels.get(layout::PanelId::Mods),
            Vec2::new(704., 544.)
        );
        let geometry = layout::Geometry::new(
            egui::Rect::from_min_max(egui::pos2(0., 38.), screen.to_pos2()),
            &app.config.panels,
            Some(layout::PanelId::Mods),
        );
        let right = geometry.detail.unwrap();
        drag(
            &mut app,
            &ctx,
            screen,
            right.max - Vec2::splat(5.),
            Vec2::new(16., 16.),
            5.,
        );
        assert_eq!(
            app.config.panels.get(layout::PanelId::Mods),
            Vec2::new(736., 576.)
        );
    }
    #[test]
    fn mod_filters_and_footer_stay_fixed_while_list_scrolls_and_grows() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.profiles[0].loader = "fabric".into();
        std::fs::create_dir_all(
            root.path()
                .join("instances")
                .join(&config.profiles[0].directory),
        )
        .unwrap();
        let mut app = Launcher::new(&ctx, root.path().into(), config, false, String::new());
        app.page = Page::Mods;
        app.searched = true;
        app.categories.insert(Source::Modrinth, vec![]);
        app.projects=(0..20).map(|i|Project{id:format!("test-{i}"),name:format!("Example mod {i}"),description:"A long description of a mod, wrapping naturally without pushing the actions or filters out of the panel.".into(),author:"Example author".into(),downloads:1000,website:String::new(),source:Source::Modrinth,icon_url:String::new(),categories:vec!["optimization".into()]}).collect();
        let size = Vec2::new(1280., 800.);
        for i in 0..4 {
            frame(&mut app, &ctx, size, i as f64 * 0.1, vec![]);
        }
        let bounds = |key| {
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(key)))
                .unwrap()
        };
        let search = bounds("mods_search_bounds");
        let footer = bounds("mods_footer_bounds");
        let list = bounds("mods_list_bounds");
        frame(
            &mut app,
            &ctx,
            size,
            1.,
            vec![
                egui::Event::PointerMoved(list.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0., -200.),
                    modifiers: Default::default(),
                },
            ],
        );
        assert_eq!(bounds("mods_search_bounds"), search);
        assert_eq!(bounds("mods_footer_bounds"), footer);
        assert!(list.bottom() <= footer.top() - layout::GRID + 1.);
        app.config
            .panels
            .set(layout::PanelId::Mods, Vec2::new(640., 608.));
        for i in 0..4 {
            frame(&mut app, &ctx, size, 2. + i as f64 * 0.1, vec![]);
        }
        let expanded = bounds("mods_list_bounds");
        assert!((expanded.height() - list.height() - 128.).abs() <= 1.);
    }
    #[test]
    fn builds_can_be_renamed_and_deleted() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let config = Config::default();
        std::fs::create_dir_all(root.path().join("instances").join("1.21.1 · Vanilla")).unwrap();
        let mut app = Launcher::new(&ctx, root.path().into(), config, false, String::new());
        app.select_instance("1.21.1", "forge", "Старая сборка", false);
        let index = app.config.selected;
        assert!(app.config.profiles[index].directory == "Старая сборка");
        let old_folder = app.config.instance_dir(root.path());
        assert!(old_folder.is_dir());

        app.edit_build_name = "Новая сборка".into();
        app.rename_build(index);
        assert_eq!(app.config.profiles[index].name, "Новая сборка");
        assert_eq!(app.config.profiles[index].directory, "Новая сборка");
        let new_folder = app.config.instance_dir(root.path());
        assert!(new_folder.is_dir());
        assert!(!old_folder.exists());

        // Удаление без папки: запись исчезает, файлы остаются.
        app.delete_build_folder = false;
        app.delete_build_now(index);
        assert_eq!(app.config.profiles.len(), 1);
        assert!(new_folder.is_dir());

        // Создаём ещё одну и удаляем вместе с папкой.
        app.select_instance("1.20.1", "fabric", "Удаляемая", false);
        let index = app.config.selected;
        let folder = app.config.instance_dir(root.path());
        assert!(folder.is_dir());
        app.delete_build_folder = true;
        app.delete_build_now(index);
        assert!(!folder.exists());
        assert_eq!(app.config.profiles.len(), 1);
    }

    #[test]
    fn builds_tab_prunes_missing_folders_and_renders_actions() {
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        let config = Config::default();
        let mut app = Launcher::new(&ctx, root.path().into(), config, false, String::new());
        // Папка активной сборки существует.
        std::fs::create_dir_all(app.config.instance_dir(root.path())).unwrap();
        // Вторая сборка создаётся вместе с папкой, затем папка удаляется снаружи.
        app.select_instance("1.21.1", "fabric", "Временная", false);
        let instances = app.config.game_root(root.path()).join("instances");
        let removed = app.config.profiles[1].directory.clone();
        std::fs::remove_dir_all(instances.join(&removed)).unwrap();
        app.page = Page::Versions;
        app.version_tab = VersionTab::Builds;
        for i in 0..4 {
            frame(
                &mut app,
                &ctx,
                Vec2::new(1100., 720.),
                i as f64 * 0.1,
                vec![],
            );
        }
        assert_eq!(app.config.profiles.len(), 1);
        assert!(!instances.join(removed).exists());
        // Состояния редактирования и подтверждения удаления не должны паниковать.
        app.editing_build = Some(0);
        app.edit_build_name = "Новое имя".into();
        frame(&mut app, &ctx, Vec2::new(1100., 720.), 1., vec![]);
        app.editing_build = None;
        app.delete_build = Some(0);
        frame(&mut app, &ctx, Vec2::new(1100., 720.), 1.2, vec![]);
    }

    #[test]
    fn inline_panels_fit_small_and_normal_windows() {
        for size in [Vec2::new(720., 560.), Vec2::new(1100., 720.)] {
            let ctx = egui::Context::default();
            let root = tempfile::tempdir().unwrap();
            let mut config = Config::default();
            config.profiles[0].loader = "fabric".into();
            let mut app = Launcher::new(&ctx, root.path().into(), config, false, String::new());
            app.searched = true;
            app.categories.insert(Source::Modrinth, vec![]);
            app.projects.push(Project {
                id: "fixture".into(),
                name: "Example Mod".into(),
                description:
                    "A useful mod description that wraps onto multiple lines on narrow windows."
                        .into(),
                author: "Example Author".into(),
                downloads: 12345678,
                website: String::new(),
                source: Source::Modrinth,
                icon_url: String::new(),
                categories: vec!["optimization".into(), "utility".into()],
            });
            app.version_list.push(versions::MinecraftVersion {
                id: "1.21.1".into(),
                kind: versions::VersionKind::Release,
                release_time: String::new(),
                url: String::new(),
                sha1: String::new(),
            });
            for page in [
                Page::Home,
                Page::Versions,
                Page::Mods,
                Page::Accounts,
                Page::Settings,
                Page::Help,
            ] {
                app.page = page;
                let mut workspace = egui::Rect::NOTHING;
                for frame in 0..4 {
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        time: Some(frame as f64 * 0.1),
                        ..Default::default()
                    };
                    let _ = ctx.run(input, |ctx| {
                        workspace = app.render(ctx);
                    });
                }
                assert!(
                    workspace.width() <= size.x + 1.,
                    "panel width {} exceeds {}",
                    workspace.width(),
                    size.x
                );
                assert!(
                    workspace.height() <= size.y + 1.,
                    "panel height {} exceeds {}",
                    workspace.height(),
                    size.y
                );
                for id in [layout::PanelId::Launcher, app.panel_id()] {
                    let (inner, used) = layout::content_bounds(&ctx, id).unwrap();
                    assert!(
                        used.right() <= inner.right() + 1.,
                        "{id:?} content overflow: {used:?} / {inner:?}"
                    );
                    assert!(
                        used.bottom() <= inner.bottom() + 1.,
                        "{id:?} content overflow: {used:?} / {inner:?}"
                    );
                }
            }
        }
    }
}
