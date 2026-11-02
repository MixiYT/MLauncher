//! Opt-in overlay for our own child process. Authenticated loopback bridge, not a public API.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

pub static REVISION: AtomicU64 = AtomicU64::new(0);
pub struct Options {
    pub dll: PathBuf,
    pub key: String,
    pub accent: [u8; 3],
    pub light: bool,
    pub effect: String,
}
impl Options {
    pub fn installed(key: String, accent: [u8; 3], light: bool, effect: impl Into<String>) -> Self {
        Self {
            dll: std::env::current_exe()
                .unwrap_or_default()
                .with_file_name("mlauncher-overlay.dll"),
            key,
            accent,
            light,
            effect: effect.into(),
        }
    }
}
struct State {
    token: String,
    host: String,
    name: String,
    mc: String,
    instance: PathBuf,
    key: zeroize::Zeroizing<String>,
    logs: Mutex<VecDeque<String>>,
    stop: AtomicBool,
    workers: AtomicUsize,
    download: AtomicBool,
    projects: Mutex<Vec<crate::packs::Entry>>,
    native_status: Mutex<Option<String>>,
}
pub struct Bridge {
    state: Arc<State>,
    pub port: u16,
}
impl Bridge {
    pub fn secret(&self) -> &str {
        &self.state.token
    }
    pub fn start(profile: &crate::model::Profile, instance: PathBuf, key: String) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let state = Arc::new(State {
            token: uuid::Uuid::new_v4().simple().to_string(),
            host: format!("127.0.0.1:{port}"),
            name: profile.name.clone(),
            mc: profile.game_version.clone(),
            instance,
            key: zeroize::Zeroizing::new(key),
            logs: Mutex::new(VecDeque::new()),
            stop: AtomicBool::new(false),
            workers: AtomicUsize::new(0),
            download: AtomicBool::new(false),
            projects: Mutex::new(vec![]),
            native_status: Mutex::new(None),
        });
        let shared = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if shared.stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else {
                    break;
                };
                if shared.workers.fetch_add(1, Ordering::SeqCst) >= 4 {
                    shared.workers.fetch_sub(1, Ordering::SeqCst);
                    drop(stream);
                    continue;
                }
                let shared = shared.clone();
                std::thread::spawn(move || {
                    struct Guard(Arc<State>);
                    impl Drop for Guard {
                        fn drop(&mut self) {
                            self.0.workers.fetch_sub(1, Ordering::SeqCst);
                        }
                    }
                    let _guard = Guard(shared.clone());
                    let _ = serve(stream, &shared);
                });
            }
        });
        Ok(Self { state, port })
    }
    pub fn configure(&self, command: &mut std::process::Command) {
        command
            .env("MLAUNCHER_OVERLAY_PORT", self.port.to_string())
            .env("MLAUNCHER_OVERLAY_TOKEN", &self.state.token);
    }
    pub fn log(&self, line: &str) {
        if let Ok(mut logs) = self.state.logs.lock() {
            let mut end = line.len().min(512);
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            if logs.len() >= 300 {
                logs.pop_front();
            }
            logs.push_back(line[..end].to_owned());
        }
    }
    pub fn take_status(&self) -> Option<String> {
        self.state.native_status.lock().ok()?.take()
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn response(stream: &mut TcpStream, code: u16, body: Value) -> Result<()> {
    let body = serde_json::to_vec(&body)?;
    write!(stream,"HTTP/1.1 {code} Response\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",body.len())?;
    stream.write_all(&body)?;
    // A Windows socket closed with unread request bytes sends RST and can discard
    // a 403/413 response. Half-close the response, then drain a bounded amount.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut ignored = [0u8; 1024];
    for _ in 0..8 {
        if !matches!(stream.read(&mut ignored),Ok(n) if n>0) {
            break;
        }
    }
    Ok(())
}
fn serve(mut stream: TcpStream, state: &State) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    let started = std::time::Instant::now();
    while !header.ends_with(b"\r\n\r\n") {
        if started.elapsed() > Duration::from_secs(2) {
            return response(&mut stream, 408, json!({"error":"Request timeout"}));
        }
        if header.len() >= 8192 {
            return response(&mut stream, 413, json!({"error":"Header too large"}));
        }
        stream.read_exact(&mut byte)?;
        header.push(byte[0]);
    }
    let header = std::str::from_utf8(&header)?;
    let mut lines = header.split("\r\n");
    let request: Vec<_> = lines.next().unwrap_or("").split_whitespace().collect();
    if request.len() != 3 {
        return response(&mut stream, 400, json!({"error":"Bad request"}));
    }
    let (mut token, mut host, mut length, mut origin, mut content_type, mut transfer) =
        ("", "", 0usize, false, "", false);
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            match name.to_ascii_lowercase().as_str() {
                "x-mlauncher-token" => token = value.trim(),
                "host" => host = value.trim(),
                "origin" => origin = true,
                "content-length" => length = value.trim().parse().unwrap_or(usize::MAX),
                "content-type" => content_type = value.trim(),
                "transfer-encoding" => transfer = true,
                _ => {}
            }
        }
    }
    if token != state.token || host != state.host || origin {
        return response(&mut stream, 403, json!({"error":"Forbidden"}));
    }
    if length > 2048 || transfer {
        return response(&mut stream, 413, json!({"error":"Body too large"}));
    }
    if request[0] == "GET" && request[1] == "/logs" {
        let logs = state
            .logs
            .lock()
            .map(|l| l.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        return response(&mut stream, 200, json!({"name":state.name,"logs":logs}));
    }
    if request[0] != "POST" || !content_type.starts_with("application/json") {
        return response(&mut stream, 405, json!({"error":"JSON POST required"}));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return response(&mut stream, 400, json!({"error":"Invalid JSON"})),
    };
    let result = (|| -> Result<Value> {
        if request[1] == "/ready" {
            let message = body["message"].as_str().context("Нет сообщения")?;
            *state
                .native_status
                .lock()
                .map_err(|_| anyhow::anyhow!("Статус недоступен"))? =
                Some(message.chars().take(256).collect());
            return Ok(json!({"ok":true}));
        }
        let provider = match body["provider"].as_str().unwrap_or("modrinth") {
            "modrinth" => crate::packs::Provider::Modrinth,
            "curseforge" => crate::packs::Provider::CurseForge,
            _ => anyhow::bail!("Неизвестный источник"),
        };
        let service = crate::packs::Service::new(state.key.to_string())?;
        match request[1] {
            "/resources" => {
                let query = body["query"].as_str().unwrap_or("");
                anyhow::ensure!(query.len() <= 256, "Слишком длинный запрос");
                let sort = match body["sort"].as_str().unwrap_or("downloads") {
                    "relevance" => crate::model::SortOrder::Relevance,
                    "updated" => crate::model::SortOrder::Updated,
                    "newest" => crate::model::SortOrder::Newest,
                    _ => crate::model::SortOrder::Downloads,
                };
                let entries = service.search_sorted(provider, query, true, &state.mc, sort)?;
                let entries: Vec<_> = entries.into_iter().take(20).collect();
                *state
                    .projects
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Каталог недоступен"))? = entries.clone();
                // Only a server-issued project may be downloaded. No client URLs or local paths.
                let installed = crate::packs::resources(&state.instance);
                let entries:Vec<_>=entries.into_iter().take(20).map(|e|json!({"id":e.id,"name":e.name,"description":e.description.chars().take(220).collect::<String>(),"author":e.author,"installed":installed.iter().any(|r|r.project==e.id&&r.provider==provider)})).collect();
                Ok(json!({"entries":entries,"minecraft":state.mc,"name":state.name}))
            }
            "/download" => {
                anyhow::ensure!(
                    !state.download.swap(true, Ordering::SeqCst),
                    "Уже идёт загрузка"
                );
                struct Guard<'a>(&'a AtomicBool);
                impl Drop for Guard<'_> {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::SeqCst);
                    }
                }
                let _guard = Guard(&state.download);
                let id = body["id"].as_str().context("Нет проекта")?;
                anyhow::ensure!(
                    id.len() <= 100
                        && id
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                    "Некорректный проект"
                );
                let entry = state
                    .projects
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Каталог недоступен"))?
                    .iter()
                    .find(|e| e.provider == provider && e.id == id)
                    .cloned()
                    .context("Сначала найдите ресурспак в каталоге")?;
                let versions = service.versions(&entry, true, &state.mc)?;
                let version = versions.first().context("Нет совместимой версии")?;
                service.resource(&entry, &version.id, &state.instance, &state.stop)?;
                REVISION.fetch_add(1, Ordering::Relaxed);
                Ok(json!({"message":"Ресурспак скачан. Включите его в настройках Minecraft."}))
            }
            _ => anyhow::bail!("Неизвестная операция"),
        }
    })();
    match result {
        Ok(value) => response(&mut stream, 200, value),
        Err(error) => response(&mut stream, 400, json!({"error":format!("{error:#}")})),
    }
}

