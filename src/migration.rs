use crate::{
    downloads,
    model::{self, Profile},
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Debug)]
pub struct Build {
    pub name: String,
    pub source: PathBuf,
    pub minecraft: String,
    pub loader: String,
    pub loader_version: String,
    pub origin: String,
}

fn json(path: &Path) -> Option<Value> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > 8 * 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

fn prism(directory: &Path) -> Option<Build> {
    let pack = json(&directory.join("mmc-pack.json"))?;
    let components = pack["components"].as_array()?;
    let minecraft = components.iter().find(|c| c["uid"] == "net.minecraft")?["version"]
        .as_str()?
        .to_owned();
    let mut build = Build {
        name: directory.file_name()?.to_string_lossy().into_owned(),
        source: directory.join(".minecraft"),
        minecraft,
        loader: "vanilla".into(),
        loader_version: String::new(),
        origin: "Prism / ElyPrism / MultiMC".into(),
    };
    if !build.source.is_dir() {
        build.source = directory.join("minecraft");
    }
    if !build.source.is_dir() {
        return None;
    }
    if let Ok(config) = fs::read_to_string(directory.join("instance.cfg")) {
        if let Some(name) = config.lines().find_map(|line| line.strip_prefix("name=")) {
            build.name = name.trim().to_owned();
        }
    }
    for (uid, loader) in [
        ("net.fabricmc.fabric-loader", "fabric"),
        ("org.quiltmc.quilt-loader", "quilt"),
        ("net.minecraftforge", "forge"),
        ("net.neoforged", "neoforge"),
    ] {
        if let Some(component) = components.iter().find(|c| c["uid"] == uid) {
            build.loader = loader.into();
            build.loader_version = component["version"].as_str()?.into();
        }
    }
    Some(build)
}

fn version_details(metadata: &Value, id: &str) -> Option<(String, String, String)> {
    let mut minecraft = metadata["inheritsFrom"].as_str().unwrap_or(id).to_owned();
    let mut loader = "vanilla".to_owned();
    let mut loader_version = String::new();
    for library in metadata["libraries"].as_array().into_iter().flatten() {
        let Some(name) = library["name"].as_str() else {
            continue;
        };
        if let Some(mc) = name.strip_prefix("net.minecraft:client:") {
            minecraft = mc.into();
        }
        for (prefix, kind) in [
            ("net.fabricmc:fabric-loader:", "fabric"),
            ("org.quiltmc:quilt-loader:", "quilt"),
            ("net.minecraftforge:forge:", "forge"),
            ("net.neoforged:neoforge:", "neoforge"),
        ] {
            if let Some(version) = name.strip_prefix(prefix) {
                loader = kind.into();
                loader_version = version.into();
                if kind == "forge" {
                    if let Some((mc, forge)) = version.split_once('-') {
                        minecraft = mc.into();
                        loader_version = forge.into();
                    }
                }
            }
        }
    }
    if loader == "vanilla" && metadata["inheritsFrom"].is_string() {
        return None;
    }
    Some((minecraft, loader, loader_version))
}

fn standard(root: &Path) -> Vec<Build> {
    let mut builds = vec![];
    for config in [
        "launcher_profiles.json",
        "TlauncherProfiles.json",
        "launcher_profiles_microsoft_store.json",
    ] {
        let Some(value) = json(&root.join(config)) else {
            continue;
        };
        let Some(profiles) = value["profiles"].as_object() else {
            continue;
        };
        for (id, profile) in profiles {
            let Some(version) = profile["lastVersionId"].as_str().filter(|v| safe_id(v)) else {
                continue;
            };
            let version_dir = root.join("versions").join(version);
            let Some(metadata) = json(&version_dir.join(format!("{version}.json"))) else {
                continue;
            };
            let Some((minecraft, loader, loader_version)) = version_details(&metadata, version)
            else {
                continue;
            };
            let source = profile["gameDir"]
                .as_str()
                .map(PathBuf::from)
                .map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        root.join(path)
                    }
                })
                .unwrap_or_else(|| {
                    if version_dir.join("mods").is_dir() || version_dir.join("saves").is_dir() {
                        version_dir
                    } else {
                        root.to_path_buf()
                    }
                });
            if !source.is_dir() {
                continue;
            }
            builds.push(Build {
                name: profile["name"].as_str().unwrap_or(id).into(),
                source,
                minecraft,
                loader,
                loader_version,
                origin: "Minecraft / TLauncher / Legacy".into(),
            });
        }
    }
    if let Ok(versions) = fs::read_dir(root.join("versions")) {
        for entry in versions.flatten() {
            let directory = entry.path();
            if !directory.join("mods").is_dir() && !directory.join("saves").is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            if !safe_id(&id) || builds.iter().any(|b| b.source == directory) {
                continue;
            }
            let Some(metadata) = json(&directory.join(format!("{id}.json"))) else {
                continue;
            };
            let Some((minecraft, loader, loader_version)) = version_details(&metadata, &id) else {
                continue;
            };
            builds.push(Build {
                name: id,
                source: directory,
                minecraft,
                loader,
                loader_version,
                origin: "TLauncher".into(),
            });
        }
    }
    builds
}

