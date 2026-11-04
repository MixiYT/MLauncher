use crate::downloads::{self, Checksum, Download};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

const REPOSITORY: &str = "MixiYT/MLauncher";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    digest: Option<String>,
}

pub struct Ready {
    pub version: String,
    pub archive: PathBuf,
    sha256: String,
}

fn version(tag: &str) -> Option<[u32; 3]> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.');
    let numbers = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    let canonical = format!("{}.{}.{}", numbers[0], numbers[1], numbers[2]);
    (tag.strip_prefix('v').unwrap_or(tag) == canonical).then_some(numbers)
}

fn find_archive<'a>(
    releases: &'a [Release],
    current: [u32; 3],
    edition: &str,
) -> Option<([u32; 3], &'a Release, &'a Asset)> {
    releases
        .iter()
        .filter_map(|release| {
            let number = version(&release.tag_name)?;
            if release.draft || number <= current {
                return None;
            }
            let [major, minor, patch] = number;
            let name = format!("{edition}-{major}.{minor}.{patch}-win64.zip");
            let asset = release.assets.iter().find(|asset| asset.name == name)?;
            Some((number, release, asset))
        })
        .max_by_key(|(number, _, _)| *number)
}

pub fn download(root: &Path) -> Result<Option<Ready>> {
    let current = version(env!("CARGO_PKG_VERSION")).context("Некорректная версия лаунчера")?;
    let client = downloads::client()?;
    let releases: Vec<Release> = client
        .get(format!(
            "https://api.github.com/repos/{REPOSITORY}/releases?per_page=100"
        ))
        .send()?
        .error_for_status()?
        .json()?;
    let edition = if crate::variant::LITE {
        "MLauncher-Lite"
    } else {
        "MLauncher"
    };
    let Some(([major, minor, patch], release, asset)) = find_archive(&releases, current, edition)
    else {
        return Ok(None);
    };
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .context("У архива обновления нет контрольной суммы SHA-256")?;
    let archive = root.join("updates").join(&asset.name);
    downloads::download(
        &client,
        &Download {
            url: format!(
                "https://github.com/{REPOSITORY}/releases/download/{}/{}",
                release.tag_name, asset.name
            ),
            path: archive.clone(),
            checksum: Checksum::Sha256(sha256.to_owned()),
            size: None, // Verify the cached archive by hash on every check.
        },
        &AtomicBool::new(false),
    )?;
    Ok(Some(Ready {
        version: format!("{major}.{minor}.{patch}"),
        archive,
        sha256: sha256.to_owned(),
    }))
}

#[cfg(windows)]
pub fn install(update: &Ready) -> Result<bool> {
    use base64::Engine;
    use std::{os::windows::process::CommandExt, process::Command};

    let log = update.archive.with_extension("install.log");
    anyhow::ensure!(
        !log.exists(),
        "Предыдущая установка завершилась ошибкой. Подробности: {}",
        log.display()
    );
    let exe = std::env::current_exe()?;
    let name = if crate::variant::LITE {
        "MLauncher-Lite.exe"
    } else {
        "MLauncher.exe"
    };
    if !exe
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|file| file.eq_ignore_ascii_case(name))
    {
        return Ok(false);
    }
    let directory = exe.parent().context("Нет папки лаунчера")?;
    let stage = tempfile::Builder::new()
        .prefix(".mlauncher-update-")
        .tempdir_in(directory)?;
    let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
    let path_arg = |path: &Path| quote(&path.to_string_lossy());
    let files = if crate::variant::LITE {
        format!("@({})", quote(name))
    } else {
        format!("@({}, 'mlauncher-overlay.dll')", quote(name))
    };
    let script = format!(
        "$exe={}\n$directory={}\n$archive={}\n$digest={}\n$stage={}\n$log={}\n$launcherPid={}\n$files={}\n{}",
        path_arg(&exe),
        path_arg(directory),
        path_arg(&update.archive),
        quote(&update.sha256),
        path_arg(stage.path()),
        path_arg(&log),
        std::process::id(),
        files,
        include_str!("install-update.ps1"),
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").context("SystemRoot")?)
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .creation_flags(0x08000000)
        .spawn()?;
    let _ = stage.keep();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_newest_numeric_release_with_matching_edition() {
        let releases: Vec<Release> = serde_json::from_str(r#"[
            {"tag_name":"0.4.0","draft":false,"assets":[{"name":"MLauncher-0.4.0-win64.zip"}]},
            {"tag_name":"v0.3.10","draft":false,"assets":[{"name":"MLauncher-Lite-0.3.10-win64.zip"}]},
            {"tag_name":"0.3.9","draft":false,"assets":[{"name":"MLauncher-Lite-0.3.9-win64.zip"}]},
            {"tag_name":"1.0.0","draft":true,"assets":[{"name":"MLauncher-Lite-1.0.0-win64.zip"}]},
            {"tag_name":"1.0.0-rc.1","draft":false,"assets":[{"name":"MLauncher-Lite-1.0.0-win64.zip"}]}
        ]"#).unwrap();
        assert_eq!(
            find_archive(&releases, [0, 3, 6], "MLauncher-Lite")
                .unwrap()
                .0,
            [0, 3, 10]
        );
        assert_eq!(
            find_archive(&releases, [0, 3, 6], "MLauncher").unwrap().0,
            [0, 4, 0]
        );
        assert!(find_archive(&releases, [0, 3, 10], "MLauncher-Lite").is_none());
    }
}