fn expected_hash() -> String {
    option_env!("MLAUNCHER_OVERLAY_SHA256")
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("MLAUNCHER_OVERLAY_SHA256").ok())
        .unwrap_or_default()
}

pub fn check(dll: &std::path::Path) -> Result<()> {
    use sha2::Digest;
    let expected = expected_hash();
    anyhow::ensure!(
        !expected.is_empty(),
        "Оверлей отсутствует в этой сборке. Используйте scripts/package.ps1"
    );
    let dll = dll
        .canonicalize()
        .context("Нет mlauncher-overlay.dll рядом с EXE")?;
    anyhow::ensure!(
        std::fs::metadata(&dll)?.len() <= 16 * 1024 * 1024,
        "Слишком большая DLL"
    );
    let actual = format!("{:x}", sha2::Sha256::digest(std::fs::read(&dll)?));
    anyhow::ensure!(
        actual.eq_ignore_ascii_case(&expected),
        "Хеш DLL оверлея не совпал с этой сборкой"
    );
    Ok(())
}

#[cfg(all(windows, not(feature = "lite")))]
pub fn inject(child: &std::process::Child, dll: &std::path::Path) -> Result<()> {
    use sha2::Digest;
    use std::{
        os::windows::{ffi::OsStrExt, io::AsRawHandle},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::*,
        System::{
            Diagnostics::{Debug::WriteProcessMemory, ToolHelp::*},
            LibraryLoader::*,
            Memory::*,
            Threading::*,
        },
    };
    unsafe fn find_module(
        pid: u32,
        select: impl Fn(&MODULEENTRY32W) -> Option<usize>,
    ) -> Result<usize> {
        for _ in 0..120 {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
            if snapshot == INVALID_HANDLE_VALUE {
                let error = GetLastError();
                anyhow::ensure!(
                    error == ERROR_BAD_LENGTH || error == ERROR_PARTIAL_COPY,
                    "Не удалось перечислить модули Java (Win32 {error})"
                );
            } else {
                let mut module: MODULEENTRY32W = std::mem::zeroed();
                module.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
                let mut next = Module32FirstW(snapshot, &mut module);
                let mut found = None;
                while next != 0 {
                    if let Some(value) = select(&module) {
                        found = Some(value);
                        break;
                    }
                    next = Module32NextW(snapshot, &mut module);
                }
                CloseHandle(snapshot);
                if let Some(value) = found {
                    return Ok(value);
                }
            }
            // The loader's module list is not stable immediately after CreateProcess.
            std::thread::sleep(Duration::from_millis(25));
        }
        anyhow::bail!("Системный модуль или DLL не появились в Java за 3 секунды")
    }
    let expected = expected_hash();
    anyhow::ensure!(
        !expected.is_empty(),
        "Оверлей отсутствует в этой сборке. Используйте scripts/package.ps1"
    );
    let dll = dll
        .canonicalize()
        .context("Нет mlauncher-overlay.dll рядом с EXE")?;
    anyhow::ensure!(
        std::fs::metadata(&dll)?.len() <= 16 * 1024 * 1024,
        "Слишком большая DLL"
    );
    let actual = format!("{:x}", sha2::Sha256::digest(std::fs::read(&dll)?));
    anyhow::ensure!(
        actual.eq_ignore_ascii_case(&expected),
        "Хеш DLL оверлея не совпал с этой сборкой"
    );
    unsafe {
        let process = child.as_raw_handle();
        let mut wow = 0;
        anyhow::ensure!(
            IsWow64Process(process, &mut wow) != 0 && wow == 0,
            "Оверлей поддерживает только x64 Java"
        );
        let kernel = GetModuleHandleW(windows_sys::w!("kernel32.dll"));
        let entry =
            GetProcAddress(kernel, windows_sys::s!("LoadLibraryW")).context("Нет LoadLibraryW")?;
        // Resolve forwarded LoadLibraryW using the owning module and its remote base (ASLR-safe).
        let mut owner = ptr::null_mut();
        anyhow::ensure!(
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                entry as *const u16,
                &mut owner
            ) != 0,
            "Не найден модуль LoadLibraryW"
        );
        let mut name = [0u16; 1024];
        let n = GetModuleFileNameW(owner, name.as_mut_ptr(), name.len() as u32);
        let local = PathBuf::from(String::from_utf16_lossy(&name[..n as usize]));
        let target_name = local
            .file_name()
            .context("Нет имени модуля")?
            .to_string_lossy();
        let found = find_module(child.id(), |module| {
            let len = module
                .szModule
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(module.szModule.len());
            if String::from_utf16_lossy(&module.szModule[..len]).eq_ignore_ascii_case(&target_name)
            {
                return Some(module.modBaseAddr as usize);
            }
            None
        })?;
        let remote_entry = found + (entry as usize - owner as usize);
        let path: Vec<u16> = dll.as_os_str().encode_wide().chain(Some(0)).collect();
        let bytes = path.len() * 2;
        let remote = VirtualAllocEx(
            process,
            ptr::null(),
            bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        );
        anyhow::ensure!(!remote.is_null(), "Не удалось выделить путь DLL");
        if WriteProcessMemory(
            process,
            remote,
            path.as_ptr().cast(),
            bytes,
            ptr::null_mut(),
        ) == 0
        {
            VirtualFreeEx(process, remote, 0, MEM_RELEASE);
            anyhow::bail!("Не удалось передать путь DLL");
        }
        let thread = CreateRemoteThread(
            process,
            ptr::null(),
            0,
            Some(std::mem::transmute::<
                usize,
                unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
            >(remote_entry)),
            remote,
            0,
            ptr::null_mut(),
        );
        if thread.is_null() {
            VirtualFreeEx(process, remote, 0, MEM_RELEASE);
            anyhow::bail!("Не удалось подключить оверлей; проверьте защитное ПО");
        }
        let wait = WaitForSingleObject(thread, 10000);
        let mut code = 0;
        GetExitCodeThread(thread, &mut code);
        CloseHandle(thread);
        // On timeout the remote thread may still read the buffer; let process exit free it.
        if wait == WAIT_OBJECT_0 {
            VirtualFreeEx(process, remote, 0, MEM_RELEASE);
        }
        anyhow::ensure!(
            wait == WAIT_OBJECT_0,
            "DLL оверлея загружается слишком долго (код {code})"
        );
        // GetExitCodeThread is only 32 bits, whereas HMODULE is 64 bits. Confirm the module
        // rather than interpreting a truncated HMODULE as a success/failure boolean.
        let expected_path = dll
            .to_string_lossy()
            .trim_start_matches("\\\\?\\")
            .to_lowercase();
        find_module(child.id(), |module| {
            let len = module
                .szExePath
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(module.szExePath.len());
            let path = String::from_utf16_lossy(&module.szExePath[..len])
                .trim_start_matches("\\\\?\\")
                .to_lowercase();
            if path == expected_path {
                return Some(1);
            }
            None
        })
        .with_context(|| format!("DLL оверлея не загрузилась (код {code})"))?;
    }
    Ok(())
}
#[cfg(any(not(windows), feature = "lite"))]
pub fn inject(_child: &std::process::Child, _dll: &std::path::Path) -> Result<()> {
    anyhow::bail!("Нативный оверлей доступен только в полной Windows x64 версии")
}

