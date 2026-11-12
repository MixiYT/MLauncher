//! Restricted AI operations. All mod downloads use the regular verified installer.
use crate::{
    api::Api,
    assistant::{Action, Plan},
    downloads,
    java::JavaSettings,
    model::{self, Profile},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

pub struct Task {
    pub request: String,
    pub profile: Profile,
    pub instances: PathBuf,
    pub default_java: JavaSettings,
    pub retries: u8,
    pub max_repairs: u8,
    pub auto_repair: bool,
    pub session: Option<u64>,
    pub repair_only: bool,
    pub cancel: Arc<AtomicBool>,
}

pub struct Outcome {
    pub profile: Profile,
    pub created: bool,
    pub backup: Option<PathBuf>,
    pub launch: bool,
}

#[derive(Serialize, Deserialize)]
struct Backup {
    profile: Profile,
    files: Vec<String>,
}

fn validate(plan: &Plan, repair_only: bool) -> Result<()> {
    if repair_only {
        anyhow::ensure!(
            matches!(plan.action, Action::Repair { .. }),
            "После сбоя разрешено только исправление текущей сборки"
        );
    }
    let (mods, memory) = match &plan.action {
        Action::Create {
            name,
            minecraft,
            loader,
            memory_mb,
            mods,
            ..
        } => {
            anyhow::ensure!(
                !name.trim().is_empty() && name.len() <= 240,
                "Некорректное название сборки"
            );
            anyhow::ensure!(
                minecraft.len() <= 80
                    && minecraft
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
                "Некорректная версия Minecraft"
            );
            anyhow::ensure!(
                matches!(
                    loader.as_str(),
                    "vanilla" | "fabric" | "quilt" | "forge" | "neoforge"
                ),
                "Неизвестный загрузчик"
            );
            anyhow::ensure!(
                loader != "vanilla" || mods.is_empty(),
                "Для модов нужен загрузчик"
            );
            (mods, Some(*memory_mb))
        }
        Action::Repair {
            install,
            update,
            disable,
            memory_mb,
            ..
        } => {
            anyhow::ensure!(
                update.len() + disable.len() + install.len() <= 40,
                "Слишком много изменений"
            );
            for filename in update.iter().chain(disable) {
                model::safe_filename(filename)?;
                anyhow::ensure!(
                    filename.ends_with(".jar"),
                    "Изменять можно только JAR модов"
                );
            }
            (install, *memory_mb)
        }
        Action::Launch => return Ok(()),
    };
    anyhow::ensure!(
        mods.len() <= 40,
        "За один запрос можно установить до 40 модов"
    );
    for id in mods {
        anyhow::ensure!(
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
            "Некорректный slug Modrinth: {id}"
        );
    }
    if let Some(memory) = memory {
        anyhow::ensure!(
            (1024..=16384).contains(&memory),
            "Память должна быть от 1024 до 16384 МиБ"
        );
    }
    Ok(())
}

pub fn apply(
    plan: &Plan,
    current: &Profile,
    instances: &Path,
    cache: &Path,
    default_java: &JavaSettings,
    repair_only: bool,
    cancel: &AtomicBool,
    progress: impl Fn(String),
) -> Result<Outcome> {
    validate(plan, repair_only)?;
    downloads::check_cancel(cancel)?;
    fs::create_dir_all(instances)?;
    let api = Api::new(String::new())?;
    match &plan.action {
        Action::Create {
            name,
            minecraft,
            loader,
            memory_mb,
            mods,
            launch,
        } => {
            let (versions, _) = crate::versions::load(cache)?;
            anyhow::ensure!(
                versions.iter().any(|version| version.id == *minecraft),
                "Версии {minecraft} нет в каталоге Mojang"
            );
            let stage = tempfile::Builder::new()
                .prefix(".ai-create-")
                .tempdir_in(instances)?;
            let mut profile = Profile {
                privacy: Default::default(),
                name: name.trim().into(),
                game_version: minecraft.clone(),
                loader: loader.clone(),
                directory: String::new(),
                icon: Default::default(),
                java: Some(JavaSettings {
                    max_memory: *memory_mb,
                    ..Default::default()
                }),
            };
            for id in mods {
                downloads::check_cancel(cancel)?;
                progress(format!("ИИ: установка {id}"));
                let project = api.modrinth_project(id)?;
                api.install(&project, &profile, stage.path(), cancel, &progress)?;
            }
            downloads::atomic_write(
                &stage.path().join("launcher/ai-request.json"),
                &serde_json::to_vec_pretty(plan)?,
            )?;
            let used = fs::read_dir(instances)?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            profile.directory =
                model::unique_directory(&profile.name, used.iter().map(String::as_str));
            downloads::check_cancel(cancel)?;
            let target = instances.join(&profile.directory);
            anyhow::ensure!(!target.exists(), "Папка сборки уже существует");
            fs::rename(stage.path(), &target)?;
            Ok(Outcome {
                profile,
                created: true,
                backup: None,
                launch: *launch,
            })
        }
        Action::Repair {
            install,
            update,
            disable,
            memory_mb,
            reset_options,
            reset_java,
            launch,
        } => {
            anyhow::ensure!(
                model::is_safe_directory(&current.directory),
                "Некорректная папка сборки"
            );
            let instance = instances.join(&current.directory);
            let _lock = session_lock(&instance)?;
            recover(&instance)?;
            let backup = instance
                .join("launcher/ai-backups")
                .join(uuid::Uuid::new_v4().to_string());
            let stage = tempfile::Builder::new()
                .prefix(".ai-repair-")
                .tempdir_in(&instance)?;
            let mut files = Vec::new();
            if !install.is_empty() || !update.is_empty() || !disable.is_empty() {
                files.extend(["mods".to_owned(), "mods.json".to_owned()]);
            }
            if *reset_options {
                files.push("options.txt".into());
            }
            for name in &files {
                let source = instance.join(name);
                if source.exists() {
                    crate::migration::copy(&source, &stage.path().join(name), cancel)?;
                }
            }
            for filename in disable {
                let source = stage.path().join("mods").join(filename);
                anyhow::ensure!(source.is_file(), "Мод {filename} отсутствует");
                let target = source.with_file_name(format!("{filename}.disabled"));
                anyhow::ensure!(
                    !target.exists(),
                    "Отключённый файл {filename} уже существует"
                );
                fs::rename(source, target)?;
                progress(format!("ИИ: отключён {filename}"));
            }
            for filename in update {
                let installed = model::installed(stage.path())?;
                let item = installed.iter().find(|item| item.filename == *filename)
                    .context(format!("У {filename} нет данных для обновления. Можно отключить его и установить проект заново."))?;
                api.update(item, current, stage.path(), cancel, &progress)?;
            }
            for id in install {
                downloads::check_cancel(cancel)?;
                progress(format!("ИИ: установка {id}"));
                api.install(
                    &api.modrinth_project(id)?,
                    current,
                    stage.path(),
                    cancel,
                    &progress,
                )?;
            }
            if *reset_options && stage.path().join("options.txt").exists() {
                fs::remove_file(stage.path().join("options.txt"))?;
            }
            let mut profile = current.clone();
            if memory_mb.is_some() || *reset_java {
                let java = profile.java.get_or_insert_with(|| default_java.clone());
                if let Some(memory) = memory_mb {
                    java.max_memory = *memory;
                    java.min_memory = java.min_memory.min(*memory);
                }
                if *reset_java {
                    java.executable.clear();
                    java.extra_args.clear();
                    java.auto_download = true;
                }
            }
            let changed = !install.is_empty()
                || !update.is_empty()
                || !disable.is_empty()
                || memory_mb.is_some()
                || *reset_options
                || *reset_java;
            if !changed {
                return Ok(Outcome {
                    profile: current.clone(),
                    created: false,
                    backup: None,
                    launch: false,
                });
            }
            downloads::check_cancel(cancel)?;
            fs::create_dir_all(backup.join("original"))?;
            downloads::atomic_write(
                &backup.join("backup.json"),
                &serde_json::to_vec_pretty(&Backup {
                    profile: current.clone(),
                    files: files.clone(),
                })?,
            )?;
            downloads::atomic_write(&backup.join("plan.json"), &serde_json::to_vec_pretty(plan)?)?;
            commit(&instance, stage.path(), &backup, &files)?;
            Ok(Outcome {
                profile,
                created: false,
                backup: Some(backup),
                launch: *launch,
            })
        }
        Action::Launch => Ok(Outcome {
            profile: current.clone(),
            created: false,
            backup: None,
            launch: true,
        }),
    }
}

fn session_lock(instance: &Path) -> Result<fs::File> {
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(instance.join(".session.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).context("Сборка используется запущенной игрой")?;
    Ok(lock)
}

fn remove(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Transaction {
    files: Vec<(String, bool)>,
}

fn rollback(instance: &Path, backup: &Path, transaction: &Transaction) -> Result<()> {
    for (name, had_original) in &transaction.files {
        anyhow::ensure!(
            matches!(name.as_str(), "mods" | "mods.json" | "options.txt"),
            "Некорректный журнал исправления"
        );
        let original = backup.join("original").join(name);
        let destination = instance.join(name);
        if original.exists() {
            remove(&destination)?;
            fs::rename(original, destination)?;
        } else if !had_original {
            remove(&destination)?;
        }
    }
    fs::remove_file(backup.join("transaction.json"))?;
    Ok(())
}

/// Caller holds the instance session lock. Restores an interrupted replacement before launch.
pub fn recover(instance: &Path) -> Result<()> {
    let root = instance.join("launcher/ai-backups");
    if !root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || entry.file_type()?.is_symlink() {
            continue;
        }
        let backup = entry.path();
        let journal = backup.join("transaction.json");
        if journal.is_file() {
            let transaction: Transaction = serde_json::from_slice(&fs::read(&journal)?)?;
            rollback(instance, &backup, &transaction).with_context(|| {
                format!(
                    "Восстановление прерванного исправления {}",
                    backup.display()
                )
            })?;
        }
    }
    Ok(())
}

fn commit(instance: &Path, work: &Path, backup: &Path, files: &[String]) -> Result<()> {
    let transaction = Transaction {
        files: files
            .iter()
            .map(|name| (name.clone(), instance.join(name).exists()))
            .collect(),
    };
    downloads::atomic_write(
        &backup.join("transaction.json"),
        &serde_json::to_vec(&transaction)?,
    )?;
    let result = (|| -> Result<()> {
        for name in files {
            let destination = instance.join(name);
            if destination.exists() {
                fs::rename(&destination, backup.join("original").join(name))?;
            }
            let source = work.join(name);
            if source.exists() {
                fs::rename(source, destination)?;
            }
        }
        fs::remove_file(backup.join("transaction.json"))?;
        Ok(())
    })();
    if let Err(error) = result {
        rollback(instance, backup, &transaction)
            .with_context(|| format!("Нужен откат из {}", backup.display()))?;
        return Err(error.context("Исправление отменено, исходные файлы восстановлены"));
    }
    Ok(())
}

pub fn undo(backup: &Path, instance: &Path) -> Result<Outcome> {
    let manifest: Backup = serde_json::from_slice(&fs::read(backup.join("backup.json"))?)?;
    anyhow::ensure!(
        instance
            .file_name()
            .is_some_and(|name| name == manifest.profile.directory.as_str()),
        "Резервная копия относится к другой сборке"
    );
    let expected = instance.join("launcher/ai-backups").canonicalize()?;
    anyhow::ensure!(
        backup.canonicalize()?.parent() == Some(expected.as_path()),
        "Некорректный путь резервной копии"
    );
    let _lock = session_lock(instance)?;
    recover(instance)?;
    let stage = tempfile::Builder::new()
        .prefix(".ai-undo-")
        .tempdir_in(instance)?;
    for name in &manifest.files {
        anyhow::ensure!(
            matches!(name.as_str(), "mods" | "mods.json" | "options.txt"),
            "Некорректная резервная копия"
        );
        let source = backup.join("original").join(name);
        if source.exists() {
            crate::migration::copy(&source, &stage.path().join(name), &AtomicBool::new(false))?;
        }
    }
    let reverse = instance
        .join("launcher/ai-backups")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(reverse.join("original"))?;
    commit(instance, stage.path(), &reverse, &manifest.files)?;
    Ok(Outcome {
        profile: manifest.profile,
        created: false,
        backup: None,
        launch: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile {
            privacy: Default::default(),
            name: "Test".into(),
            game_version: "1.21.1".into(),
            loader: "fabric".into(),
            directory: "Test".into(),
            icon: Default::default(),
            java: None,
        }
    }

    fn repair(disable: Vec<String>, memory_mb: Option<u32>) -> Plan {
        Plan {
            explanation: "Fix".into(),
            action: Action::Repair {
                install: vec![],
                update: vec![],
                disable,
                memory_mb,
                reset_options: false,
                reset_java: false,
                launch: true,
            },
        }
    }

    #[test]
    fn repairs_keep_global_java_and_can_restore_original_mods() {
        let root = tempfile::tempdir().unwrap();
        let instances = root.path().join("instances");
        let instance = instances.join("Test");
        fs::create_dir_all(instance.join("mods")).unwrap();
        fs::write(instance.join("mods/conflict.jar"), b"original").unwrap();
        let defaults = JavaSettings {
            max_memory: 6144,
            extra_args: "-Dtest=value".into(),
            ..Default::default()
        };
        let outcome = apply(
            &repair(vec!["conflict.jar".into()], None),
            &profile(),
            &instances,
            root.path(),
            &defaults,
            true,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert!(outcome.profile.java.is_none());
        assert!(instance.join("mods/conflict.jar.disabled").is_file());
        assert!(!instance.join("mods/conflict.jar").exists());
        let restored = undo(outcome.backup.as_ref().unwrap(), &instance).unwrap();
        assert!(restored.profile.java.is_none());
        assert_eq!(
            fs::read(instance.join("mods/conflict.jar")).unwrap(),
            b"original"
        );
        assert!(!instance.join("mods/conflict.jar.disabled").exists());

        let outcome = apply(
            &repair(vec![], Some(8192)),
            &profile(),
            &instances,
            root.path(),
            &defaults,
            true,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let java = outcome.profile.java.unwrap();
        assert_eq!(java.extra_args, defaults.extra_args);
        assert_eq!(java.max_memory, 8192);
        assert!(instance.join("mods/conflict.jar").is_file());
    }

    #[test]
    fn session_lock_and_cancellation_block_mutations() {
        let root = tempfile::tempdir().unwrap();
        let instances = root.path().join("instances");
        let instance = instances.join("Test");
        fs::create_dir_all(instance.join("mods")).unwrap();
        fs::write(instance.join("mods/test.jar"), b"original").unwrap();
        let lock = session_lock(&instance).unwrap();
        let plan = repair(vec!["test.jar".into()], None);
        assert!(apply(
            &plan,
            &profile(),
            &instances,
            root.path(),
            &JavaSettings::default(),
            true,
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err());
        drop(lock);
        assert!(apply(
            &plan,
            &profile(),
            &instances,
            root.path(),
            &JavaSettings::default(),
            true,
            &AtomicBool::new(true),
            |_| {}
        )
        .is_err());
        assert_eq!(
            fs::read(instance.join("mods/test.jar")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn interrupted_transaction_is_recovered_before_use() {
        let root = tempfile::tempdir().unwrap();
        let instance = root.path().join("Test");
        let backup = instance.join("launcher/ai-backups/one");
        fs::create_dir_all(backup.join("original/mods")).unwrap();
        fs::create_dir_all(instance.join("mods")).unwrap();
        fs::write(backup.join("original/mods/old.jar"), b"old").unwrap();
        fs::write(instance.join("mods/new.jar"), b"new").unwrap();
        fs::write(instance.join("mods.json"), b"new registry").unwrap();
        downloads::atomic_write(
            &backup.join("transaction.json"),
            &serde_json::to_vec(&Transaction {
                files: vec![("mods".into(), true), ("mods.json".into(), false)],
            })
            .unwrap(),
        )
        .unwrap();
        recover(&instance).unwrap();
        assert_eq!(fs::read(instance.join("mods/old.jar")).unwrap(), b"old");
        assert!(!instance.join("mods/new.jar").exists());
        assert!(!instance.join("mods.json").exists());
        recover(&instance).unwrap();
    }

    #[test]
    fn invalid_ai_actions_cannot_escape_the_instance_or_change_the_game_version() {
        assert!(validate(&repair(vec!["../outside.jar".into()], None), true).is_err());
        assert!(validate(&repair(vec![], Some(100_000)), true).is_err());
        let create = Plan {
            explanation: "Create".into(),
            action: Action::Create {
                name: "Test".into(),
                minecraft: "1.21.1".into(),
                loader: "fabric".into(),
                memory_mb: 4096,
                mods: vec![],
                launch: true,
            },
        };
        assert!(validate(&create, true).is_err());
        assert!(serde_json::from_str::<Plan>(r#"{"explanation":"x","action":{"type":"repair","command":"delete worlds","launch":true}}"#).is_err());
    }
}

#[cfg(test)]
mod live_tests {
    #[test]
    #[ignore = "Downloads a small real Modrinth build"]
    fn ai_create_live_modrinth_build() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let plan = Plan {
            explanation: "Install compatible Fabric mods".into(),
            action: Action::Create {
                name: "AI smoke".into(),
                minecraft: "1.21.1".into(),
                loader: "fabric".into(),
                memory_mb: 4096,
                mods: vec!["fabric-api".into(), "sodium".into()],
                launch: false,
            },
        };
        let instances = root.path().join("instances");
        let outcome = apply(
            &plan,
            &Profile {
                privacy: Default::default(),
                name: "Existing".into(),
                game_version: "1.21.1".into(),
                loader: "vanilla".into(),
                directory: "Existing".into(),
                icon: Default::default(),
                java: None,
            },
            &instances,
            &root.path().join("cache"),
            &JavaSettings::default(),
            false,
            &AtomicBool::new(false),
            |message| println!("{message}"),
        )
        .unwrap();
        assert!(outcome.created);
        let instance = instances.join(outcome.profile.directory);
        let mods = model::installed(&instance).unwrap();
        assert!(mods
            .iter()
            .any(|item| item.name.to_ascii_lowercase().contains("fabric")));
        assert!(mods
            .iter()
            .any(|item| item.name.to_ascii_lowercase().contains("sodium")));
        assert!(mods
            .iter()
            .all(|item| instance.join("mods").join(&item.filename).is_file()));
        println!("AI build installed and verified with {} mods", mods.len());
    }
}
