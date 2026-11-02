//! Encrypted session storage. Tokens are protected with Windows DPAPI for the
//! current user and never written to `config.json` in plain text.
use crate::auth::{Account, AccountKind, Session};
use anyhow::{bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const FILE: &str = "sessions.dat";

#[derive(Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    entries: HashMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct Packed {
    access_token: String,
    client_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

pub fn key(kind: AccountKind, uuid: &str) -> String {
    format!("{}:{uuid}", crate::auth::kind_key(kind))
}

fn path(root: &Path) -> PathBuf {
    root.join(FILE)
}

/// Separate DPAPI-protected API keys; never mixed with account sessions or exports.
pub fn save_api_keys(root: &Path, ai: &str, curseforge: &str) -> Result<()> {
    let plain = Zeroizing::new(serde_json::to_vec(&(ai, curseforge))?);
    crate::downloads::atomic_write(&root.join("api-keys.dat"), &protect(&plain)?)
}

pub fn load_api_keys(root: &Path) -> (String, String) {
    std::fs::read(root.join("api-keys.dat"))
        .ok()
        .and_then(|data| unprotect(&data).ok())
        .and_then(|data| serde_json::from_slice(&Zeroizing::new(data)).ok())
        .unwrap_or_default()
}

pub fn load(root: &Path) -> HashMap<String, Session> {
    let mut sessions = HashMap::new();
    let Ok(bytes) = std::fs::read(path(root)) else {
        return sessions;
    };
    let Ok(store) = serde_json::from_slice::<StoreFile>(&bytes) else {
        return sessions;
    };
    for (key, encoded) in store.entries {
        let Some(packed) = STANDARD
            .decode(encoded)
            .ok()
            .and_then(|blob| unprotect(&blob).ok())
            .and_then(|plain| serde_json::from_slice::<Packed>(&plain).ok())
        else {
            continue;
        };
        sessions.insert(
            key,
            Session {
                access_token: Zeroizing::new(packed.access_token),
                client_token: Zeroizing::new(packed.client_token),
                refresh_token: packed.refresh_token.map(Zeroizing::new),
            },
        );
    }
    sessions
}

pub fn save(root: &Path, accounts: &[Account]) -> Result<()> {
    let mut store = StoreFile::default();
    for account in accounts {
        let Some(session) = &account.session else {
            continue;
        };
        if account.kind == AccountKind::Offline {
            continue;
        }
        let packed = Packed {
            access_token: session.access_token.to_string(),
            client_token: session.client_token.to_string(),
            refresh_token: session.refresh_token.as_ref().map(|t| t.to_string()),
        };
        let plain = Zeroizing::new(serde_json::to_vec(&packed)?);
        store.entries.insert(
            key(account.kind, &account.uuid),
            STANDARD.encode(protect(&plain)?),
        );
    }
    std::fs::create_dir_all(root)?;
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    {
        use std::io::Write;
        file.write_all(&serde_json::to_vec(&store)?)?;
        file.as_file().sync_all()?;
    }
    file.persist(path(root))?;
    Ok(())
}

pub fn remove(root: &Path, account_key: &str) -> Result<()> {
    let Ok(bytes) = std::fs::read(path(root)) else {
        return Ok(());
    };
    let mut store: StoreFile = serde_json::from_slice(&bytes)?;
    if store.entries.remove(account_key).is_some() {
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        {
            use std::io::Write;
            file.write_all(&serde_json::to_vec(&store)?)?;
            file.as_file().sync_all()?;
        }
        file.persist(path(root))?;
    }
    Ok(())
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut output,
        )
    };
    if ok == 0 {
        bail!("DPAPI не смог зашифровать сессию");
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe {
        LocalFree(output.pbData as _);
    }
    Ok(bytes)
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut output,
        )
    };
    if ok == 0 {
        bail!("DPAPI не смог расшифровать сессию");
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe {
        LocalFree(output.pbData as _);
    }
    Ok(bytes)
}

#[cfg(not(windows))]
fn protect(_data: &[u8]) -> Result<Vec<u8>> {
    bail!("Защищённое хранение сессий реализовано только для Windows")
}

#[cfg(not(windows))]
fn unprotect(_data: &[u8]) -> Result<Vec<u8>> {
    bail!("Защищённое хранение сессий реализовано только для Windows")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Account;

    #[cfg(windows)]
    #[test]
    fn api_keys_are_encrypted_separately() {
        let root = tempfile::tempdir().unwrap();
        save_api_keys(root.path(), "private-ai-key", "private-cf-key").unwrap();
        let bytes = std::fs::read(root.path().join("api-keys.dat")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("private-"));
        assert_eq!(
            load_api_keys(root.path()),
            ("private-ai-key".into(), "private-cf-key".into())
        );
        save(root.path(), &[]).unwrap();
        assert_eq!(load_api_keys(root.path()).0, "private-ai-key");
    }

    #[cfg(windows)]
    #[test]
    fn sessions_roundtrip_encrypted_and_removed() {
        let root = tempfile::tempdir().unwrap();
        let mut account = Account::offline("Player", true).unwrap();
        account.kind = AccountKind::ElyBy;
        account.session = Some(Session {
            access_token: Zeroizing::new("secret-access-token".into()),
            client_token: Zeroizing::new("secret-client".into()),
            refresh_token: Some(Zeroizing::new("secret-refresh".into())),
        });
        save(root.path(), std::slice::from_ref(&account)).unwrap();
        // В файле нет открытых токенов.
        let raw = std::fs::read(root.path().join(FILE)).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("secret-access"));
        let loaded = load(root.path());
        let session = loaded.get(&key(AccountKind::ElyBy, &account.uuid)).unwrap();
        assert_eq!(&*session.access_token, "secret-access-token");
        assert_eq!(
            session.refresh_token.as_ref().map(|t| t.to_string()),
            Some("secret-refresh".into())
        );
        remove(root.path(), &key(AccountKind::ElyBy, &account.uuid)).unwrap();
        assert!(load(root.path()).is_empty());
    }
}
