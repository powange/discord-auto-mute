//! "Application" mute method: a session with Discord's local RPC API (connect,
//! authorize, follow the microphone state), restarted when it ends.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use dam_core::config::{MuteMethod, Settings};
use dam_core::i18n::tr;
use discord_voice_rpc::auth::{AuthError, CallbackPages, DiscordTokens, TokenService, SCOPES};
use discord_voice_rpc::ipc::{DiscordConnection, DiscordIpc, EventSink, Rpc, RpcError};
use discord_voice_rpc::secrets::SecretsFile;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{mpsc, watch, Notify};

use crate::host::{secrets, show_notification, Event};

const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(30);
const BROWSER_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum DiscordState {
    /// The shortcut method is used: no connection.
    Shortcut,
    /// Client ID or client secret missing.
    NotConfigured,
    Connecting,
    Authorizing,
    /// The user has to authorize the application (reason).
    NeedsAuthorization(String),
    /// Discord not running or connection lost (reason); retried automatically.
    Unavailable(String),
    Connected,
}

enum End {
    /// Nothing changes until the user acts (settings, authorization).
    NeedsUser(DiscordState),
    Failed(String),
}

struct Inner {
    app: AppHandle,
    events: mpsc::UnboundedSender<Event>,
    settings: RefCell<Settings>,
    secrets: Rc<SecretsFile>,
    ipc: RefCell<Option<Rc<dyn DiscordConnection>>>,
    /// Restarts the session (settings changed, authorization asked).
    restart: Notify,
    /// The authorization prompt may be shown (not again after a refusal).
    may_prompt: Cell<bool>,
    force_refresh: Cell<bool>,
    auth_failures: Cell<u32>,
    cancel_browser: watch::Sender<bool>,
}

#[derive(Clone)]
pub struct Client(Rc<Inner>);

impl Client {
    /// Runs inside the engine thread's LocalSet.
    pub fn start(
        app: AppHandle,
        events: mpsc::UnboundedSender<Event>,
        settings: Settings,
    ) -> Client {
        let client = Client(Rc::new(Inner {
            app,
            events,
            settings: RefCell::new(settings),
            secrets: Rc::new(secrets()),
            ipc: RefCell::default(),
            restart: Notify::new(),
            may_prompt: Cell::new(true),
            force_refresh: Cell::new(false),
            auth_failures: Cell::new(0),
            cancel_browser: watch::channel(true).0,
        }));
        tokio::task::spawn_local(client.clone().run());
        client
    }

    pub fn configure(&self, settings: Settings, credentials_changed: bool) {
        let old = self.0.settings.replace(settings.clone());
        if old.method != settings.method || old.rpc != settings.rpc || credentials_changed {
            self.0.may_prompt.set(true);
            self.0.cancel_browser.send_replace(true);
            self.0.restart.notify_one();
        }
    }

    pub fn reauthorize(&self) {
        self.tokens().clear();
        self.0.may_prompt.set(true);
        self.0.cancel_browser.send_replace(true);
        self.0.restart.notify_one();
    }

    pub async fn set_mute(&self, mute: bool) {
        let Some(ipc) = self.0.ipc.borrow().clone() else {
            log::warn!("Discord not connected: microphone not changed");
            return;
        };
        if let Err(e) = Rpc(&*ipc).set_voice_settings(json!({ "mute": mute })).await {
            log::warn!("Cannot change the Discord microphone: {e}");
        }
    }

    fn state(&self, state: DiscordState) {
        let _ = self.0.events.send(Event::Discord(state));
    }

