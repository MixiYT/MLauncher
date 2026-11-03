use crate::downloads::{self, Checksum, Download};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum Collector {
    #[default]
    Default,
    G1,
    Zgc,
    Shenandoah,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JavaSettings {
    pub executable: String,
    pub auto_download: bool,
    pub min_memory: u32,
    pub max_memory: u32,
    pub extra_args: String,
    pub collector: Collector,
    pub window_width: u32,
    pub window_height: u32,
    pub fullscreen: bool,
}
impl Default for JavaSettings {
    fn default() -> Self {
        Self {
            executable: String::new(),
            auto_download: true,
            min_memory: 512,
            max_memory: 4096,
            extra_args: String::new(),
            collector: Collector::Default,
            window_width: 960,
            window_height: 540,
            fullscreen: false,
        }
    }
}
#[derive(Clone)]
pub struct Installation {
    pub path: PathBuf,
    pub major: u32,
    pub description: String,
}

fn command(path: &Path) -> Command {
    let mut command = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .env_remove("JAVA_TOOL_OPTIONS")
        .env_remove("_JAVA_OPTIONS")
        .env_remove("JDK_JAVA_OPTIONS");
    command
}
pub fn parse_major(text: &str) -> Option<u32> {
    let re = regex::Regex::new(r#"(?:java|openjdk) version "([^\"]+)""#).ok()?;
    let value = re.captures(text)?.get(1)?.as_str();
    let mut parts = value.split(['.', '-', '+']);
    let first = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}
// Oracle's javapath/java.exe can be a forwarding process rather than the JVM.
// Resolve java.home from the runtime itself, so process lifetime, cancellation
// and the native overlay all belong to the actual Minecraft process.
fn runtime_executable(requested: &Path, properties: &str) -> Result<PathBuf> {
    let home = properties
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "java.home").then(|| PathBuf::from(value.trim()))
        })
        .context("Java не сообщила java.home")?;
    anyhow::ensure!(home.is_absolute(), "Java сообщила относительный java.home");
    let executable = home
        .join("bin")
        .join(if cfg!(windows) { "java.exe" } else { "java" });
    anyhow::ensure!(
        executable.is_file(),
        "В java.home нет исполняемого файла Java: {}",
        requested.display()
    );
    Ok(executable)
}
fn has_runtime(path: &Path) -> bool {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let Some(bin) = resolved.parent() else {
        return false;
    };
    if cfg!(windows) {
        bin.join("server/jvm.dll").is_file() || bin.join("client/jvm.dll").is_file()
    } else {
        bin.parent().is_some_and(|home| {
            home.join("lib/server/libjvm.so").is_file()
                || home.join("lib/server/libjvm.dylib").is_file()
        })
    }
}
pub fn probe(path: &Path) -> Result<Installation> {
    // Property output can exceed the capacity of a Windows pipe. Capture to a
    // temporary file so the bounded process wait cannot deadlock on stderr.
    use std::io::{Read, Seek, SeekFrom};
    let mut capture = tempfile::tempfile()?;
    let mut child = command(path)
        .args(["-XshowSettings:properties", "-version"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(capture.try_clone()?))
        .spawn()
        .context("Не удалось запустить Java")?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(8) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Java не отвечает");
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    anyhow::ensure!(status.success(), "Java завершилась с ошибкой");
    capture.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    capture.take(128 * 1024).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let major = parse_major(&text).context("Не удалось определить версию Java")?;
    if text.contains("32-Bit") || text.contains("Client VM") {
        bail!("Нужна 64-битная Java");
    }
    Ok(Installation {
        path: runtime_executable(path, &text)?,
        major,
        description: text
            .lines()
            .find(|line| line.contains("version \""))
            .unwrap_or("Java")
            .to_owned(),
    })
}
fn scan_directory(root: &Path, depth: usize, paths: &mut Vec<PathBuf>) {
    if depth == 0 || paths.len() > 160 {
        return;
    }
    for exe in [
        root.join("bin/java.exe"),
        root.join("java.exe"),
        root.join("bin/java"),
    ] {
        if exe.is_file() {
            paths.push(exe);
        }
    }
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten().take(100) {
            if entry
                .file_type()
                .map(|t| t.is_dir() && !t.is_symlink())
                .unwrap_or(false)
            {
                scan_directory(&entry.path(), depth - 1, paths);
            }
        }
    }
}
pub fn discover(storage: &Path) -> Vec<Installation> {
    let mut paths = vec![];
    for var in ["JAVA_HOME", "JDK_HOME"] {
        if let Some(path) = std::env::var_os(var) {
            scan_directory(Path::new(&path), 1, &mut paths);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let exe = dir.join(if cfg!(windows) { "java.exe" } else { "java" });
            if exe.is_file() {
                paths.push(exe);
            }
        }
    }
    for root in [
        Some(storage.join("runtimes")),
        std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join("Java")),
        std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join("Eclipse Adoptium")),
        std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join("Microsoft")),
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join(".minecraft/runtime")),
        std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Programs/Eclipse Adoptium")),
    ]
    .into_iter()
    .flatten()
    {
        scan_directory(&root, 5, &mut paths);
    }
    let mut seen = HashSet::new();
    let mut result = vec![];
    for path in paths {
        if !seen.insert(path.to_string_lossy().to_lowercase()) {
            continue;
        }
        if let Ok(java) = probe(&path) {
            result.push(java);
        }
    }
    result.sort_by(|a, b| b.major.cmp(&a.major).then_with(|| a.path.cmp(&b.path)));
    result
}
pub fn select(
    storage: &Path,
    major: u32,
    settings: &JavaSettings,
    cancel: &AtomicBool,
    progress: &impl Fn(String),
) -> Result<PathBuf> {
    if !settings.executable.trim().is_empty() {
        let installation = probe(Path::new(settings.executable.trim()))?;
        if installation.major != major {
            bail!(
                "Для этой версии нужна Java {major}, выбрана Java {}",
                installation.major
            );
        }
        return Ok(installation.path);
    }
    // Кеш выбранной автоматически Java: повторный запуск не сканирует диск и не запускает java -version.
    let marker = storage.join("metadata").join(format!("java-{major}.txt"));
    if let Ok(text) = fs::read_to_string(&marker) {
        let path = PathBuf::from(text.trim());
        if path.is_file() {
            if has_runtime(&path) {
                return Ok(path);
            }
            // Migrate markers written by older builds that cached Oracle shims.
            if let Ok(installation) = probe(&path) {
                if installation.major == major {
                    let _ = fs::write(&marker, installation.path.to_string_lossy().as_bytes());
                    return Ok(installation.path);
                }
            }
        }
    }
    progress(format!("Поиск Java {major}…"));
    if let Some(java) = discover(storage)
        .into_iter()
        .find(|java| java.major == major)
    {
        fs::create_dir_all(storage.join("metadata"))?;
        let _ = fs::write(&marker, java.path.to_string_lossy().as_bytes());
        return Ok(java.path);
    }
    if !settings.auto_download {
        bail!("Java {major} не найдена. Выберите путь или включите автоматическую загрузку.");
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    bail!("Автоустановка Java пока поддерживает Windows x64");
    let client = downloads::client()?;
    let runtime = storage.join("runtimes").join(format!("temurin-{major}"));
    let data=downloads::json(&client,&format!("https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture=x64&image_type=jre&os=windows&vendor=eclipse"),&storage.join("metadata").join(format!("java-{major}.json")),cancel)?;
    let package = &data
        .as_array()
        .and_then(|a| a.first())
        .context("Нет сборки Java для этой платформы")?["binary"]["package"];
    let url = package["link"].as_str().context("Нет URL Java")?;
    let hash = package["checksum"].as_str().context("Нет SHA256 Java")?;
    let archive = storage
        .join("downloads")
        .join(format!("java-{major}-{hash}.zip"));
    progress(format!("Скачивание Java {major}…"));
    downloads::download(
        &client,
        &Download {
            url: url.into(),
            path: archive.clone(),
            checksum: Checksum::Sha256(hash.into()),
            size: package["size"].as_u64(),
        },
        cancel,
    )?;
    downloads::check_cancel(cancel)?;
    let parent = runtime.parent().context("Папка Java")?;
    fs::create_dir_all(parent)?;
    let temporary = tempfile::tempdir_in(parent)?;
    progress(format!("Распаковка Java {major}…"));
    downloads::extract(&archive, temporary.path(), false, cancel)?;
    let mut candidates = vec![];
    scan_directory(temporary.path(), 4, &mut candidates);
    let exe = candidates
        .into_iter()
        .find(|p| p.ends_with("bin/java.exe"))
        .context("В архиве Java нет java.exe")?;
    let relative = exe.strip_prefix(temporary.path())?.to_path_buf();
    let installation = probe(&exe)?;
    if installation.major != major {
        bail!("Загруженная Java не соответствует версии");
    }
    if runtime.exists() {
        bail!(
            "Папка Java уже существует, но среда не найдена: {}. Выберите другую Java вручную.",
            runtime.display()
        );
    }
    fs::rename(temporary.path(), &runtime)?;
    let executable = runtime.join(relative);
    let _ = fs::write(&marker, executable.to_string_lossy().as_bytes());
    Ok(executable)
}
pub fn jvm_args(settings: &JavaSettings, major: u32) -> Result<Vec<String>> {
    if settings.min_memory < 128
        || settings.max_memory < 512
        || settings.min_memory > settings.max_memory
        || settings.max_memory > 65536
    {
        bail!("Проверьте память: Xms ≥ 128, Xmx ≥ 512 и Xms ≤ Xmx (МиБ)");
    }
    let mut args = vec![
        format!("-Xms{}M", settings.min_memory),
        format!("-Xmx{}M", settings.max_memory),
        "-Dfile.encoding=UTF-8".into(),
        "-Dlog4j2.formatMsgNoLookups=true".into(),
    ];
    match settings.collector {
        Collector::Default => {}
        Collector::G1 => args.push("-XX:+UseG1GC".into()),
        Collector::Zgc if major >= 17 => args.push("-XX:+UseZGC".into()),
        Collector::Shenandoah if major >= 17 => args.push("-XX:+UseShenandoahGC".into()),
        _ => bail!("Выбранный сборщик мусора требует Java 17+"),
    }
    for arg in settings
        .extra_args
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !arg.starts_with('-')
            || arg.contains(['\r', '\n'])
            || arg.starts_with("-Xmx")
            || arg.starts_with("-Xms")
            || ["-cp", "-classpath", "--class-path", "-jar"].contains(&arg)
            || arg.starts_with("-XX:PermSize")
            || arg.starts_with("-XX:MaxPermSize")
        {
            bail!("Недопустимый JVM-аргумент: {arg}. Один аргумент на строку, память задаётся отдельно.");
        }
        args.push(arg.into());
    }
    Ok(args)
}
pub fn java_command(path: &Path) -> Command {
    command(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn java_home_resolves_forwarders_and_never_falls_back_to_the_shim() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("Real JDK");
        fs::create_dir_all(home.join("bin/server")).unwrap();
        let binary = home
            .join("bin")
            .join(if cfg!(windows) { "java.exe" } else { "java" });
        fs::write(&binary, b"runtime").unwrap();
        let shim = root.path().join("javapath/java.exe");
        let output = format!("Property settings:\n    java.home = {}\n    java.vendor = Oracle\njava version \"21.0.11\"",home.display());
        assert_eq!(runtime_executable(&shim, &output).unwrap(), binary);
        assert!(runtime_executable(&shim, "java version \"21.0.11\"").is_err());
        assert!(!has_runtime(&shim));
        fs::write(
            home.join(if cfg!(windows) {
                "bin/server/jvm.dll"
            } else {
                "lib/server/libjvm.so"
            }),
            b"jvm",
        )
        .ok();
        if cfg!(windows) {
            assert!(has_runtime(&binary));
        }
    }
    #[test]
    #[ignore = "Requires the locally installed Oracle javapath forwarder"]
    fn oracle_forwarder_and_old_marker_resolve_to_the_real_runtime() {
        let shim = Path::new(r"C:\Program Files\Common Files\Oracle\Java\javapath\java.exe");
        let installation = probe(shim).unwrap();
        assert!(has_runtime(&installation.path));
        assert!(!installation.path.to_string_lossy().contains("javapath"));
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("metadata")).unwrap();
        let marker = root
            .path()
            .join("metadata")
            .join(format!("java-{}.txt", installation.major));
        fs::write(&marker, shim.to_string_lossy().as_bytes()).unwrap();
        let resolved = select(
            root.path(),
            installation.major,
            &JavaSettings::default(),
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
        assert_eq!(resolved, installation.path);
        assert_eq!(
            fs::read_to_string(marker).unwrap(),
            resolved.to_string_lossy()
        );
        println!(
            "Oracle forwarder + cached marker resolve to {}",
            resolved.display()
        );
    }
    #[test]
    fn versions() {
        assert_eq!(parse_major("java version \"1.8.0_452\""), Some(8));
        assert_eq!(parse_major("openjdk version \"21.0.2\" 2024"), Some(21));
        assert_eq!(parse_major("not java"), None);
    }
    #[test]
    fn memory_and_arguments() {
        let mut settings = JavaSettings::default();
        assert!(jvm_args(&settings, 21)
            .unwrap()
            .contains(&"-Xmx4096M".into()));
        settings.min_memory = 8192;
        assert!(jvm_args(&settings, 21).is_err());
        settings.min_memory = 512;
        settings.extra_args = "-XX:PermSize=104M".into();
        assert!(jvm_args(&settings, 21).is_err());
    }
}
