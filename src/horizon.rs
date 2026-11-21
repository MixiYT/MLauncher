//! Install and launch Horizon without invoking its launcher.
use crate::{
    downloads::{self, Checksum, Download},
    model::HorizonSettings,
};
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

const CLIENT: &str = if cfg!(windows) {
    "horizon-client.exe"
} else {
    "horizon-client"
};

fn json(path: &Path) -> Option<Value> {
    let reader = File::open(path).ok()?.take(4 * 1024 * 1024);
    serde_json::from_reader(reader).ok()
}

fn leaf(value: &str) -> bool {
    let mut components = Path::new(value).components();
    !value.contains(['/', '\\', ':'])
        && matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
}

struct Build {
    client: PathBuf,
    resources: PathBuf,
}

fn prepared(path: &Path) -> Option<Build> {
    let client = path.join(CLIENT);
    if !path.is_absolute() || !client.is_file() {
        return None;
    }
    let lock = json(&path.join("research-lock.json"))?;
    let version = lock["minecraft"]["version"].as_str().filter(|v| leaf(v))?;
    let resources = path.join(".local/horizon-client").join(version);
    let manifest = json(&resources.join("horizon-resources.json"))?;
    (manifest["minecraft"].as_str() == Some(version)).then_some(Build { client, resources })
}

#[derive(Default)]
pub struct Player {
    pub name: String,
    pub ely_skins: bool,
    pub local_skin: Option<crate::skins::LocalSkin>,
}

pub fn game_home(settings: &HorizonSettings, root: &Path) -> Result<PathBuf> {
    let home = if settings.game_home.as_os_str().is_empty() {
        root.join("horizon-runtime/game")
    } else {
        ensure!(
            settings.game_home.is_absolute() && settings.game_home.is_dir(),
            "Выберите существующую папку игры Horizon"
        );
        settings.game_home.clone()
    };
    Ok(home)
}

fn command(
    settings: &HorizonSettings,
    root: &Path,
    build: Build,
    player: Player,
) -> Result<Command> {
    let home = game_home(settings, root)?;
    fs::create_dir_all(&home)?;
    let mut command = Command::new(build.client);
    command
        .current_dir(&home)
        .env("HORIZON_HOME", &home)
        .env_remove("HORIZON_LAUNCHER_EXE")
        .env_remove("HORIZON_ACCOUNT_SESSIONS")
        .env_remove("HORIZON_SESSION_FILE")
        .env_remove("HORIZON_API_DISCOVERY")
        .env_remove("HORIZON_OFFLINE_SKIN")
        .env_remove("HORIZON_OFFLINE_SKIN_MODEL")
        .env_remove("HORIZON_OFFLINE_CAPE")
        .arg("--resources")
        .arg(build.resources);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    if !player.name.is_empty() {
        crate::auth::validate_name(&player.name)?;
        command.arg("--name").arg(&player.name);
    }
    let mut skin = player
        .local_skin
        .filter(|skin| skin.path(root).is_some_and(|path| path.is_file()));
    if skin.is_none() && player.ely_skins {
        skin = crate::skins::ely(root, &player.name).ok().flatten();
    }
    if let Some((skin, path)) = skin.and_then(|skin| skin.path(root).map(|path| (skin, path))) {
        command.env("HORIZON_OFFLINE_SKIN", path);
        command.env(
            "HORIZON_OFFLINE_SKIN_MODEL",
            if skin.slim { "slim" } else { "classic" },
        );
    }
    Ok(command)
}

