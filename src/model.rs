use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Source {
    #[default]
    Modrinth,
    CurseForge,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Modrinth => "Modrinth",
                Self::CurseForge => "CurseForge",
            }
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildIcon {
    Block,
    Mods,
    Landscape,
    Home,
    Folder,
    Star,
    Shield,
    Swords,
    Pickaxe,
    Compass,
    Lightning,
    None,
    Custom,
    #[default]
    #[serde(other)]
    Box,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub icon: BuildIcon,
    pub game_version: String,
    pub loader: String,
    /// Имя папки инстанса; совпадает с названием сборки, но очищено для файловой системы.
    #[serde(default)]
    pub directory: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java: Option<crate::java::JavaSettings>,
}

/// Безопасное имя папки из названия сборки. Никаких разделителей и зарезервированных имён Windows.
pub fn directory_from_name(name: &str) -> String {
    let mut cleaned = String::new();
    for c in name.chars() {
        if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            continue;
        }
        cleaned.push(c);
    }
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).trim();
    let mut result = if cleaned.is_empty() {
        "instance".to_owned()
    } else {
        cleaned.to_owned()
    };
    if result
        .to_ascii_uppercase()
        .split(['.', ' '])
        .next()
        .is_some_and(|stem| {
            matches!(stem, "CON" | "PRN" | "AUX" | "NUL")
                || (stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.ends_with(['1', '2', '3', '4', '5', '6', '7', '8', '9']))
        })
    {
        result.push_str("_instance");
    }
    result
        .chars()
        .take(80)
        .collect::<String>()
        .trim()
        .to_owned()
}

pub fn is_safe_directory(directory: &str) -> bool {
    !directory.is_empty()
        && directory == directory.trim()
        && !directory.contains(['/', '\\', ':'])
        && directory != "."
        && directory != ".."
        && !directory.ends_with(['.', ' '])
}

