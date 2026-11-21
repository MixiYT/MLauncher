use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, io::Read, path::Path};

const MAX_PACKAGE: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
pub struct Mod {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
}

fn manifest(bytes: impl Read) -> Result<Mod> {
    let mut text = String::new();
    bytes.take(65_537).read_to_string(&mut text)?;
    ensure!(text.len() <= 65_536, "Слишком большой манифест");
    let value: toml::Value = toml::from_str(&text)?;
    let metadata: Mod = value.get("mod").cloned().unwrap_or(value).try_into()?;
    ensure!(
        !metadata.id.is_empty()
            && metadata.id.len() <= 64
            && metadata
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'),
        "Некорректный ID мода Horizon"
    );
    Ok(metadata)
}

fn package(bytes: &[u8]) -> Result<Mod> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let value = zip
        .by_name("horizon-mod.toml")
        .context("В архиве отсутствует horizon-mod.toml")?;
    let metadata = manifest(value)?;
    ensure!(
        zip.by_name("Cargo.toml").is_err() && zip.by_name("horizon-mixin.toml").is_err(),
        "Этот мод требует сборки нативного клиента Horizon"
    );
    Ok(metadata)
}

pub fn install(home: &Path, file: &Path) -> Result<()> {
    ensure!(
        file.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("hmod")),
        "Выберите файл .hmod"
    );
    let mut bytes = Vec::new();
    fs::File::open(file)?
        .take(MAX_PACKAGE + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_PACKAGE,
        "Размер мода превышает 64 МиБ"
    );
    let metadata = package(&bytes)?;
    let destination = home
        .join("horizon-mods")
        .join(format!("{}.hmod", metadata.id));
    if destination.exists() {
        ensure!(
            fs::read(&destination)? == bytes,
            "Мод с этим ID уже установлен"
        );
    } else {
        crate::downloads::atomic_write(&destination, &bytes)?;
    }
    set_enabled(home, &metadata.id, true)
}

pub fn installed(home: &Path) -> Result<BTreeMap<String, Mod>> {
    let mut result = BTreeMap::new();
    let entries = match fs::read_dir(home.join("horizon-mods")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(result),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path();
        let metadata = if path.is_dir() {
            fs::File::open(path.join("horizon-mod.toml"))
                .ok()
                .and_then(|file| manifest(file).ok())
        } else if path.extension().is_some_and(|ext| ext == "hmod") {
            (|| -> Result<Mod> {
                let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
                let entry = zip.by_name("horizon-mod.toml")?;
                manifest(entry)
            })()
            .ok()
        } else {
            None
        };
        if let Some(metadata) = metadata {
            result.entry(metadata.id.clone()).or_insert(metadata);
        }
    }
    Ok(result)
}

pub fn options(home: &Path) -> Result<String> {
    match fs::read_to_string(home.join("horizon-mods.txt")) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

pub fn enabled(options: &str, id: &str) -> bool {
    let key = format!("mod.{id}");
    options
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.trim() == key)
        .last()
        .is_some_and(|(_, value)| value.trim() == "true")
}

pub fn set_enabled(home: &Path, id: &str, enabled: bool) -> Result<()> {
    let key = format!("mod.{id}");
    let options = options(home)?;
    let mut next = String::new();
    for line in options.lines() {
        if !line
            .split_once(':')
            .is_some_and(|(name, _)| name.trim() == key)
        {
            next.push_str(line);
            next.push('\n');
        }
    }
    next.push_str(&format!("{key}:{enabled}\n"));
    crate::downloads::atomic_write(&home.join("horizon-mods.txt"), next.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn importing_enables_mod_without_changing_other_options() -> Result<()> {
        let home = tempfile::tempdir()?;
        let file = home.path().join("example.hmod");
        let mut zip = zip::ZipWriter::new(fs::File::create(&file)?);
        zip.start_file("horizon-mod.toml", zip::write::SimpleFileOptions::default())?;
        zip.write_all(b"id = \"example\"\nname = \"Example\"\nversion = \"1.0.0\"\n")?;
        zip.finish()?;
        set_enabled(home.path(), "other", true)?;
        install(home.path(), &file)?;
        assert_eq!(installed(home.path())?["example"].version, "1.0.0");
        assert!(enabled(&options(home.path())?, "example"));
        set_enabled(home.path(), "example", false)?;
        let options = options(home.path())?;
        assert!(!enabled(&options, "example"));
        assert!(enabled(&options, "other"));
        Ok(())
    }
}