pub fn launch(
    settings: &HorizonSettings,
    root: &Path,
    player: Player,
    ely: Option<ElySession>,
    progress: &(impl Fn(String) + Sync),
) -> Result<()> {
    let runtime = root.join("horizon-runtime");
    let installed = managed_build(&runtime).and_then(|path| prepared(&path));
    // Older MLauncher configurations may contain an explicitly imported build.
    let needs_install = installed.is_none() || settings.auto_update;
    let fallback = || {
        installed
            .map(|build| (build, true))
            .or_else(|| prepared(&settings.build).map(|build| (build, false)))
    };
    let (build, managed) = if supports_install() && needs_install {
        match install(root, progress) {
            Ok(path) => (
                prepared(&path).context("Установленные ресурсы Horizon не найдены")?,
                true,
            ),
            Err(error) => match fallback() {
                Some(build) => {
                    progress(format!("Обновление Horizon пропущено: {error}"));
                    build
                }
                None => return Err(error),
            },
        }
    } else {
        fallback().context("Horizon ещё не установлен")?
    };
    ensure!(
        managed || ely.is_none(),
        "Для входа на серверы Ely.by установите Horizon через MLauncher"
    );
    let mut command = command(settings, root, build, player)?;
    let discovery = ely
        .as_ref()
        .map(|session| {
            session.validate()?;
            ElyDiscovery::start()
        })
        .transpose()?;
    if let Some(discovery) = &discovery {
        command
            .env("HORIZON_API_DISCOVERY", &discovery.url)
            .arg("--session-stdin");
    }
    let log_path = root.join("horizon-launch.log");
    let log = File::create(&log_path)?;
    let mut child = command
        .stdin(if ely.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
        .context("Не удалось запустить клиент Horizon")?;
    if let Some(session) = ely {
        let sent = child
            .stdin
            .take()
            .context("Клиент Horizon не принял сессию")
            .and_then(|stdin| session.send(stdin));
        if let Err(error) = sent {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    }
    progress("Клиент Horizon запущен".into());
    let status = child.wait()?;
    ensure!(
        status.success(),
        "Клиент Horizon завершился с ошибкой ({status}). Журнал: {}",
        log_path.display()
    );
    Ok(())
}

pub struct ElySession {
    name: String,
    uuid: uuid::Uuid,
    token: zeroize::Zeroizing<String>,
}

impl ElySession {
    pub fn from_account(account: &crate::auth::Account) -> Option<Self> {
        Some(Self {
            name: account.name.clone(),
            uuid: uuid::Uuid::parse_str(&account.uuid).ok()?,
            token: account.session.as_ref()?.access_token.clone(),
        })
    }

    fn validate(&self) -> Result<()> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let response = client
            .post("https://authserver.ely.by/auth/validate")
            .json(&serde_json::json!({"accessToken": self.token.as_str()}))
            .send()
            .context("Не удалось проверить сессию Ely.by")?;
        ensure!(
            response.status().is_success(),
            "Сессия Ely.by недействительна. Войдите в аккаунт заново."
        );
        Ok(())
    }

    fn send(&self, stdin: impl Write) -> Result<()> {
        #[derive(serde::Serialize)]
        struct Session<'a> {
            name: &'a str,
            uuid: String,
            access_token: &'a str,
        }
        Ok(serde_json::to_writer(
            stdin,
            &Session {
                name: &self.name,
                uuid: self.uuid.simple().to_string(),
                access_token: self.token.as_str(),
            },
        )?)
    }
}

struct ElyDiscovery {
    url: String,
    stop: Arc<AtomicBool>,
}