pub fn smoke_test() -> Result<()> {
    anyhow::ensure!(!crate::variant::LITE, "Оверлей отсутствует в Lite");
    let root = tempfile::tempdir()?;
    let profile = crate::model::Config::default().profiles.remove(0);
    let bridge = Bridge::start(&profile, root.path().join("instance"), String::new())?;
    bridge.log("Тест нативного оверлея · только изолированный OpenGL-host, не Minecraft.");
    bridge.log("Shift+Tab: открыть / закрыть. Журнал и ресурспаки работают через локальную связь.");
    let native = std::env::current_dir()?.join("native/bin");
    let output = std::env::var_os("MLAUNCHER_OVERLAY_SCREENSHOT")
        .map(PathBuf::from)
        .unwrap_or(root.path().join("overlay-smoke.bmp"));
    let mut command = std::process::Command::new(native.join("overlay-test-host.exe"));
    bridge.configure(&mut command);
    command
        .arg(&output)
        .env("MLAUNCHER_OVERLAY_TEST", "1")
        .env("MLAUNCHER_OVERLAY_ACCENT", "234,112,141")
        .env("MLAUNCHER_OVERLAY_STYLE", "dark")
        .env("MLAUNCHER_OVERLAY_EFFECT", "Waves")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .context("Сначала выполните scripts/build-overlay.ps1 -TestHost")?;
    if let Err(e) = inject(&child, &native.join("mlauncher-overlay.dll")) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e);
    }
    let start = std::time::Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(35) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("OpenGL host завис");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let result = child.wait_with_output()?;
    println!("{}", String::from_utf8_lossy(&result.stdout));
    anyhow::ensure!(
        result.status.success(),
        "OpenGL smoke failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    anyhow::ensure!(output.is_file(), "Нет снимка framebuffer");
    if std::env::var_os("MLAUNCHER_OVERLAY_TEST_QUERY").is_some() {
        let entry = bridge
            .state
            .projects
            .lock()
            .unwrap()
            .first()
            .cloned()
            .context("Нативный поиск не вернул карточки")?;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()?;
        let url = format!("http://127.0.0.1:{}/download", bridge.port);
        for _ in 0..2 {
            let result: Value = client
                .post(&url)
                .header("X-MLauncher-Token", bridge.secret())
                .json(&json!({"id":entry.id,"provider":"modrinth"}))
                .send()?
                .json()?;
            anyhow::ensure!(
                result.get("error").is_none(),
                "Overlay download failed: {result}"
            );
        }
        anyhow::ensure!(
            crate::packs::resources(&bridge.state.instance).len() == 1,
            "Нет реестра скачанного ресурспака"
        );
        println!(
            "Native WinHTTP catalog search + authenticated resource download + duplicate check OK"
        );
    }
    println!("Authenticated loopback bridge + native DLL injection + Shift+Tab OK");
    println!("Screenshot: {}", output.display());
    if let Some(message) = bridge.take_status() {
        println!("{message}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_rejects_web_origins_and_requires_session_token() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = Bridge::start(
            &crate::model::Config::default().profiles[0],
            dir.path().into(),
            String::new(),
        )
        .unwrap();
        bridge.log("test log");
        let client = reqwest::blocking::Client::new();
        let url = format!("http://127.0.0.1:{}/logs", bridge.port);
        assert_eq!(client.get(&url).send().unwrap().status(), 403);
        assert_eq!(
            client
                .get(&url)
                .header("X-MLauncher-Token", &bridge.state.token)
                .header("Origin", "https://evil.invalid")
                .send()
                .unwrap()
                .status(),
            403
        );
        let body: Value = client
            .get(&url)
            .header("X-MLauncher-Token", &bridge.state.token)
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(body["logs"][0], "test log");
        assert_eq!(
            client
                .get(format!("http://127.0.0.1:{}/download", bridge.port))
                .header("X-MLauncher-Token", &bridge.state.token)
                .send()
                .unwrap()
                .status(),
            405
        );
        let url = format!("http://127.0.0.1:{}/download", bridge.port);
        let result = client
            .post(&url)
            .header("X-MLauncher-Token", bridge.secret())
            .json(&json!({"id":"not-issued","provider":"modrinth"}))
            .send()
            .unwrap();
        assert_eq!(result.status(), 400);
        let result = client
            .post(&url)
            .header("X-MLauncher-Token", bridge.secret())
            .header("Content-Type", "application/json")
            .body("x".repeat(2049))
            .send()
            .unwrap();
        assert_eq!(result.status(), 413);
        let other = Bridge::start(
            &crate::model::Config::default().profiles[0],
            dir.path().into(),
            String::new(),
        )
        .unwrap();
        assert_eq!(
            client
                .get(format!("http://127.0.0.1:{}/logs", bridge.port))
                .header("X-MLauncher-Token", other.secret())
                .send()
                .unwrap()
                .status(),
            403
        );
    }
}
