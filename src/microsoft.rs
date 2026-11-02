//! OAuth device flow for this application's own public client ID.
//! No passwords, borrowed application IDs, local callback server, or persisted tokens.
use crate::auth::{Account, AccountKind, Session};
use anyhow::{bail, Context, Result};
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Deserializer};
use serde_json::json;
use std::{
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const OAUTH: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0";

fn secret<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Zeroizing<String>, D::Error> {
    String::deserialize(d).map(Zeroizing::new)
}
fn secret_opt<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Zeroizing<String>>, D::Error> {
    Option::<String>::deserialize(d).map(|value| value.map(Zeroizing::new))
}

#[derive(Deserialize)]
struct DeviceResponse {
    #[serde(deserialize_with = "secret")]
    device_code: Zeroizing<String>,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}
fn default_interval() -> u64 {
    5
}

// Only the user-facing code is sent to the UI. Never derive Debug for token payloads.
pub struct DevicePrompt {
    pub code: String,
    pub url: String,
    pub expires_in: u64,
}

#[derive(Deserialize)]
struct OAuthToken {
    #[serde(deserialize_with = "secret")]
    access_token: Zeroizing<String>,
    #[serde(default, deserialize_with = "secret_opt")]
    refresh_token: Option<Zeroizing<String>>,
}
#[derive(Deserialize)]
struct OAuthError {
    error: String,
}
#[derive(Deserialize)]
struct XboxToken {
    #[serde(rename = "Token", deserialize_with = "secret")]
    token: Zeroizing<String>,
    #[serde(rename = "DisplayClaims")]
    claims: Claims,
}
#[derive(Deserialize)]
struct Claims {
    xui: Vec<UserHash>,
}
#[derive(Deserialize)]
struct UserHash {
    uhs: String,
}
#[derive(Deserialize)]
struct XboxError {
    #[serde(rename = "XErr")]
    xerr: Option<u64>,
}
#[derive(Deserialize)]
struct MinecraftToken {
    #[serde(deserialize_with = "secret")]
    access_token: Zeroizing<String>,
}
#[derive(Deserialize)]
struct Entitlements {
    items: Vec<Entitlement>,
}
#[derive(Deserialize)]
struct Entitlement {
    name: String,
}
#[derive(Deserialize)]
struct MinecraftProfile {
    id: String,
    name: String,
}

fn check_cancel(cancel: &Receiver<()>) -> Result<()> {
    match cancel.try_recv() {
        Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => bail!("Вход отменён"),
        Err(std::sync::mpsc::TryRecvError::Empty) => Ok(()),
    }
}

fn require_success(response: Response, stage: &str) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status.as_u16() == 403 {
        bail!("{stage}: доступ запрещён. Проверьте разрешения приложения и одобрение Client ID для Minecraft API.");
    }
    if status.as_u16() == 429 {
        bail!("{stage}: слишком много запросов. Повторите позже.");
    }
    bail!("{stage}: ошибка HTTP {}", status.as_u16());
}

