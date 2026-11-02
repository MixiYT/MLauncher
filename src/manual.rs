//! Explicit local import. No scraping, WebView automation or download restriction bypass.
use anyhow::{Context, Result};
use std::{fs, io::Write, path::Path};
pub fn import(source: &Path, instance: &Path) -> Result<()> {
    let name = source
        .file_name()
        .and_then(|s| s.to_str())
        .context("Нет имени JAR")?;
    crate::model::safe_filename(name)?;
    anyhow::ensure!(
        source.metadata()?.len() <= 512 * 1024 * 1024,
        "JAR больше 512 МиБ"
    );
    let mut zip = zip::ZipArchive::new(fs::File::open(source)?)?;
    anyhow::ensure!(
        zip.by_name("fabric.mod.json").is_ok()
            || zip.by_name("META-INF/mods.toml").is_ok()
            || zip.by_name("META-INF/neoforge.mods.toml").is_ok()
            || zip.by_name("quilt.mod.json").is_ok()
            || zip.by_name("mcmod.info").is_ok(),
        "Не найдены метаданные мода"
    );
    let mods = instance.join("mods");
    fs::create_dir_all(&mods)?;
    let destination = mods.join(name);
    anyhow::ensure!(
        !destination.exists(),
        "Этот файл уже установлен; замена вручную не выполняется"
    );
    let mut output = tempfile::NamedTempFile::new_in(&mods)?;
    std::io::copy(&mut fs::File::open(source)?, &mut output)?;
    output.flush()?;
    output.as_file().sync_all()?;
    output.persist_noclobber(destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn import_requires_mod_metadata_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("local.jar");
        let instance = root.path().join("instance");
        fs::write(&source, b"not a zip").unwrap();
        assert!(import(&source, &instance).is_err());
        let mut zip = zip::ZipWriter::new(fs::File::create(&source).unwrap());
        zip.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();
        import(&source, &instance).unwrap();
        assert!(import(&source, &instance).is_err());
        assert!(instance.join("mods/local.jar").is_file());
    }
}
