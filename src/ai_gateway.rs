//! Public defaults contain no credentials. Developer credentials stay in Windows DPAPI storage.
use anyhow::{Context, Result};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path, time::Duration};
use zeroize::{Zeroize, Zeroizing};

pub const DEFAULTS_URL: &str =
    "https://raw.githubusercontent.com/MixiYT/MLauncher/main/config/ai-defaults.json";
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    pub revision: u64,
    pub enabled: bool,
    pub endpoint: String,
    pub model: String,
}
impl Defaults {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.model.len() <= 160 && self.endpoint.len() <= 1024,
            "Слишком длинные настройки ИИ"
        );
        if self.enabled || !self.endpoint.trim().is_empty() {
            let url = reqwest::Url::parse(&self.endpoint)?;
            anyhow::ensure!(
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none(),
                "Для общего ИИ нужен HTTPS URL без ключей и параметров"
            );
            anyhow::ensure!(!self.model.trim().is_empty(), "Укажите модель");
        }
        Ok(())
    }
}
pub fn fetch() -> Result<Defaults> {
    let response = client()?
        .get(DEFAULTS_URL)
        .header("Cache-Control", "no-cache")
        .send()?
        .error_for_status()?;
    let mut bytes = Vec::new();
    response.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 64 * 1024, "Слишком большой конфиг общего ИИ");
    let defaults: Defaults = serde_json::from_slice(&bytes)?;
    defaults.validate()?;
    Ok(defaults)
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("MLauncher AI configuration")
        .build()?)
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Developer {
    #[serde(default)]
    access_hash: String,
    #[serde(default)]
    access_salt: String,
    #[serde(skip)]
    unlocked: bool,
    #[serde(skip)]
    access_input: String,
    #[serde(skip)]
    access_confirm: String,
    #[serde(skip)]
    retry_after: Option<std::time::Instant>,
    pub account: String,
    pub worker: String,
    pub token: String,
    pub keys: Vec<String>,
    pub defaults: Defaults,
    #[serde(skip)]
    pub status: String,
    #[serde(skip)]
    pub busy: bool,
}
impl Default for Developer {
    fn default() -> Self {
        Self {
            access_hash: String::new(),
            access_salt: String::new(),
            unlocked: false,
            access_input: String::new(),
            access_confirm: String::new(),
            retry_after: None,
            account: String::new(),
            worker: "mlauncher-ai".into(),
            token: String::new(),
            keys: vec![String::new(); 3],
            defaults: Defaults {
                revision: 1,
                enabled: false,
                endpoint: String::new(),
                model: "openrouter/free".into(),
            },
            status: String::new(),
            busy: false,
        }
    }
}
impl Drop for Developer {
    fn drop(&mut self) {
        self.access_input.zeroize();
        self.access_confirm.zeroize();
        self.token.zeroize();
        for key in &mut self.keys {
            key.zeroize();
        }
    }
}
impl Developer {
    pub fn lock(&mut self) {
        self.unlocked = false;
        self.access_input.zeroize();
        self.access_confirm.zeroize();
    }
    fn verify_code(&self, code: &str) -> bool {
        let actual = access_hash(&self.access_salt, code);
        !self.access_hash.is_empty()
            && actual
                .bytes()
                .zip(self.access_hash.bytes())
                .fold(0u8, |diff, (a, b)| diff | (a ^ b))
                == 0
            && actual.len() == self.access_hash.len()
    }
    pub fn load(root: &Path) -> Self {
        crate::secrets::load_developer(root)
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, root: &Path) -> Result<()> {
        let bytes = Zeroizing::new(serde_json::to_vec(self)?);
        crate::secrets::save_developer(root, &bytes)
    }
    pub fn upload(&self) -> Result<()> {
        anyhow::ensure!(self.unlocked, "Управление API-ключами заблокировано");
        anyhow::ensure!(
            self.account.len() == 32 && self.account.chars().all(|c| c.is_ascii_hexdigit()),
            "Account ID должен содержать 32 шестнадцатеричных символа"
        );
        anyhow::ensure!(
            !self.worker.is_empty()
                && self.worker.len() <= 63
                && self
                    .worker
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "Некорректное имя Worker"
        );
        anyhow::ensure!(
            !self.token.trim().is_empty(),
            "Укажите Cloudflare API token"
        );
        let keys: Vec<_> = self
            .keys
            .iter()
            .map(|k| k.trim())
            .filter(|k| !k.is_empty())
            .collect();
        anyhow::ensure!(
            !keys.is_empty() && keys.len() <= 32 && keys.iter().all(|k| k.len() <= 1024),
            "Добавьте от 1 до 32 ключей (до 1024 символов каждый)"
        );
        let text = Zeroizing::new(serde_json::to_string(&keys)?);
        let url = format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/workers/scripts/{}/secrets",
            self.account, self.worker
        );
        let response = client()?
            .put(url)
            .bearer_auth(self.token.trim())
            .json(
                &serde_json::json!({"name":"AI_KEYS", "type":"secret_text", "text":text.as_str()}),
            )
            .send()
            .context("Не удалось связаться с Cloudflare")?;
        let status = response.status();
        // Never echo Cloudflare/provider response bodies: they may contain sensitive input.
        anyhow::ensure!(
            status.is_success(),
            "Cloudflare вернул HTTP {}. Проверьте Account ID, Worker и права токена.",
            status.as_u16()
        );
        let mut bytes = Vec::new();
        response.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            value["success"] == true,
            "Cloudflare не подтвердил сохранение Secret"
        );
        Ok(())
    }
    /// True when the owner explicitly chooses to upload the private key pool.
    pub fn ui(&mut self, ui: &mut egui::Ui, root: &Path) -> bool {
        ui.separator();
        ui.strong("Разработчик · Cloudflare Worker");
        if !self.unlocked {
            let setup = self.access_hash.is_empty();
            ui.label(if setup {
                "Задайте код доступа к управлению ключами на этом компьютере (от 8 символов)"
            } else {
                "Введите код доступа к управлению API-ключами"
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.access_input)
                    .password(true)
                    .hint_text("Код доступа"),
            );
            if setup {
                ui.add(
                    egui::TextEdit::singleline(&mut self.access_confirm)
                        .password(true)
                        .hint_text("Повторите код"),
                );
            }
            let ready = self
                .retry_after
                .is_none_or(|t| std::time::Instant::now() >= t);
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new(if setup {
                        "Установить код"
                    } else {
                        "Разблокировать"
                    }),
                )
                .clicked()
            {
                if setup {
                    if self.access_input.chars().count() < 8
                        || self.access_input != self.access_confirm
                    {
                        self.status = "Нужен код от 8 символов и совпадающее повторение".into();
                    } else {
                        self.access_salt = uuid::Uuid::new_v4().to_string();
                        self.access_hash = access_hash(&self.access_salt, &self.access_input);
                        match self.save(root) {
                            Ok(()) => {
                                self.unlocked = true;
                                self.status.clear();
                            }
                            Err(e) => {
                                self.access_hash.clear();
                                self.status = format!("{e:#}");
                            }
                        }
                    }
                } else if self.verify_code(&self.access_input) {
                    self.unlocked = true;
                    self.status.clear();
                } else {
                    self.status = "Неверный код. Повторите через 3 секунды".into();
                    self.retry_after = Some(std::time::Instant::now() + Duration::from_secs(3));
                }
                self.access_input.zeroize();
                self.access_confirm.zeroize();
            }
            if !ready {
                ui.ctx().request_repaint_after(Duration::from_secs(1));
            }
            if !self.status.is_empty() {
                ui.label(&self.status);
            }
            ui.small("Код защищает локальные настройки. Общий пул остаётся на сервере и не загружается пользователям.");
            return false;
        }
        if ui.button("Заблокировать управление ключами").clicked() {
            self.unlocked = false;
            return false;
        }

        ui.label("Account ID");
        ui.text_edit_singleline(&mut self.account);
        ui.label("Имя Worker");
        ui.text_edit_singleline(&mut self.worker);
        ui.label("Cloudflare API token · Workers Scripts: Edit");
        ui.add(egui::TextEdit::singleline(&mut self.token).password(true));
        ui.label("Пул ключей OpenRouter (хранится в Secret AI_KEYS на Worker)");
        let mut remove = None;
        for (i, key) in self.keys.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Ключ {}", i + 1));
                ui.add(
                    egui::TextEdit::singleline(key)
                        .password(true)
                        .desired_width(180.),
                );
                if ui.small_button("Убрать").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.keys[i].zeroize();
            self.keys.remove(i);
        }
        if ui
            .add_enabled(self.keys.len() < 32, egui::Button::new("+ API-ключ"))
            .clicked()
        {
            self.keys.push(String::new());
        }
        if ui.button("Сохранить локально (DPAPI)").clicked() {
            self.status = match self.save(root) {
                Ok(()) => "Сохранено только на этом компьютере".into(),
                Err(e) => format!("{e:#}"),
            };
        }
        let upload = ui
            .add_enabled(
                !self.busy,
                egui::Button::new("Отправить пул ключей в Cloudflare Secrets"),
            )
            .clicked();
        ui.small("Ключи передаются только в ваш аккаунт Cloudflare. Они не попадают в GitHub и клиентские обновления. Worker сначала нужно развернуть по docs/AI-GATEWAY.md.");
        ui.separator();
        ui.label("Публичный микропатч (без ключей)");
        ui.checkbox(&mut self.defaults.enabled, "Включить общий ИИ");
        ui.label("HTTPS адрес Worker с /v1/chat/completions");
        ui.text_edit_singleline(&mut self.defaults.endpoint);
        ui.label("Модель");
        ui.text_edit_singleline(&mut self.defaults.model);
        ui.horizontal(|ui| {
            ui.label("Ревизия");
            ui.add(egui::DragValue::new(&mut self.defaults.revision).range(1..=u64::MAX));
        });
        if ui.button("Экспортировать ai-defaults.json…").clicked() {
            self.status = match self.defaults.validate().and_then(|_| { if let Some(path)=rfd::FileDialog::new().set_file_name("ai-defaults.json").save_file() { crate::downloads::atomic_write(&path, &serde_json::to_vec_pretty(&self.defaults)?)?; } Ok(()) }) { Ok(())=>"Опубликуйте файл как config/ai-defaults.json в main на GitHub. Ключей в нём нет.".into(), Err(e)=>format!("{e:#}") };
        }
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
        upload
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_patch_rejects_secrets_and_bad_destinations() {
        assert!(serde_json::from_str::<Defaults>(
            r#"{"revision":1,"enabled":false,"endpoint":"","model":"","keys":["secret"]}"#
        )
        .is_err());
        let mut d = Defaults {
            revision: 1,
            enabled: true,
            endpoint: "https://gateway.example/v1/chat/completions".into(),
            model: "openrouter/free".into(),
        };
        assert!(d.validate().is_ok());
        for url in [
            "http://host/api",
            "https://user:key@host/api",
            "https://host/api?key=secret",
        ] {
            d.endpoint = url.into();
            assert!(d.validate().is_err());
        }
    }
    #[test]
    fn developer_secrets_are_encrypted_and_public_export_contains_none() {
        let root = tempfile::tempdir().unwrap();
        let mut d = Developer::default();
        d.token = "CF-SECRET".into();
        d.keys = vec!["AI-SECRET".into()];
        d.save(root.path()).unwrap();
        let data = std::fs::read(root.path().join("developer-keys.dat")).unwrap();
        assert!(!String::from_utf8_lossy(&data).contains("AI-SECRET"));
        assert_eq!(Developer::load(root.path()).keys[0], "AI-SECRET");
        let public = serde_json::to_string(&d.defaults).unwrap();
        assert!(!public.contains("SECRET"));
    }
}