fn xbox_error(code: Option<u64>) -> &'static str {
    match code {
        Some(2148916233) => "Создайте профиль Xbox на xbox.com и повторите вход.",
        Some(2148916238) => {
            "Для детского аккаунта требуется семейная группа Microsoft и разрешение родителя."
        }
        Some(2148916235) => "Xbox Live недоступен для региона аккаунта.",
        Some(2148916236 | 2148916237) => "Подтвердите возраст в настройках Xbox.",
        Some(2148916227) => "Аккаунт заблокирован в Xbox Live.",
        _ => "Xbox Live отклонил вход. Проверьте настройки аккаунта Xbox.",
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Poll {
    Pending,
    SlowDown,
    Denied,
    Expired,
    InvalidClient,
    Other,
}
fn poll_error(code: &str) -> Poll {
    match code {
        "authorization_pending" => Poll::Pending,
        "slow_down" => Poll::SlowDown,
        "authorization_declined" | "access_denied" => Poll::Denied,
        "expired_token" | "code_expired" => Poll::Expired,
        "invalid_client" | "unauthorized_client" | "invalid_scope" => Poll::InvalidClient,
        _ => Poll::Other,
    }
}

pub fn authenticate(
    client_id: &str,
    cancel: &Receiver<()>,
    prompt: impl Fn(DevicePrompt),
    progress: impl Fn(&str),
) -> Result<Account> {
    let client_id = uuid::Uuid::parse_str(client_id.trim())
        .context("Укажите Microsoft Client ID в настройках")?
        .to_string();
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("MLauncher/0.1.0")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?;
    check_cancel(cancel)?;
    let response = client
        .post(format!("{OAUTH}/devicecode"))
        .form(&[
            ("client_id", client_id.as_str()),
            ("scope", "XboxLive.signin offline_access"),
        ])
        .send()?;
    let device: DeviceResponse = require_success(response, "Microsoft: получение кода")?
        .json()
        .context("Некорректный ответ Microsoft")?;
    // Only Microsoft's public verification URLs may be opened by the UI.
    let verification = reqwest::Url::parse(&device.verification_uri)?;
    if verification.scheme() != "https"
        || !matches!(
            verification.host_str(),
            Some(
                "microsoft.com"
                    | "www.microsoft.com"
                    | "login.microsoftonline.com"
                    | "login.live.com"
            )
        )
    {
        bail!("Неожиданный адрес входа Microsoft");
    }
    let expires = Duration::from_secs(device.expires_in.min(3600));
    let deadline = Instant::now() + expires;
    let mut interval = Duration::from_secs(device.interval.max(5));
    check_cancel(cancel)?;
    prompt(DevicePrompt {
        code: device.user_code,
        url: device.verification_uri,
        expires_in: expires.as_secs(),
    });
    let oauth = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("Код Microsoft истёк. Начните вход заново.");
        }
        match cancel.recv_timeout(interval.min(remaining)) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => bail!("Вход отменён"),
        }
        if Instant::now() >= deadline {
            bail!("Код Microsoft истёк. Начните вход заново.");
        }
        let response = client
            .post(format!("{OAUTH}/token"))
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id.as_str()),
                ("device_code", device.device_code.as_str()),
            ])
            .send()?;
        check_cancel(cancel)?;
        if response.status().is_success() {
            break response
                .json::<OAuthToken>()
                .context("Microsoft не вернул токен")?;
        }
        if response.status().as_u16() == 429 {
            interval += Duration::from_secs(5);
            continue;
        }
        let error: OAuthError = response.json().context("Ошибка протокола Microsoft")?;
        match poll_error(&error.error) {
            Poll::Pending => {},
            Poll::SlowDown => interval += Duration::from_secs(5),
            Poll::Denied => bail!("Вход не разрешён пользователем"),
            Poll::Expired => bail!("Код Microsoft истёк. Начните вход заново."),
            Poll::InvalidClient => bail!("Проверьте Client ID, поддержку личных аккаунтов и public client/device flow в регистрации Microsoft."),
            Poll::Other => bail!("Microsoft отклонил авторизацию. Повторите вход."),
        }
    };
    let refresh = oauth.refresh_token.clone();
    complete_login(&client, &oauth.access_token, refresh, cancel, &progress)
}