pub fn scan(extra: Option<PathBuf>) -> Vec<Build> {
    let mut queue = VecDeque::new();
    if let Some(extra) = extra {
        queue.push_back((extra, 0));
    } else {
        for variable in ["APPDATA", "LOCALAPPDATA", "USERPROFILE"] {
            if let Some(root) = std::env::var_os(variable) {
                queue.push_back((PathBuf::from(root), 0));
            }
        }
        #[cfg(windows)]
        for drive in b'C'..=b'Z' {
            let root = PathBuf::from(format!("{}:/", drive as char));
            #[link(name = "kernel32")]
            extern "system" {
                fn GetDriveTypeW(root: *const u16) -> u32;
            }
            let wide: Vec<u16> = format!("{}:/", drive as char)
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };
            if (drive_type == 2 || drive_type == 3) && root.is_dir() {
                queue.push_back((root, 0));
            }
        }
    }
    let mut visited = HashSet::new();
    let mut unique = HashSet::new();
    let mut builds = vec![];
    while let Some((directory, depth)) = queue.pop_front() {
        if visited.len() >= 60_000 {
            break;
        }
        let Ok(metadata) = fs::symlink_metadata(&directory) else {
            continue;
        };
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                continue;
            }
        }
        if metadata.is_symlink() || !metadata.is_dir() {
            continue;
        }
        let Ok(canonical) = directory.canonicalize() else {
            continue;
        };
        if !visited.insert(canonical) {
            continue;
        }
        let candidates = prism(&directory).into_iter().chain(standard(&directory));
        for mut build in candidates {
            if !safe_id(&build.minecraft)
                || (!build.loader_version.is_empty() && !safe_id(&build.loader_version))
            {
                continue;
            }
            let Ok(source) = build.source.canonicalize() else {
                continue;
            };
            if unique.insert((
                source.clone(),
                build.minecraft.clone(),
                build.loader.clone(),
                build.loader_version.clone(),
            )) {
                build.source = source;
                builds.push(build);
            }
        }
        if directory.join("mmc-pack.json").is_file() || depth >= 6 {
            continue;
        }
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if [
                "windows",
                "program files",
                "program files (x86)",
                "$recycle.bin",
                "system volume information",
                "node_modules",
                ".git",
                "target",
                "cache",
                "mlauncher-cache",
                "assets",
                "libraries",
                "runtime",
                "runtimes",
                "saves",
                "screenshots",
                "mods",
                "resourcepacks",
                "shaderpacks",
                "mlauncher",
                "mlauncherlite",
            ]
            .contains(&name.as_str())
            {
                continue;
            }
            if entry
                .file_type()
                .is_ok_and(|t| t.is_dir() && !t.is_symlink())
            {
                queue.push_back((entry.path(), depth + 1));
            }
        }
    }
    builds.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    builds
}

pub fn import(build: &Build, instances: &Path, cancel: &AtomicBool) -> Result<Profile> {
    anyhow::ensure!(
        safe_id(&build.minecraft)
            && (build.loader_version.is_empty() || safe_id(&build.loader_version)),
        "Некорректная версия сборки"
    );
    fs::create_dir_all(instances)?;
    let stage = tempfile::Builder::new()
        .prefix(".import-")
        .tempdir_in(instances)?;
    for name in [
        "mods",
        "config",
        "defaultconfigs",
        "resourcepacks",
        "shaderpacks",
        "saves",
        "screenshots",
        "kubejs",
        "scripts",
        "journeymap",
        "xaero",
        "XaeroWaypoints",
        "XaeroWorldMap",
        "schematics",
        "patchouli_books",
        "local",
        "options.txt",
        "optionsof.txt",
        "optionsshaders.txt",
        "servers.dat",
        "servers.dat_old",
    ] {
        let source = build.source.join(name);
        if source.try_exists()? {
            copy(&source, &stage.path().join(name), cancel)?;
        }
    }
    downloads::atomic_write(
        &stage.path().join("launcher/pack-loader.json"),
        &serde_json::to_vec(
            &serde_json::json!({"loader":build.loader,"version":build.loader_version}),
        )?,
    )?;
    downloads::check_cancel(cancel)?;
    let used = fs::read_dir(instances)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let directory = model::unique_directory(&build.name, used.iter().map(String::as_str));
    let target = instances.join(&directory);
    anyhow::ensure!(!target.exists(), "Папка сборки уже существует");
    fs::rename(stage.path(), &target).context("Не удалось сохранить импортированную сборку")?;
    Ok(Profile {
        privacy: Default::default(),
        name: build.name.clone(),
        icon: Default::default(),
        game_version: build.minecraft.clone(),
        loader: build.loader.clone(),
        directory,
        java: None,
    })
}