fn access_hash(salt: &str, code: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(salt.as_bytes());
    hash.update(code.as_bytes());
    let mut bytes = hash.finalize().to_vec();
    for _ in 0..100_000 {
        let mut h = Sha256::new();
        h.update(&bytes);
        h.update(salt.as_bytes());
        h.update(code.as_bytes());
        bytes = h.finalize().to_vec();
    }
    let result = bytes.iter().map(|b| format!("{b:02x}")).collect();
    bytes.zeroize();
    result
}

#[cfg(test)]
mod access_tests {
    use super::*;
    #[test]
    fn access_is_required_and_never_persisted_unlocked() {
        let root = tempfile::tempdir().unwrap();
        let mut d = Developer::default();
        assert!(d
            .upload()
            .unwrap_err()
            .to_string()
            .contains("заблокировано"));
        d.access_salt = "random-salt".into();
        d.access_hash = access_hash(&d.access_salt, "correct-code");
        assert!(d.verify_code("correct-code"));
        assert!(!d.verify_code("incorrect-code"));
        d.unlocked = true;
        d.access_input = "never-persist-this".into();
        d.save(root.path()).unwrap();
        let loaded = Developer::load(root.path());
        assert!(!loaded.unlocked);
        assert!(loaded.access_input.is_empty());
        assert!(loaded.verify_code("correct-code"));
        d.lock();
        assert!(d.upload().is_err());
    }
}