/// Обмен Microsoft access token на игровой профиль Minecraft.
fn complete_login(
    client: &Client,
    microsoft_access: &Zeroizing<String>,
    refresh_token: Option<Zeroizing<String>>,
    cancel: &Receiver<()>,
    progress: &dyn Fn(&str),
) -> Result<Account> {
    progress("Xbox Live…");
    check_cancel(cancel)?;
    let xbox: XboxToken = require_success(client.post("https://user.auth.xboxlive.com/user/authenticate")
        .header("Accept", "application/json")
        .json(&json!({"Properties":{"AuthMethod":"RPS","SiteName":"user.auth.xboxlive.com","RpsTicket":format!("d={}", microsoft_access.as_str())},"RelyingParty":"http://auth.xboxlive.com","TokenType":"JWT"}))
        .send()?, "Xbox Live")?.json().context("Некорректный ответ Xbox Live")?;
    check_cancel(cancel)?;
    let response = client.post("https://xsts.auth.xboxlive.com/xsts/authorize").header("Accept", "application/json")
        .json(&json!({"Properties":{"SandboxId":"RETAIL","UserTokens":[&*xbox.token]},"RelyingParty":"rp://api.minecraftservices.com/","TokenType":"JWT"})).send()?;
    if !response.status().is_success() {
        let error = response.json::<XboxError>().ok();
        bail!("{}", xbox_error(error.and_then(|e| e.xerr)));
    }
    let xsts: XboxToken = response.json().context("Некорректный ответ XSTS")?;
    let hash = &xsts.claims.xui.first().context("Нет профиля Xbox")?.uhs;
    if xbox.claims.xui.first().map(|u| &u.uhs) != Some(hash) {
        bail!("Профили Xbox Live и XSTS не совпали");
    }
    check_cancel(cancel)?;
    progress("Проверка Minecraft…");
    let token: MinecraftToken = require_success(
        client
            .post("https://api.minecraftservices.com/authentication/login_with_xbox")
            .json(&json!({"identityToken":format!("XBL3.0 x={hash};{}", *xsts.token)}))
            .send()?,
        "Minecraft",
    )?
    .json()
    .context("Minecraft не вернул токен")?;
    check_cancel(cancel)?;
    let entitlements: Entitlements = require_success(
        client
            .get("https://api.minecraftservices.com/entitlements/mcstore")
            .bearer_auth(&*token.access_token)
            .send()?,
        "Лицензия Minecraft",
    )?
    .json()
    .context("Некорректный ответ о лицензии Minecraft")?;
    if !owns_java(&entitlements) {
        bail!("На аккаунте не найдена лицензия Minecraft Java Edition. Проверьте покупку или подписку Game Pass.");
    }
    check_cancel(cancel)?;
    let response = client
        .get("https://api.minecraftservices.com/minecraft/profile")
        .bearer_auth(&*token.access_token)
        .send()?;
    if response.status().as_u16() == 404 {
        bail!("Профиль Minecraft ещё не создан. Откройте официальный лаунчер или minecraft.net и выберите игровой ник.");
    }
    let profile: MinecraftProfile = require_success(response, "Профиль Minecraft")?
        .json()
        .context("Некорректный профиль Minecraft")?;
    check_cancel(cancel)?;
    if token.access_token.is_empty() {
        bail!("Minecraft вернул пустой токен");
    }
    let uuid = uuid::Uuid::parse_str(&profile.id)
        .context("Некорректный UUID Minecraft")?
        .simple()
        .to_string();
    Ok(Account {
        name: profile.name,
        uuid,
        kind: AccountKind::Microsoft,
        ely_skins: false,
        session: Some(Session {
            access_token: token.access_token,
            client_token: Zeroizing::new(uuid::Uuid::new_v4().to_string()),
            refresh_token,
        }),
    })
}

/// Обновляет сессию Microsoft по refresh token, без повторного входа в браузере.
pub fn refresh(
    client_id: &str,
    session: &Session,
    cancel: &Receiver<()>,
    progress: &dyn Fn(&str),
) -> Result<Account> {
    let client_id = uuid::Uuid::parse_str(client_id.trim())
        .context("Укажите Microsoft Client ID в настройках")?
        .to_string();
    let refresh_token = session
        .refresh_token
        .as_ref()
        .context("Нет refresh token Microsoft. Войдите заново.")?;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("MLauncher/0.1.0")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?;
    check_cancel(cancel)?;
    progress("Обновление сессии Microsoft…");
    let response = client
        .post(format!("{OAUTH}/token"))
        .form(&[
            ("client_id", client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("scope", "XboxLive.signin offline_access"),
        ])
        .send()?;
    if !response.status().is_success() {
        bail!("Сессия Microsoft истекла. Войдите повторно.");
    }
    let oauth: OAuthToken = response.json().context("Microsoft не вернул токен")?;
    let next_refresh = oauth
        .refresh_token
        .map(|token| Zeroizing::new(token.to_string()))
        .or_else(|| Some(Zeroizing::new(refresh_token.to_string())));
    complete_login(&client, &oauth.access_token, next_refresh, cancel, progress)
}

fn owns_java(entitlements: &Entitlements) -> bool {
    entitlements
        .items
        .iter()
        .any(|e| matches!(e.name.as_str(), "game_minecraft" | "product_minecraft"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handles_device_flow_responses() {
        assert_eq!(poll_error("authorization_pending"), Poll::Pending);
        assert_eq!(poll_error("slow_down"), Poll::SlowDown);
        assert_eq!(poll_error("access_denied"), Poll::Denied);
        assert_eq!(poll_error("expired_token"), Poll::Expired);
        assert_eq!(poll_error("invalid_client"), Poll::InvalidClient);
    }
    #[test]
    fn bedrock_is_not_a_java_license() {
        let bedrock: Entitlements =
            serde_json::from_str(r#"{"items":[{"name":"game_minecraft_bedrock"}]}"#).unwrap();
        assert!(!owns_java(&bedrock));
        let java: Entitlements =
            serde_json::from_str(r#"{"items":[{"name":"game_minecraft"}]}"#).unwrap();
        assert!(owns_java(&java));
    }
    #[test]
    fn cancellation_and_missing_client_fail_before_network() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(()).unwrap();
        assert!(check_cancel(&rx).is_err());
        assert!(authenticate("", &rx, |_| {}, |_| {}).is_err());
    }
}
