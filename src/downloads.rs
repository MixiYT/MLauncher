//! Verified, atomic downloads and safe ZIP extraction shared by game installation and Java.
use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

#[derive(Clone)]
pub enum Checksum {
    Sha1(String),
    Sha256(String),
}
#[derive(Clone)]
pub struct Download {
    pub url: String,
    pub path: PathBuf,
    pub checksum: Checksum,
    pub size: Option<u64>,
}
pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent("MLauncher/0.2 (Rust Minecraft launcher)")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(240))
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.url().scheme() != "https" {
                a.error("HTTPS required")
            } else if a.previous().len() > 8 {
                a.error("Too many redirects")
            } else {
                a.follow()
            }
        }))
        .build()?)
}
pub fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("Операция отменена");
    }
    Ok(())
}
pub fn relative(path: &str) -> Result<PathBuf> {
    if path.is_empty() || path.contains(['\\', ':', '\0']) {
        bail!("Небезопасный путь в метаданных");
    }
    let value = Path::new(path);
    if value
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || path
            .split('/')
            .any(|c| c == "." || c == ".." || c.ends_with(['.', ' ']))
    {
        bail!("Небезопасный путь: {path}");
    }
    Ok(value.to_path_buf())
}
pub fn verified(path: &Path, checksum: &Checksum, size: Option<u64>) -> Result<bool> {
    let Ok(mut file) = File::open(path) else {
        return Ok(false);
    };
    if size.is_some_and(|size| file.metadata().map(|m| m.len() != size).unwrap_or(true)) {
        return Ok(false);
    }
    let mut sha1 = sha1::Sha1::new();
    let mut sha256 = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        match checksum {
            Checksum::Sha1(_) => sha1.update(&buffer[..count]),
            Checksum::Sha256(_) => sha256.update(&buffer[..count]),
        }
    }
    let (actual, expected) = match checksum {
        Checksum::Sha1(expected) => (format!("{:x}", sha1.finalize()), expected),
        Checksum::Sha256(expected) => (format!("{:x}", sha256.finalize()), expected),
    };
    Ok(actual.eq_ignore_ascii_case(expected))
}
/// Быстрая проверка перед скачиванием: совпадает ли размер файла.
/// Полный хеш по-прежнему проверяется сразу после загрузки; при повторных запусках
/// повторное хеширование сотен мегабайт не выполняется (как в официальном лаунчере).
pub fn current(path: &Path, size: Option<u64>) -> bool {
    match (std::fs::metadata(path), size) {
        (Ok(metadata), Some(expected)) => metadata.len() == expected,
        _ => false,
    }
}
pub fn download(client: &Client, item: &Download, cancel: &AtomicBool) -> Result<()> {
    check_cancel(cancel)?;
    if current(&item.path, item.size) {
        return Ok(());
    }
    if item.size.is_none() && verified(&item.path, &item.checksum, None)? {
        return Ok(());
    }
    let url = reqwest::Url::parse(&item.url)?;
    if url.scheme() != "https" {
        bail!("Загрузка разрешена только по HTTPS");
    }
    let parent = item.path.parent().context("Нет папки загрузки")?;
    fs::create_dir_all(parent)?;
    for attempt in 0..3 {
        check_cancel(cancel)?;
        let result = (|| -> Result<()> {
            let mut response = client.get(url.clone()).send()?.error_for_status()?;
            let mut file = tempfile::NamedTempFile::new_in(parent)?;
            let mut buffer = [0u8; 65536];
            let mut size = 0u64;
            // Хешируем поток при записи, чтобы не перечитывать сотни мегабайт с диска.
            let mut sha1 = sha1::Sha1::new();
            let mut sha256 = Sha256::new();
            loop {
                check_cancel(cancel)?;
                let count = response.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                size += count as u64;
                if size > 1024 * 1024 * 1024 {
                    bail!("Файл превышает 1 ГиБ");
                }
                match item.checksum {
                    Checksum::Sha1(_) => sha1.update(&buffer[..count]),
                    Checksum::Sha256(_) => sha256.update(&buffer[..count]),
                }
                file.write_all(&buffer[..count])?;
            }
            let actual = match item.checksum {
                Checksum::Sha1(_) => format!("{:x}", sha1.finalize()),
                Checksum::Sha256(_) => format!("{:x}", sha256.finalize()),
            };
            let expected = match &item.checksum {
                Checksum::Sha1(value) | Checksum::Sha256(value) => value,
            };
            if !actual.eq_ignore_ascii_case(expected)
                || item.size.is_some_and(|expected| expected != size)
            {
                bail!("Контрольная сумма или размер не совпали");
            }
            file.as_file().sync_all()?;
            check_cancel(cancel)?;
            file.persist(&item.path)?;
            Ok(())
        })();
        match result {
            Ok(()) => return Ok(()),
            Err(error) if attempt == 2 => {
                return Err(error.context(format!(
                    "Не удалось загрузить {}",
                    item.path.file_name().unwrap_or_default().to_string_lossy()
                )))
            }
            Err(_) => {
                for _ in 0..5 {
                    check_cancel(cancel)?;
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
    unreachable!()
}
pub fn batch(
    client: &Client,
    items: &[Download],
    cancel: &AtomicBool,
    concurrency: usize,
    progress: impl Fn(usize, usize) + Sync,
) -> Result<()> {
    let next = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let error = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..concurrency.clamp(1, 32).min(items.len()) {
            let client = client.clone();
            let next = &next;
            let completed = &completed;
            let failed = &failed;
            let error = &error;
            let progress = &progress;
            scope.spawn(move || {
                while !failed.load(Ordering::Relaxed) {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= items.len() {
                        break;
                    }
                    if let Err(e) = download(&client, &items[index], cancel) {
                        failed.store(true, Ordering::Relaxed);
                        if let Ok(mut error) = error.lock() {
                            if error.is_none() {
                                *error = Some(e);
                            }
                        }
                        break;
                    }
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    progress(done, items.len());
                }
            });
        }
    });
    if let Some(error) = error
        .into_inner()
        .map_err(|_| anyhow::anyhow!("Ошибка потока загрузки"))?
    {
        return Err(error);
    }
    check_cancel(cancel)
}
pub fn json(
    client: &Client,
    url: &str,
    cache: &Path,
    cancel: &AtomicBool,
) -> Result<serde_json::Value> {
    check_cancel(cancel)?;
    let parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() != "https" {
        bail!("HTTPS required");
    }
    let result = (|| -> Result<serde_json::Value> {
        let response = client.get(parsed).send()?.error_for_status()?;
        let mut bytes = vec![];
        response
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 * 1024 {
            bail!("Метаданные слишком большие");
        }
        let value = serde_json::from_slice(&bytes)?;
        atomic_write(cache, &bytes)?;
        Ok(value)
    })();
    match result {
        Ok(value) => Ok(value),
        Err(e) => {
            check_cancel(cancel)?;
            match fs::read(cache)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
            {
                Some(value) => Ok(value),
                None => Err(e),
            }
        }
    }
}
/// Для неизменяемых метаданных: сначала локальный кеш, сеть только при его отсутствии.
pub fn json_cached(
    client: &Client,
    url: &str,
    cache: &Path,
    cancel: &AtomicBool,
) -> Result<serde_json::Value> {
    if let Ok(bytes) = fs::read(cache) {
        if let Ok(value) = serde_json::from_slice(&bytes) {
            return Ok(value);
        }
    }
    json(client, url, cache, cancel)
}

pub fn text_cached(
    client: &Client,
    url: &str,
    cache: &Path,
    cancel: &AtomicBool,
) -> Result<String> {
    if let Ok(text) = fs::read_to_string(cache) {
        if !text.trim().is_empty() {
            return Ok(text);
        }
    }
    download_text(client, url, cache, cancel)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Нет родительской папки")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
pub fn download_text(
    client: &Client,
    url: &str,
    cache: &Path,
    cancel: &AtomicBool,
) -> Result<String> {
    check_cancel(cancel)?;
    let parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() != "https" {
        bail!("HTTPS required");
    }
    let result = (|| -> Result<String> {
        let response = client.get(parsed).send()?.error_for_status()?;
        let mut text = String::new();
        response
            .take(16 * 1024 * 1024 + 1)
            .read_to_string(&mut text)?;
        if text.len() > 16 * 1024 * 1024 {
            bail!("Метаданные слишком большие");
        }
        atomic_write(cache, text.as_bytes())?;
        Ok(text)
    })();
    match result {
        Ok(text) => Ok(text),
        Err(e) => {
            check_cancel(cancel)?;
            match fs::read_to_string(cache).ok() {
                Some(text) => Ok(text),
                None => Err(e),
            }
        }
    }
}
pub fn extract(
    zip: &Path,
    destination: &Path,
    natives_only: bool,
    cancel: &AtomicBool,
) -> Result<()> {
    fs::create_dir_all(destination)?;
    let mut archive = zip::ZipArchive::new(File::open(zip)?)?;
    if archive.len() > 100_000 {
        bail!("Слишком много файлов в архиве");
    }
    let mut total = 0u64;
    for i in 0..archive.len() {
        check_cancel(cancel)?;
        let mut entry = archive.by_index(i)?;
        let name = entry.name().replace('\\', "/");
        if entry.is_dir() {
            continue;
        }
        let relative = relative(&name)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            bail!("Ссылки в ZIP запрещены");
        }
        if natives_only
            && (name.starts_with("META-INF/")
                || ![".dll", ".so", ".dylib"]
                    .iter()
                    .any(|extension| name.ends_with(extension)))
        {
            continue;
        }
        total += entry.size();
        if total > 2 * 1024 * 1024 * 1024 || entry.size() > 512 * 1024 * 1024 {
            bail!("Архив превышает лимит распаковки");
        }
        let path = destination.join(relative);
        fs::create_dir_all(path.parent().context("Нет папки файла")?)?;
        let mut output = File::create(path)?;
        let mut buffer = [0u8; 65536];
        loop {
            check_cancel(cancel)?;
            let n = entry.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_and_hashes() {
        for path in [
            "../bad",
            "/absolute",
            "C:/bad",
            "x/../bad",
            "x\\bad",
            "a/./bad",
        ] {
            assert!(relative(path).is_err(), "{path}");
        }
        assert!(relative("org/lwjgl/file.jar").is_ok());
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x");
        fs::write(&file, b"abc").unwrap();
        assert!(verified(
            &file,
            &Checksum::Sha1("a9993e364706816aba3e25717850c26c9cd0d89d".into()),
            Some(3)
        )
        .unwrap());
        assert!(!verified(&file, &Checksum::Sha1("wrong".into()), None).unwrap());
        // Быстрый путь: совпадение размера пропускает повторное хеширование.
        assert!(current(&file, Some(3)));
        assert!(!current(&file, Some(4)));
        assert!(!current(&file, None));
        assert!(!current(&dir.path().join("missing"), Some(3)));
    }
    #[test]
    fn immutable_metadata_uses_cache_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cached.json");
        fs::write(&cache, br#"{"cached":true}"#).unwrap();
        let client = client().unwrap();
        let cancel = AtomicBool::new(false);
        let value = json_cached(
            &client,
            "https://invalid.invalid/never-requested.json",
            &cache,
            &cancel,
        )
        .unwrap();
        assert_eq!(value["cached"], true);
        let text_cache = dir.path().join("cached.xml");
        fs::write(&text_cache, "<versions><version>1.0</version></versions>").unwrap();
        assert!(text_cached(
            &client,
            "https://invalid.invalid/never-requested.xml",
            &text_cache,
            &cancel
        )
        .unwrap()
        .contains("1.0"));
    }
}