impl ElyDiscovery {
    fn start() -> Result<Self> {
        let server = TcpListener::bind("127.0.0.1:0")?;
        server.set_nonblocking(true)?;
        let path = format!("/{}", uuid::Uuid::new_v4());
        let url = format!("http://{}{path}", server.local_addr()?);
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        // Only the discovery document is local. Game authentication goes directly to Ely.by.
        let body = br#"{"discovery":{"session":{"endpoints":{"join":{"uri":"https://authserver.ely.by/session/join"},"verify":{"uri":"https://authserver.ely.by/session/hasJoined"}}}}}"#;
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                match server.accept() {
                    Ok((mut stream, _)) => {
                        let timeout = Some(Duration::from_millis(400));
                        let _ = stream.set_read_timeout(timeout);
                        let _ = stream.set_write_timeout(timeout);
                        let mut request = String::new();
                        let mut reader = BufReader::new((&stream).take(2048));
                        if reader.read_line(&mut request).is_ok()
                            && request.split_ascii_whitespace().take(2).eq(["GET", &path])
                        {
                            let _ = write!(stream, "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len());
                            let _ = stream.write_all(body);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self { url, stop })
    }
}

impl Drop for ElyDiscovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn managed_build(root: &Path) -> Option<PathBuf> {
    let current = json(&root.join("current.json"))?;
    let id = current["sha256"].as_str().filter(|id| digest(id))?;
    let path = root.join("versions").join(id);
    prepared(&path).map(|_| path)
}

pub const fn supports_install() -> bool {
    cfg!(all(windows, target_arch = "x86_64"))
}

pub fn install(root: &Path, progress: &(impl Fn(String) + Sync)) -> Result<PathBuf> {
    ensure!(
        supports_install(),
        "Установка Horizon поддерживается на Windows x64"
    );
    let runtime = root.join("horizon-runtime");
    let versions = runtime.join("versions");
    fs::create_dir_all(&versions)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(runtime.join(".install.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).context("Установка Horizon уже выполняется")?;
    let cancel = AtomicBool::new(false);
    let client = downloads::client()?;
    progress("Проверка версии Horizon…".into());
    let release = downloads::json(
        &client,
        "https://api.github.com/repos/horizon-rust/horizon/releases/tags/nightly",
        &runtime.join("release.json"),
        &cancel,
    )?;
    let asset = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|asset| asset["name"] == "horizon-windows-x64.zip")
        .context("В официальном релизе Horizon нет архива Windows x64")?;
    let hash = asset["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|value| digest(value))
        .context("У архива Horizon нет SHA-256")?;
    let url =
        "https://github.com/horizon-rust/horizon/releases/download/nightly/horizon-windows-x64.zip";
    ensure!(
        asset["browser_download_url"] == url,
        "Неожиданный адрес архива Horizon"
    );
    let destination = versions.join(hash);
    if prepared(&destination).is_none() {
        ensure!(
            !destination.exists(),
            "Папка установки Horizon повреждена: {}",
            destination.display()
        );
        let archive = runtime
            .join("downloads")
            .join(format!("horizon-{hash}.zip"));
        progress("Скачивание клиента Horizon…".into());
        downloads::download(
            &client,
            &Download {
                url: url.into(),
                path: archive.clone(),
                checksum: Checksum::Sha256(hash.into()),
                size: None,
            },
            &cancel,
        )?;
        let stage = tempfile::tempdir_in(&versions)?;
        progress("Распаковка клиента Horizon…".into());
        downloads::extract(&archive, stage.path(), false, &cancel)?;
        let package = if stage.path().join(CLIENT).is_file() {
            stage.path().to_path_buf()
        } else {
            fs::read_dir(stage.path())?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .find(|path| path.join(CLIENT).is_file())
                .context("В архиве Horizon нет клиента")?
        };
        prepare(&client, &runtime, &package, &cancel, progress)?;
        ensure!(
            prepared(&package).is_some(),
            "Horizon не создал готовые ресурсы"
        );
        fs::rename(package, &destination)?;
        let _ = fs::remove_file(archive);
    }
    downloads::atomic_write(
        &runtime.join("current.json"),
        &serde_json::to_vec(&serde_json::json!({
            "sha256": hash
        }))?,
    )?;
    Ok(destination)
}

fn prepare(
    client: &reqwest::blocking::Client,
    runtime: &Path,
    package: &Path,
    cancel: &AtomicBool,
    progress: &(impl Fn(String) + Sync),
) -> Result<()> {
    let lock = json(&package.join("research-lock.json")).context("Нет research-lock.json")?;
    let version = lock["minecraft"]["version"]
        .as_str()
        .filter(|v| leaf(v))
        .context("Нет версии Minecraft в пакете Horizon")?;
    let major = lock["minecraft"]["java_major"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .context("Нет версии JDK в пакете Horizon")?;
    let minecraft = runtime.join("minecraft");
    let server = crate::game::horizon_inputs(
        client,
        &minecraft,
        &runtime.join("versions-cache.json"),
        version,
        cancel,
        progress,
    )?;
    let jdk = crate::java::jdk_home(runtime, major, cancel, progress)?;
    let preparer = package.join("horizon-prepare.exe");
    ensure!(preparer.is_file(), "В архиве нет horizon-prepare.exe");
    let log_path = runtime.join("prepare.log");
    let log = File::create(&log_path)?;
    progress(format!(
        "Подготовка ресурсов Horizon для Minecraft {version}…"
    ));
    let mut command = Command::new(preparer);
    command
        .current_dir(package)
        .arg("--root")
        .arg(package)
        .args(["--prepare", "--complete", "--server-jar"])
        .arg(server)
        .arg("--minecraft-dir")
        .arg(&minecraft)
        .arg("--java-home")
        .arg(jdk)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .context("Не удалось запустить подготовку Horizon")?;
    let deadline = Instant::now() + Duration::from_secs(1200);
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(
                status.success(),
                "Подготовка Horizon не завершилась ({status}). Журнал: {}",
                log_path.display()
            );
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "Истекло время подготовки Horizon. Журнал: {}",
                log_path.display()
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ely_session_preserves_the_full_uuid_and_routes_only_to_ely() -> Result<()> {
        let session = ElySession {
            name: "TestPlayer".into(),
            uuid: uuid::Uuid::from_u128(u128::MAX),
            token: zeroize::Zeroizing::new("fixture-token".into()),
        };
        let mut bytes = Vec::new();
        session.send(&mut bytes)?;
        let session: Value = serde_json::from_slice(&bytes)?;
        assert_eq!(session["uuid"], "ffffffffffffffffffffffffffffffff");
        let bridge = ElyDiscovery::start()?;
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()?;
        let actual: Value = client.get(&bridge.url).send()?.error_for_status()?.json()?;
        assert_eq!(
            actual["discovery"]["session"]["endpoints"]["join"]["uri"],
            "https://authserver.ely.by/session/join"
        );
        assert!(actual["discovery"]["player"].is_null());
        Ok(())
    }

    #[test]
    fn offline_player_uses_imported_skin_and_selected_model() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let file = root.join("skin.png");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([10, 20, 30, 255])).save(&file)?;
        let mut skin = crate::skins::import(root, &file)?;
        skin.slim = true;
        let expected = skin.path(root).unwrap();
        let command = command(
            &HorizonSettings::default(),
            root,
            Build {
                client: root.join(CLIENT),
                resources: root.join("resources"),
            },
            Player {
                name: "TestPlayer".into(),
                local_skin: Some(skin),
                ely_skins: true,
            },
        )?;
        assert!(command.get_args().any(|arg| arg == "TestPlayer"));
        let environment: std::collections::HashMap<_, _> = command.get_envs().collect();
        assert_eq!(
            environment[std::ffi::OsStr::new("HORIZON_OFFLINE_SKIN")],
            Some(expected.as_os_str())
        );
        assert_eq!(
            environment[std::ffi::OsStr::new("HORIZON_OFFLINE_SKIN_MODEL")],
            Some(std::ffi::OsStr::new("slim"))
        );
        Ok(())
    }

    #[test]
    #[ignore = "Downloads and prepares the official Horizon release"]
    fn live_install_without_horizon_launcher() -> Result<()> {
        let root = PathBuf::from(
            std::env::var_os("MLAUNCHER_HORIZON_INSTALL_TEST")
                .context("Set MLAUNCHER_HORIZON_INSTALL_TEST to a test data directory")?,
        );
        let build = install(&root, &|message| println!("{message}"))?;
        assert!(prepared(&build).is_some());
        assert_eq!(managed_build(&root.join("horizon-runtime")), Some(build));
        if std::env::var_os("MLAUNCHER_HORIZON_PLAY_TEST").is_some() {
            let settings = HorizonSettings {
                auto_update: false,
                ..HorizonSettings::default()
            };
            let player = Player {
                name: std::env::var("MLAUNCHER_HORIZON_TEST_PLAYER")
                    .unwrap_or_else(|_| "MLauncherTest".into()),
                ely_skins: true,
                local_skin: std::env::var_os("MLAUNCHER_HORIZON_TEST_PNG")
                    .map(|path| crate::skins::import(&root, Path::new(&path)))
                    .transpose()?,
            };
            if std::env::var_os("MLAUNCHER_HORIZON_TEST_PLAYER").is_some()
                && player.local_skin.is_none()
            {
                assert!(crate::skins::ely(&root, &player.name)?.is_some());
            }
            launch(&settings, &root, player, None, &|message| {
                println!("{message}")
            })?;
        }
        Ok(())
    }
}