    fn tokens(&self) -> DiscordTokens {
        let opener = self.0.app.clone();
        let mut pages = CallbackPages::english("Discord Auto-Mute");
        for (field, text) in [
            (&mut pages.not_found, "Not found"),
            (&mut pages.invalid_title, "Invalid request"),
            (
                &mut pages.foreign_link,
                "This authorization link is not the one {app} opened.",
            ),
            (&mut pages.no_code, "No authorization code received."),
            (&mut pages.cancelled_title, "Authorization cancelled"),
            (&mut pages.cancelled_text, "You can close this window."),
            (&mut pages.success_title, "Authorization successful"),
            (
                &mut pages.success_text,
                "You can close this window and return to {app}.",
            ),
        ] {
            // {app} is left for CallbackPages to fill in.
            *field = tr(text, &[("app", "{app}")]);
        }
        DiscordTokens::new(
            self.0.secrets.clone(),
            Rc::new(move |url: &str| {
                if let Err(e) = opener.opener().open_url(url, None::<&str>) {
                    log::warn!("Cannot open the browser: {e}");
                }
            }),
        )
        .with_pages(pages)
    }

    async fn run(self) {
        let mut delay = RETRY_MIN;
        loop {
            if self.0.settings.borrow().method != MuteMethod::Rpc {
                self.state(DiscordState::Shortcut);
                self.0.restart.notified().await;
                continue;
            }
            self.state(DiscordState::Connecting);
            let ended = tokio::select! {
                ended = self.session() => Some(ended),
                _ = self.0.restart.notified() => None,
            };
            let was_connected = self.0.ipc.borrow_mut().take().is_some();
            let _ = self.0.events.send(Event::DiscordMuted(None));
            match ended {
                None => {
                    log::info!("Discord session restarted");
                    delay = RETRY_MIN;
                }
                Some(End::NeedsUser(state)) => {
                    log::warn!("Discord: {state:?}");
                    self.state(state);
                    self.0.restart.notified().await;
                    delay = RETRY_MIN;
                }
                Some(End::Failed(reason)) => {
                    if was_connected {
                        log::warn!("Discord connection lost: {reason}");
                        delay = RETRY_MIN;
                    } else {
                        log::info!("Discord not available: {reason} (retrying in {delay:?})");
                    }
                    self.state(DiscordState::Unavailable(reason));
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = self.0.restart.notified() => delay = RETRY_MIN,
                    }
                    delay = (delay * 2).min(RETRY_MAX);
                }
            }
        }
    }

    async fn session(&self) -> End {
        let settings = self.0.settings.borrow().clone();
        let secret = self.0.secrets.client_secret();
        let client_id = settings.rpc.client_id.trim().to_string();
        if client_id.is_empty() || secret.is_empty() {
            return End::NeedsUser(DiscordState::NotConfigured);
        }
        let events = self.0.events.clone();
        let sink: EventSink = Rc::new(move |evt: &str, data: Value| {
            if evt == "VOICE_SETTINGS_UPDATE" {
                let _ = events.send(Event::DiscordMuted(data["mute"].as_bool()));
            }
        });
        let ipc: Rc<dyn DiscordConnection> = match DiscordIpc::connect(&client_id, sink).await {
            Ok(ipc) => Rc::new(ipc),
            Err(e) => return End::Failed(e.to_string()),
        };
        let ended = self.session_on(&ipc, &settings, &client_id, &secret).await;
        ipc.close();
        ended
    }

    async fn session_on(
        &self,
        ipc: &Rc<dyn DiscordConnection>,
        settings: &Settings,
        client_id: &str,
        secret: &str,
    ) -> End {
        let tokens = self.tokens();
        let force = self.0.force_refresh.replace(false);
        let token = match tokens.access_token(client_id, secret, force).await {
            Ok(Some(token)) => token,
            Ok(None) => match self
                .authorize(&**ipc, &tokens, settings, client_id, secret)
                .await
            {
                Ok(token) => token,
                Err(end) => return end,
            },
            Err(e) => return End::Failed(e.to_string()),
        };
        let rpc = Rpc(&**ipc);
        match rpc.authenticate(&token).await {
            Ok(_) => self.0.auth_failures.set(0),
            Err(RpcError::Rpc { message, .. }) => {
                let failures = self.0.auth_failures.get() + 1;
                self.0.auth_failures.set(failures);
                self.0.force_refresh.set(true);
                if failures >= 2 {
                    log::warn!("Discord keeps rejecting the token, a new authorization is needed");
                    tokens.clear();
                }
                return End::Failed(format!("Discord rejected the access token: {message}"));
            }
            Err(e) => return End::Failed(e.to_string()),
        }
        let voice = match rpc.get_voice_settings().await {
            Ok(voice) => voice,
            Err(e) => return End::Failed(e.to_string()),
        };
        if let Err(e) = rpc.subscribe("VOICE_SETTINGS_UPDATE").await {
            return End::Failed(e.to_string());
        }
        let _ = self
            .0
            .events
            .send(Event::DiscordMuted(voice["mute"].as_bool()));
        // Only now: set_mute must not run before the session is authenticated.
        *self.0.ipc.borrow_mut() = Some(ipc.clone());
        log::info!("Connected to Discord");
        self.state(DiscordState::Connected);
        End::Failed(format!("closed: {}", ipc.wait_closed().await))
    }

    async fn authorize(
        &self,
        ipc: &dyn DiscordConnection,
        tokens: &DiscordTokens,
        settings: &Settings,
        client_id: &str,
        secret: &str,
    ) -> Result<String, End> {
        let needs = |reason: &str| End::NeedsUser(DiscordState::NeedsAuthorization(reason.into()));
        if !self.0.may_prompt.get() {
            return Err(needs(
                "Authorization required: use 'Authorize Discord' in the tray menu",
            ));
        }
        self.0.may_prompt.set(false);
        self.state(DiscordState::Authorizing);
        show_notification(
            &self.0.app,
            &tr("Discord authorization", &[]),
            &tr("Approve Discord Auto-Mute in the Discord window.", &[]),
        );
        let redirect = settings.rpc.redirect_uri();
        let retry_later = |reason: String| {
            self.0.may_prompt.set(true);
            End::Failed(reason)
        };
        match Rpc(ipc).authorize(client_id, &SCOPES).await {
            Ok(code) => {
                match tokens
                    .exchange(client_id, secret, &code, &[Some(redirect.clone()), None])
                    .await
                {
                    Ok(token) => return Ok(token),
                    Err(AuthError::Transient(e)) => return Err(retry_later(e)),
                    Err(AuthError::Refused(e)) => {
                        log::warn!("Code exchange failed ({e}), trying the browser")
                    }
                }
            }
            Err(RpcError::Closed(e) | RpcError::Unavailable(e)) => return Err(retry_later(e)),
            Err(RpcError::Timeout) => {
                return Err(needs(
                    "The authorization request in Discord was not answered",
                ))
            }
            Err(RpcError::Rpc { message, .. }) => {
                let text = message.to_lowercase();
                if ["denied", "cancel", "reject"]
                    .iter()
                    .any(|w| text.contains(w))
                {
                    return Err(needs("Authorization declined in Discord"));
                }
                log::warn!("In-app authorization failed ({message}), trying the browser");
            }
        }

        show_notification(
            &self.0.app,
            &tr("Discord authorization", &[]),
            &tr("Approve Discord Auto-Mute in your browser.", &[]),
        );
        self.0.cancel_browser.send_replace(false);
        let code = tokens
            .browser_authorize(
                client_id,
                &redirect,
                settings.rpc.oauth_port,
                BROWSER_TIMEOUT,
                self.0.cancel_browser.subscribe(),
            )
            .await;
        self.0.cancel_browser.send_replace(true);
        let result = match code {
            Ok(code) => {
                tokens
                    .exchange(client_id, secret, &code, &[Some(redirect)])
                    .await
            }
            Err(e) => Err(e),
        };
        match result {
            Ok(token) => Ok(token),
            Err(AuthError::Transient(e)) => Err(retry_later(e)),
            Err(AuthError::Refused(e)) => Err(needs(&format!("Discord authorization failed: {e}"))),
        }
    }
}