pub fn unique_directory<'a>(name: &str, used: impl Iterator<Item = &'a str>) -> String {
    let base = directory_from_name(name);
    let used: Vec<&str> = used.collect();
    let taken = |candidate: &str| {
        used.iter()
            .any(|other| other.eq_ignore_ascii_case(candidate))
    };
    if !taken(&base) {
        return base;
    }
    for suffix in 2..10_000 {
        let candidate = format!("{base} ({suffix})");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", uuid::Uuid::new_v4().simple())
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnGameStart {
    #[default]
    KeepOpen,
    Minimize,
    Close,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HorizonSettings {
    pub selected: bool,
    pub launcher: PathBuf,
    pub instance: String,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryGroup {
    #[default]
    Catalog,
    MyBuilds,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryTabs {
    pub packs: LibraryGroup,
    pub exchange: LibraryGroup,
}

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub profiles: Vec<Profile>,
    pub selected: usize,
    #[serde(default)]
    pub accounts: Vec<crate::auth::Account>,
    #[serde(default)]
    pub selected_account: usize,
    #[serde(default)]
    pub microsoft_client_id: String,
    #[serde(default)]
    pub appearance: crate::appearance::Appearance,
    #[serde(default)]
    pub panels: crate::layout::PanelSizes,
    #[serde(default)]
    pub library_tabs: LibraryTabs,
    #[serde(default)]
    pub game_directory: String,
    #[serde(default)]
    pub java: crate::java::JavaSettings,
    #[serde(default = "default_version_filters")]
    pub version_filters: [bool; 4],
    #[serde(default)]
    pub favorite_versions: std::collections::BTreeSet<String>,
    #[serde(default)]
    pub assistant: crate::assistant::Settings,
    #[serde(default)]
    pub multi_session: MultiSession,
    #[serde(default)]
    pub curseforge_manual: bool,
    #[serde(default)]
    pub on_game_start: OnGameStart,
    #[serde(default)]
    pub horizon: HorizonSettings,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MultiSession {
    pub enabled: bool,
    pub limit: usize,
    pub memory_mb: u32,
    pub overlay: bool,
    pub native_overlay: bool,
}
impl Default for MultiSession {
    fn default() -> Self {
        Self {
            enabled: false,
            limit: 2,
            memory_mb: 2048,
            overlay: false,
            native_overlay: false,
        }
    }
}
impl MultiSession {
    pub fn java(&self, base: &crate::java::JavaSettings) -> crate::java::JavaSettings {
        let mut settings = base.clone();
        if self.enabled {
            settings.max_memory = self.memory_mb.clamp(1024, 16384);
            settings.min_memory = 256;
            settings.window_width = 854;
            settings.window_height = 480;
            settings.fullscreen = false;
        }
        settings
    }
}

fn default_version_filters() -> [bool; 4] {
    [true; 4]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            profiles: vec![Profile {
                java: None,
                icon: Default::default(),
                name: "1.21.1 · Vanilla".into(),
                game_version: "1.21.1".into(),
                loader: "vanilla".into(),
                directory: "1.21.1 · Vanilla".into(),
            }],
            selected: 0,
            accounts: vec![],
            selected_account: 0,
            microsoft_client_id: String::new(),
            appearance: Default::default(),
            panels: Default::default(),
            library_tabs: LibraryTabs::default(),
            game_directory: String::new(),
            java: Default::default(),
            version_filters: default_version_filters(),
            favorite_versions: Default::default(),
            assistant: crate::assistant::Settings {
                use_shared: true,
                ..Default::default()
            },
            multi_session: Default::default(),
            curseforge_manual: false,
            on_game_start: OnGameStart::default(),
            horizon: HorizonSettings::default(),
        }
    }
}

impl Config {
    /// Reuse an instance for the same version+loader; never change existing instances.
    pub fn select_instance(
        &mut self,
        version: &str,
        loader: &str,
        name: &str,
        force_new: bool,
    ) -> Result<()> {
        if version.is_empty()
            || !matches!(
                loader,
                "vanilla" | "fabric" | "forge" | "neoforge" | "quilt"
            )
        {
            bail!("Некорректная версия или загрузчик");
        }
        if !force_new {
            if let Some(index) = self
                .profiles
                .iter()
                .position(|p| p.game_version == version && p.loader == loader)
            {
                self.selected = index;
                return Ok(());
            }
        }
        let name = if name.trim().is_empty() {
            format!("{version} · {loader}")
        } else {
            name.trim().to_owned()
        };
        let directory = unique_directory(&name, self.profiles.iter().map(|p| p.directory.as_str()));
        self.profiles.push(Profile {
            java: None,
            icon: Default::default(),
            name,
            game_version: version.into(),
            loader: loader.into(),
            directory,
        });
        self.selected = self.profiles.len() - 1;
        Ok(())
    }

    /// Старые конфиги не содержали каталог: сохраняем прежний `profile-N`, затем
    /// безопасно переименовываем папку под название сборки (только если целевой папки нет).
    /// Возвращает (изменилось ли что-то, предупреждения для пользователя).
    pub fn migrate_directories(&mut self, root: &Path) -> (bool, Vec<String>) {
        let mut changed = false;
        let mut warnings = vec![];
        for index in 0..self.profiles.len() {
            if self.profiles[index].directory.is_empty() {
                self.profiles[index].directory = format!("profile-{}", index + 1);
                changed = true;
            }
            let target = self.desired_directory(index);
            let synced = self.sync_directory(index, root);
            changed |= synced;
            if !synced && !self.profiles[index].directory.eq_ignore_ascii_case(&target) {
                warnings.push(format!(
                    "Не удалось переименовать папку «{}» в «{}». Закройте её в Проводнике (или проверьте антивирус) и перезапустите лаунчер.",
                    self.profiles[index].directory, target
                ));
            }
        }
        (changed, warnings)
    }

    fn desired_directory(&self, index: usize) -> String {
        unique_directory(
            &self.profiles[index].name,
            self.profiles
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, p)| p.directory.as_str()),
        )
    }

    /// Приводит папку инстанса в соответствие названию сборки. Ничего не удаляет.
    pub fn sync_directory(&mut self, index: usize, root: &Path) -> bool {
        if index >= self.profiles.len() || !is_safe_directory(&self.profiles[index].directory) {
            return false;
        }
        let target = self.desired_directory(index);
        if self.profiles[index].directory.eq_ignore_ascii_case(&target) {
            return false;
        }
        let instances = self.game_root(root).join("instances");
        let old_path = instances.join(&self.profiles[index].directory);
        let new_path = instances.join(&target);
        if new_path.exists() {
            return false;
        }
        if old_path.is_dir() {
            // Explorer/антивирус могут кратковременно держать папку — несколько попыток.
            let mut renamed = false;
            for _ in 0..3 {
                if std::fs::rename(&old_path, &new_path).is_ok() {
                    renamed = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
            if !renamed {
                return false;
            }
        }
        // Если старой папки нет (новый профиль или сменённый путь установки),
        // просто принимаем новое имя: ничего не перемещаем и не создаём.
        self.profiles[index].directory = target;
        true
    }

    /// Убирает сборки, чьи папки были удалены вне лаунчера. Возвращает число удалённых.
    pub fn prune_missing(&mut self, root: &Path) -> usize {
        let instances = self.game_root(root).join("instances");
        let selected_dir = self
            .profiles
            .get(self.selected)
            .map(|profile| profile.directory.clone());
        let before = self.profiles.len();
        self.profiles
            .retain(|profile| instances.join(&profile.directory).is_dir());
        let removed = before - self.profiles.len();
        if removed > 0 {
            self.selected = selected_dir
                .and_then(|directory| {
                    self.profiles
                        .iter()
                        .position(|profile| profile.directory == directory)
                })
                .unwrap_or(0);
        }
        self.selected = self.selected.min(self.profiles.len().saturating_sub(1));
        removed
    }

    /// Если после очистки не осталось ни одной сборки, возвращает одну стандартную.
    pub fn ensure_default(&mut self) {
        if self.profiles.is_empty() {
            let mut fallback = Config::default();
            self.profiles.push(fallback.profiles.remove(0));
            self.selected = 0;
        }
    }

    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("config.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut config: Self = serde_json::from_slice(&std::fs::read(path)?)
            .context("Не удалось прочитать config.json; файл не перезаписан")?;
        if config.profiles.is_empty() {
            bail!("В config.json отсутствуют профили");
        }
        config.selected = config.selected.min(config.profiles.len() - 1);
        config.selected_account = config
            .selected_account
            .min(config.accounts.len().saturating_sub(1));
        for (index, profile) in config.profiles.iter_mut().enumerate() {
            if profile.directory.is_empty() {
                profile.directory = format!("profile-{}", index + 1);
            }
        }
        Ok(config)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        std::fs::create_dir_all(root)?;
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.as_file().sync_all()?;
        file.persist(root.join("config.json"))?;
        Ok(())
    }

    pub fn instance_dir(&self, root: &Path) -> PathBuf {
        let directory = self
            .profiles
            .get(self.selected)
            .map(|p| p.directory.as_str())
            .filter(|d| is_safe_directory(d))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("profile-{}", self.selected + 1));
        self.game_root(root).join("instances").join(directory)
    }
    pub fn game_root(&self, root: &Path) -> PathBuf {
        if self.game_directory.trim().is_empty() {
            root.to_path_buf()
        } else {
            PathBuf::from(self.game_directory.trim())
        }
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub downloads: u64,
    pub website: String,
    pub source: Source,
    pub icon_url: String,
    pub categories: Vec<String>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum SortOrder {
    #[default]
    Downloads,
    Relevance,
    Updated,
    Newest,
}
impl SortOrder {
    pub const ALL: [Self; 4] = [
        Self::Downloads,
        Self::Relevance,
        Self::Updated,
        Self::Newest,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Downloads => "Самые скачиваемые",
            Self::Relevance => "По релевантности",
            Self::Updated => "Недавно обновлённые",
            Self::Newest => "Самые новые",
        }
    }
    pub fn modrinth(self) -> &'static str {
        match self {
            Self::Downloads => "downloads",
            Self::Relevance => "relevance",
            Self::Updated => "updated",
            Self::Newest => "newest",
        }
    }
    pub fn curseforge(self) -> &'static str {
        match self {
            Self::Downloads => "6",
            Self::Relevance => "1",
            Self::Updated => "3",
            Self::Newest => "11",
        }
    }
}
#[derive(Clone, Default)]
pub struct SearchOptions {
    pub sort: SortOrder,
    pub category: Option<String>,
}
#[derive(Clone)]
pub struct Category {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct InstalledMod {
    pub source: Source,
    pub project_id: String,
    pub version_id: String,
    pub filename: String,
    pub name: String,
    #[serde(default)]
    pub icon_url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub categories: Vec<String>,
}

pub fn safe_filename(name: &str) -> Result<&str> {
    if name.is_empty()
        || name.contains(['/', '\\', ':'])
        || name.starts_with('.')
        || name.ends_with(['.', ' '])
        || !name.ends_with(".jar")
        || name
            .chars()
            .any(|c| c.is_control() || "<>\"|?*".contains(c))
    {
        bail!("Небезопасное имя файла: {name}");
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(['1', '2', '3', '4', '5', '6', '7', '8', '9']))
    {
        bail!("Зарезервированное имя файла");
    }
    Ok(name)
}

/// Resolve the physical file without re-enabling a deliberately disabled mod.
pub fn mod_path(instance: &Path, filename: &str) -> Result<PathBuf> {
    safe_filename(filename)?;
    let active = instance.join("mods").join(filename);
    let disabled = instance.join("mods").join(format!("{filename}.disabled"));
    Ok(if !active.exists() && disabled.is_file() {
        disabled
    } else {
        active
    })
}

pub fn installed(instance: &Path) -> Result<Vec<InstalledMod>> {
    let path = instance.join("mods.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let entries: Vec<InstalledMod> = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(entries
        .into_iter()
        .filter(|item| mod_path(instance, &item.filename).is_ok_and(|path| path.is_file()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_companion_setting_does_not_consent_to_native_injection() {
        let setting: MultiSession =
            serde_json::from_str(r#"{"enabled":true,"overlay":true}"#).unwrap();
        assert!(setting.overlay);
        assert!(!setting.native_overlay);
    }
    #[test]
    fn multi_session_limits_and_settings_persist() {
        let mut config = Config::default();
        config.multi_session.enabled = true;
        config.multi_session.memory_mb = 3072;
        config.curseforge_manual = true;
        let encoded = serde_json::to_vec(&config).unwrap();
        let decoded: Config = serde_json::from_slice(&encoded).unwrap();
        assert!(decoded.multi_session.enabled && decoded.curseforge_manual);
        let settings = decoded.multi_session.java(&decoded.java);
        assert_eq!(settings.max_memory, 3072);
        assert_eq!(settings.min_memory, 256);
        assert!(!settings.fullscreen);
        assert_eq!(decoded.java.max_memory, 4096);
    }
    #[test]
    fn rejects_unsafe_names() {
        for name in [
            "../evil.jar",
            "..\\evil.jar",
            "C:evil.jar",
            "CON.jar",
            "a.jar:bad",
            ".jar",
            "x.exe",
            "a?.jar",
        ] {
            assert!(safe_filename(name).is_err(), "{name}");
        }
        assert!(safe_filename("sodium-0.6.0+mc1.21.1.jar").is_ok());
    }
    #[test]
    fn config_round_trip() {
        let root = tempfile::tempdir().unwrap();
        Config::default().save(root.path()).unwrap();
        assert_eq!(
            Config::load(root.path()).unwrap().profiles[0].loader,
            "vanilla"
        );
        let mut config = Config::load(root.path()).unwrap();
        config.profiles[0].name = "Изменённый профиль".into();
        config.save(root.path()).unwrap();
        assert_eq!(
            Config::load(root.path()).unwrap().profiles[0].name,
            "Изменённый профиль"
        );
    }

    #[test]
    fn version_filters_survive_restart() {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            version_filters: [false, true, false, true],
            favorite_versions: ["1.21.1".into(), "26.3".into()].into(),
            ..Default::default()
        };
        config.save(root.path()).unwrap();
        let loaded = Config::load(root.path()).unwrap();
        assert_eq!(loaded.version_filters, [false, true, false, true]);
        assert_eq!(loaded.favorite_versions, config.favorite_versions);
    }
    #[test]
    fn legacy_config_accepts_new_microsoft_setting() {
        let config: Config = serde_json::from_str(r#"{"profiles":[{"name":"Old profile","game_version":"1.20.1","loader":"forge"}],"selected":0}"#).unwrap();
        assert!(config.microsoft_client_id.is_empty());
        assert_eq!(config.profiles[0].name, "Old profile");
        assert_eq!(config.profiles[0].loader, "forge");
        assert!(config.appearance.background.is_empty());
    }
    #[test]
    fn old_mod_manifests_and_sort_mapping() {
        let old:InstalledMod=serde_json::from_str(r#"{"source":"Modrinth","project_id":"test","version_id":"1","filename":"mod.jar","name":"Test"}"#).unwrap();
        assert!(old.icon_url.is_empty());
        assert!(old.categories.is_empty());
        assert_eq!(SortOrder::Downloads.modrinth(), "downloads");
        assert_eq!(SortOrder::Downloads.curseforge(), "6");
        assert_eq!(SortOrder::Updated.modrinth(), "updated");
        assert_eq!(SortOrder::Newest.curseforge(), "11");
    }
    #[test]
    fn corrupt_config_is_not_replaced() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("config.json"), "broken").unwrap();
        assert!(Config::load(root.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("config.json")).unwrap(),
            "broken"
        );
    }

    #[test]
    fn instances_are_isolated_per_version_and_loader() {
        let root = Path::new("data");
        let mut config = Config::default();
        let vanilla = config.instance_dir(root);
        config
            .select_instance("1.21.1", "fabric", "Fabric", false)
            .unwrap();
        let fabric = config.instance_dir(root);
        assert_ne!(vanilla, fabric);
        config
            .select_instance("b1.7.3", "vanilla", "Beta", false)
            .unwrap();
        assert_ne!(config.instance_dir(root), fabric);
        assert_ne!(config.instance_dir(root), vanilla);
        config
            .select_instance("1.21.1", "fabric", "Ignored", false)
            .unwrap();
        assert_eq!(config.instance_dir(root), fabric);
        assert_eq!(config.profiles.len(), 3);
        config
            .select_instance("1.21.1", "fabric", "Another modpack", true)
            .unwrap();
        assert_ne!(config.instance_dir(root), fabric);
        assert_eq!(config.profiles[0].loader, "vanilla");
        assert_eq!(config.profiles[2].game_version, "b1.7.3");
        // Папки называются как сборки в лаунчере.
        assert_eq!(config.profiles[0].directory, "1.21.1 · Vanilla");
        assert_eq!(config.profiles[1].directory, "Fabric");
        assert_eq!(config.profiles[2].directory, "Beta");
        assert_eq!(config.profiles[3].directory, "Another modpack");
    }

    #[test]
    fn folder_names_are_sanitized_unique_and_migrated() {
        assert_eq!(directory_from_name("Sborka"), "Sborka");
        assert_eq!(directory_from_name("My/Mod: Pack?"), "MyMod Pack");
        assert_eq!(directory_from_name("CON"), "CON_instance");
        assert_eq!(directory_from_name("..."), "instance");
        assert_eq!(
            unique_directory("Sborka", ["sborka"].into_iter()),
            "Sborka (2)"
        );
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("instances/profile-1")).unwrap();
        std::fs::write(root.path().join("instances/profile-1/marker"), b"keep").unwrap();
        let mut config = Config::default();
        config.profiles[0].name = "Sborka".into();
        config.profiles[0].directory = "profile-1".into();
        assert!(config.migrate_directories(root.path()).0);
        assert_eq!(config.profiles[0].directory, "Sborka");
        assert!(root.path().join("instances/Sborka/marker").is_file());
        assert!(!root.path().join("instances/profile-1").exists());
        // Повторный вызов ничего не меняет.
        assert!(!config.migrate_directories(root.path()).0);
        assert_eq!(
            config.instance_dir(root.path()),
            root.path().join("instances/Sborka")
        );
    }

    #[test]
    fn missing_instance_folders_are_pruned_and_selection_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        let first = config.profiles[0].directory.clone();
        std::fs::create_dir_all(root.path().join("instances").join(&first)).unwrap();
        config
            .select_instance("1.21.1", "fabric", "Вторая", false)
            .unwrap();
        let second = config.profiles[1].directory.clone();
        std::fs::create_dir_all(root.path().join("instances").join(&second)).unwrap();
        config
            .select_instance("1.20.1", "forge", "Третья", false)
            .unwrap();
        let third = config.profiles[2].directory.clone();
        std::fs::create_dir_all(root.path().join("instances").join(&third)).unwrap();
        config.selected = 2;
        std::fs::remove_dir_all(root.path().join("instances").join(&third)).unwrap();
        assert_eq!(config.prune_missing(root.path()), 1);
        assert_eq!(config.profiles.len(), 2);
        assert_eq!(config.selected, 0);
        std::fs::remove_dir_all(root.path().join("instances").join(&first)).unwrap();
        std::fs::remove_dir_all(root.path().join("instances").join(&second)).unwrap();
        assert_eq!(config.prune_missing(root.path()), 2);
        assert!(config.profiles.is_empty());
        config.ensure_default();
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(
            config.instance_dir(root.path()).file_name().unwrap(),
            std::ffi::OsStr::new(&config.profiles[0].directory)
        );
    }

    #[test]
    fn migration_respects_custom_game_directory() {
        let data = tempfile::tempdir().unwrap();
        let game = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(game.path().join("instances/profile-2")).unwrap();
        std::fs::write(game.path().join("instances/profile-2/marker"), b"city").unwrap();
        let mut config = Config {
            game_directory: game.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        config.profiles[0].name = "Город".into();
        config.profiles[0].directory = "profile-2".into();
        assert!(config.migrate_directories(data.path()).0);
        assert_eq!(config.profiles[0].directory, "Город");
        assert!(game.path().join("instances/Город/marker").is_file());
        assert_eq!(
            config.instance_dir(data.path()),
            game.path().join("instances/Город")
        );
    }
}