pub(crate) fn copy(source: &Path, destination: &Path, cancel: &AtomicBool) -> Result<()> {
    downloads::check_cancel(cancel)?;
    let metadata = fs::symlink_metadata(source)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        anyhow::ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "Ссылка в сборке: {}",
            source.display()
        );
    }
    anyhow::ensure!(
        !metadata.is_symlink(),
        "Ссылка в сборке: {}",
        source.display()
    );
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy(&entry.path(), &destination.join(entry.file_name()), cancel)?;
        }
    } else if metadata.is_file() {
        use std::io::{Read, Write};
        let mut input = fs::File::open(source)?;
        let mut output = fs::File::create(destination)?;
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            downloads::check_cancel(cancel)?;
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .with_context(|| format!("Копирование {}", source.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn prism_discovery_and_transfer_preserve_worlds_and_loader() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("ElyPrism/instances/Fabric");
        write(&source.join("mmc-pack.json"), br#"{"components":[{"uid":"net.minecraft","version":"1.21.1"},{"uid":"net.fabricmc.fabric-loader","version":"0.16.9"}]}"#);
        write(
            &source.join("instance.cfg"),
            b"[General]
name=My Fabric",
        );
        write(&source.join(".minecraft/mods/test.jar"), b"mod");
        write(&source.join(".minecraft/saves/World/level.dat"), b"world");
        write(&source.join(".minecraft/options.txt"), b"options");
        write(
            &source.join(".minecraft/launcher_accounts.json"),
            b"private",
        );
        let builds = scan(Some(root.path().join("ElyPrism")));
        assert_eq!(builds.len(), 1);
        assert_eq!(builds[0].minecraft, "1.21.1");
        assert_eq!(builds[0].loader, "fabric");
        let instances = root.path().join("destination/instances");
        let profile = import(&builds[0], &instances, &AtomicBool::new(false)).unwrap();
        let target = instances.join(&profile.directory);
        assert_eq!(
            fs::read(target.join("saves/World/level.dat")).unwrap(),
            b"world"
        );
        assert_eq!(fs::read(target.join("mods/test.jar")).unwrap(), b"mod");
        assert!(source.join(".minecraft/saves/World/level.dat").is_file());
        assert!(!target.join("launcher_accounts.json").exists());
        let loader = json(&target.join("launcher/pack-loader.json")).unwrap();
        assert_eq!(loader["version"], "0.16.9");
        let second = import(&builds[0], &instances, &AtomicBool::new(false)).unwrap();
        assert_ne!(profile.directory, second.directory);
        assert!(import(&builds[0], &instances, &AtomicBool::new(true)).is_err());
        assert_eq!(fs::read_dir(&instances).unwrap().count(), 2);
    }

    #[test]
    fn isolated_tlauncher_and_explicit_game_directory_are_recognized() {
        let root = tempfile::tempdir().unwrap();
        let isolated = root.path().join("versions/ForgePack");
        write(&isolated.join("ForgePack.json"), br#"{"id":"ForgePack","inheritsFrom":"1.20.1","libraries":[{"name":"net.minecraftforge:forge:1.20.1-47.3.0"}]}"#);
        write(&isolated.join("mods/test.jar"), b"mod");
        write(
            &root.path().join("versions/1.21.1/1.21.1.json"),
            br#"{"id":"1.21.1","libraries":[]}"#,
        );
        write(&root.path().join("custom/saves/World/level.dat"), b"world");
        write(&root.path().join("launcher_profiles.json"), br#"{"profiles":{"isolated":{"name":"Forge","lastVersionId":"ForgePack"},"vanilla":{"lastVersionId":"1.21.1","gameDir":"custom"},"missing":{"lastVersionId":"fabric-missing"}}}"#);
        let builds = scan(Some(root.path().to_path_buf()));
        assert_eq!(builds.len(), 2);
        let forge = builds.iter().find(|b| b.loader == "forge").unwrap();
        assert_eq!(forge.minecraft, "1.20.1");
        assert_eq!(forge.loader_version, "47.3.0");
        assert_eq!(forge.source, isolated.canonicalize().unwrap());
        let vanilla = builds.iter().find(|b| b.loader == "vanilla").unwrap();
        assert_eq!(
            vanilla.source,
            root.path().join("custom").canonicalize().unwrap()
        );
        assert!(version_details(
            &serde_json::json!({"inheritsFrom":"1.20.1","libraries":[]}),
            "Unknown"
        )
        .is_none());
    }
}
