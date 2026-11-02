//! Ely.by's documented Yggdrasil flow. Passwords and tokens are never serialized.
use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use zeroize::Zeroizing;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountKind {
    Offline,
    ElyBy,
    Microsoft,
}

#[derive(Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub uuid: String,
    pub kind: AccountKind,
    pub ely_skins: bool,
    #[serde(skip)]
    pub session: Option<Session>,
}

// Intentionally no Debug/Serialize implementation: secrets must not reach logs/config.
pub struct Session {
    pub access_token: Zeroizing<String>,
    pub client_token: Zeroizing<String>,
    pub refresh_token: Option<Zeroizing<String>>,
}
impl Session {
    pub fn new(access_token: Zeroizing<String>, client_token: Zeroizing<String>) -> Self {
        Self {
            access_token,
            client_token,
            refresh_token: None,
        }
    }
}

pub fn kind_key(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Offline => "offline",
        AccountKind::ElyBy => "ely",
        AccountKind::Microsoft => "ms",
    }
}

impl Account {
    pub fn offline(name: &str, skins: bool) -> Result<Self> {
        validate_name(name)?;
        // Minecraft's UUID.nameUUIDFromBytes("OfflinePlayer:" + name), not UUID v3 with a namespace.
        let mut bytes: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
        bytes[6] = (bytes[6] & 0x0f) | 0x30;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(Self {
            name: name.into(),
            uuid: uuid::Uuid::from_bytes(bytes).simple().to_string(),
            kind: AccountKind::Offline,
            ely_skins: skins,
            session: None,
        })
    }
}

pub fn validate_name(name: &str) -> Result<()> {
    if !(3..=16).contains(&name.len())
        || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        bail!("Ник должен содержать 3–16 символов: A–Z, a–z, 0–9 и подчёркивание.");
    }
    Ok(())
}

pub fn authenticate(
    login: String,
    password: Zeroizing<String>,
    totp: Zeroizing<String>,
) -> Result<Account> {
    if login.trim().is_empty() || password.is_empty() {
        bail!("Введите логин и пароль Ely.by");
    }
    if !totp.is_empty() && (totp.len() != 6 || !totp.bytes().all(|c| c.is_ascii_digit())) {
        bail!("Код 2FA должен состоять из 6 цифр");
    }
    let combined = Zeroizing::new(if totp.is_empty() {
        password.to_string()
    } else {
        format!("{}:{}", *password, *totp)
    });
    let client_token = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("MLauncher/0.1.0")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .build()?;
    let mut payload = json!({ "username": login.trim(), "password": &*combined, "clientToken": &*client_token, "requestUser": true });
    let request = client
        .post("https://authserver.ely.by/auth/authenticate")
        .json(&payload);
    // Drop the intermediate JSON copy before waiting on the network.
    if let Some(Value::String(value)) = payload.get_mut("password") {
        use zeroize::Zeroize;
        value.zeroize();
    }
    let response = request.send()?;
    let status = response.status();
    let mut data: Value = response
        .json()
        .context("Ely.by вернул некорректный ответ")?;
    if !status.is_success() {
        // Never echo response bodies: a remote error must not accidentally expose credentials.
        let message = data["errorMessage"].as_str().unwrap_or_default();
        if message == "Account protected with two factor auth." {
            bail!("Аккаунт защищён 2FA. Введите код из приложения и повторите вход.");
        }
        if status.as_u16() == 429 {
            bail!("Слишком много попыток. Подождите и повторите вход.");
        }
        bail!(
            "Не удалось войти в Ely.by (HTTP {}). Проверьте логин, пароль и код 2FA.",
            status.as_u16()
        );
    }
    let token = data["accessToken"]
        .as_str()
        .context("Ely.by не вернул токен")?
        .to_owned();
    if token.is_empty() {
        bail!("Ely.by вернул пустой токен");
    }
    if let Some(Value::String(value)) = data.get_mut("accessToken") {
        use zeroize::Zeroize;
        value.zeroize();
    }
    let name = data["selectedProfile"]["name"]
        .as_str()
        .context("Нет имени профиля")?
        .to_owned();
    let id = data["selectedProfile"]["id"]
        .as_str()
        .context("Нет UUID профиля")?;
    let id = uuid::Uuid::parse_str(id)
        .context("Некорректный UUID Ely.by")?
        .simple()
        .to_string();
    Ok(Account {
        name,
        uuid: id,
        kind: AccountKind::ElyBy,
        ely_skins: true,
        session: Some(Session::new(Zeroizing::new(token), client_token)),
    })
}

/// Обновляет access token Ely.by без повторного ввода пароля.
pub fn refresh_elyby(session: &Session) -> Result<Session> {
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("MLauncher/0.1.0")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .build()?;
    let response = client
        .post("https://authserver.ely.by/auth/refresh")
        .json(&json!({
            "accessToken": &*session.access_token,
            "clientToken": &*session.client_token,
            "requestUser": true
        }))
        .send()?;
    if !response.status().is_success() {
        bail!("Сессия Ely.by истекла. Войдите повторно.");
    }
    let data: Value = response
        .json()
        .context("Ely.by вернул некорректный ответ")?;
    let token = data["accessToken"]
        .as_str()
        .context("Ely.by не вернул токен")?
        .to_owned();
    if token.is_empty() {
        bail!("Ely.by вернул пустой токен");
    }
    Ok(Session::new(
        Zeroizing::new(token),
        Zeroizing::new(session.client_token.to_string()),
    ))
}

pub fn invalidate(session: Session) -> Result<()> {
    reqwest::blocking::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(30)).build()?
        .post("https://authserver.ely.by/auth/invalidate")
        .json(&json!({ "accessToken": &*session.access_token, "clientToken": &*session.client_token }))
        .send()?.error_for_status()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_uuid_matches_minecraft() {
        let account = Account::offline("Notch", true).unwrap();
        assert_eq!(account.uuid, "b50ad385829d3141a2167e7d7539ba7f");
        assert!(account.ely_skins);
        assert!(Account::offline("bad/name", true).is_err());
    }
    #[test]
    fn secrets_are_not_serialized() {
        let mut account = Account::offline("Player", true).unwrap();
        account.session = Some(Session {
            access_token: Zeroizing::new("secret-access".into()),
            client_token: Zeroizing::new("secret-client".into()),
            refresh_token: Some(Zeroizing::new("secret-refresh".into())),
        });
        for kind in [AccountKind::ElyBy, AccountKind::Microsoft] {
            account.kind = kind;
            let json = serde_json::to_string(&account).unwrap();
            assert!(!json.contains("secret"));
            assert!(!json.contains("session"));
            assert!(serde_json::from_str::<Account>(&json)
                .unwrap()
                .session
                .is_none());
        }
    }
}
