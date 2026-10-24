//! The settings window (ui/settings.html) and the commands it calls. The settings
//! live in `Store`; every change is saved, then sent to the engine.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use dam_core::config::{
    log_dir, settings_file, MuteMethod, ProfileError, Settings, Shortcut, ShortcutAction,
};
use dam_core::engine::Status;
use dam_core::i18n::{self, tr};
use dam_core::keys;
use dam_core::profile::{presets, Preset, Profile};
use discord_voice_rpc::auth::{CredentialCheck, TokenStore};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

use crate::discord::DiscordState;
use crate::games::{self, InstalledGame};
use crate::host::{secrets, Event, Host};

/// The settings in use, and a revision the window compares to notice changes made
/// elsewhere (tray menu).
pub struct Store {
    settings: Mutex<Settings>,
    revision: AtomicU64,
}

impl Store {
    pub fn new(settings: Settings) -> Store {
        Store {
            settings: Mutex::new(settings),
            revision: AtomicU64::new(1),
        }
    }

    pub fn get(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }
}

/// The last scan of the installed games (None: not scanned yet).
#[derive(Default)]
pub struct Games(Mutex<Option<Vec<InstalledGame>>>);

async fn installed(app: &AppHandle, rescan: bool) -> Vec<InstalledGame> {
    if !rescan {
        if let Some(games) = app.state::<Games>().0.lock().unwrap().clone() {
            return games;
        }
    }
    let games = tauri::async_runtime::spawn_blocking(games::scan)
        .await
        .unwrap_or_default();
    *app.state::<Games>().0.lock().unwrap() = Some(games.clone());
    games
}

/// Steam and Epic games of this computer (scanned again on each call).
#[tauri::command]
pub async fn installed_games(app: AppHandle) -> Vec<games::GameView> {
    games::views(&installed(&app, true).await)
}

/// Profile id -> icon (data URL), for the profiles that have one.
#[tauri::command]
pub async fn profile_icons(app: AppHandle) -> HashMap<String, String> {
    let games = installed(&app, false).await;
    let profiles = app.state::<Store>().get().profiles;
    tauri::async_runtime::spawn_blocking(move || games::profile_icons(&profiles, &games))
        .await
        .unwrap_or_default()
}

/// How long the user has to click into Discord's keybind field.
const SEND_DELAY: Duration = Duration::from_secs(5);

pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    // Debug builds can open a given tab (screenshots in tests): DAM_SETTINGS_TAB=discord.
    let page = match std::env::var("DAM_SETTINGS_TAB") {
        Ok(tab) if cfg!(debug_assertions) => format!("settings.html#{tab}"),
        _ => "settings.html".to_string(),
    };
    let built = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App(page.into()))
        .title(tr("{app} – Settings", &[("app", "Discord Auto-Mute")]))
        .inner_size(820.0, 680.0)
        .min_inner_size(680.0, 560.0)
        .center()
        .build();
    if let Err(e) = built {
        log::error!("Cannot open the settings window: {e}");
    }
}

fn save_error(e: impl std::fmt::Display) -> String {
    format!("{}\n{e}", tr("Could not save the settings:", &[]))
}

/// Changes the settings under the lock (no lost update between two commands),
/// saves them and hands them to the engine. `change` returns whether the Discord
/// credentials changed.
fn modify(
    app: &AppHandle,
    change: impl FnOnce(&mut Settings) -> Result<bool, String>,
) -> Result<Settings, String> {
    let store = app.state::<Store>();
    let mut current = store.settings.lock().unwrap();
    let mut settings = current.clone();
    let credentials_changed = change(&mut settings)?;
    settings.save(&settings_file()).map_err(save_error)?;
    *current = settings.clone();
    store.revision.fetch_add(1, Ordering::SeqCst);
    drop(current);
    app.state::<Host>().send(Event::Settings {
        settings: settings.clone(),
        credentials_changed,
    });
    Ok(settings)
}

fn profile_error(e: ProfileError) -> String {
    match e {
        ProfileError::UnknownProfile => tr("This profile no longer exists.", &[]),
        ProfileError::UnknownPreset => tr("This preset no longer exists.", &[]),
        ProfileError::EmptyName => tr("The profile needs a name.", &[]),
        ProfileError::NotFromPreset => tr("This profile was not created from a preset.", &[]),
    }
}

pub fn set_profile_enabled(app: &AppHandle, id: &str, enabled: bool) -> Result<(), String> {
    modify(app, |settings| {
        settings.set_enabled(id, enabled).map_err(profile_error)?;
        Ok(false)
    })
    .map(drop)
}

#[derive(Serialize)]
pub struct SettingsData {
    settings: Settings,
    revision: u64,
    client_secret: String,
    redirect_uri: String,
    autostart: bool,
    platform: &'static str,
    version: &'static str,
    language: &'static str,
    languages: Vec<(&'static str, &'static str)>,
    /// English text -> translation (empty in English).
    texts: HashMap<String, String>,
    presets: &'static [Preset],
}

#[tauri::command]
pub fn get_settings(app: AppHandle, store: tauri::State<'_, Store>) -> SettingsData {
    let (settings, revision) = {
        let settings = store.settings.lock().unwrap();
        (settings.clone(), store.revision())
    };
    SettingsData {
        revision,
        redirect_uri: settings.rpc.redirect_uri(),
        settings,
        client_secret: secrets().client_secret(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        platform: if cfg!(windows) { "windows" } else { "linux" },
        version: env!("CARGO_PKG_VERSION"),
        language: i18n::current(),
        languages: i18n::languages(),
        texts: i18n::catalog(i18n::current()),
        presets: presets(),
    }
}

/// Live state, polled by the window.
#[derive(Serialize)]
pub struct StateData {
    /// Changes when the settings change (the window reloads them).
    revision: u64,
    status: Status,
    discord: DiscordState,
    input_error: Option<String>,
    recent: Vec<String>,
}

#[tauri::command]
pub fn get_state(host: tauri::State<'_, Host>, store: tauri::State<'_, Store>) -> StateData {
    let shared = host.snapshot();
    StateData {
        revision: store.revision(),
        status: shared.status,
        discord: shared.discord,
        input_error: shared.input_error,
        recent: shared.recent,
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProfileAction {
    AddPreset {
        preset: String,
    },
    AddCustom {
        name: String,
    },
    AddGame {
        name: String,
        executables: Vec<String>,
    },
    Update {
        profile: Profile,
    },
    Remove {
        id: String,
    },
    SetEnabled {
        id: String,
        enabled: bool,
    },
    Reset {
        id: String,
    },
}

/// Cleans what the window sends: trimmed, deduplicated, known keys only.
fn clean(mut profile: Profile) -> Result<Profile, String> {
    let mut executables: Vec<String> = Vec::new();
    for exe in &profile.binding.executables {
        let exe = exe.trim();
        if !exe.is_empty() && !executables.iter().any(|e| e.eq_ignore_ascii_case(exe)) {
            executables.push(exe.to_string());
        }
    }
    profile.binding.executables = executables;
    if let Some(key) = profile.binding.keys.iter().find(|k| !keys::is_known(k)) {
        return Err(tr("This key is not supported: {key}", &[("key", key)]));
    }
    let mut seen = Vec::new();
    profile.binding.keys.retain(|k| {
        let first = !seen.contains(k);
        seen.push(k.clone());
        first
    });
    Ok(profile)
}

#[tauri::command]
pub fn profile_action(app: AppHandle, action: ProfileAction) -> Result<Settings, String> {
    modify(&app, |settings| {
        match action {
            ProfileAction::AddPreset { preset } => settings.add_from_preset(&preset).map(drop),
            ProfileAction::AddCustom { name } => settings.add_custom(&name).map(drop),
            ProfileAction::AddGame { name, executables } => {
                settings.add_game(&name, &executables).map(drop)
            }
            ProfileAction::Update { profile } => settings.update(clean(profile)?),
            ProfileAction::Remove { id } => settings.remove(&id),
            ProfileAction::SetEnabled { id, enabled } => settings.set_enabled(&id, enabled),
            ProfileAction::Reset { id } => settings.reset_to_preset(&id),
        }
        .map_err(profile_error)?;
        Ok(false)
    })
}

#[derive(Deserialize)]
pub struct GeneralPayload {
    language: String,
    method: MuteMethod,
    shortcut: Shortcut,
    client_id: String,
    client_secret: String,
    /// u32: an out-of-range port gets a readable error rather than a parse error.
    oauth_port: u32,
    autostart: bool,
    check_updates: bool,
}

/// The keys the application may type for Discord.
fn check_shortcut(codes: &[String], action: ShortcutAction) -> Result<(), String> {
    if codes.is_empty() {
        return Err(tr("Choose the shortcut set in Discord.", &[]));
    }
    if let Some(key) = codes
        .iter()
        .find(|k| !keys::is_known(k) || keys::is_mouse(k))
    {
        return Err(tr("This key cannot be sent: {key}", &[("key", key)]));
    }
    // Held for the whole sentence: a held Ctrl, Shift or Alt would reach the game.
    if action == ShortcutAction::PushToMute && codes.iter().any(|k| keys::is_modifier(k)) {
        return Err(tr(
            "With Push to Mute, use a single key without Ctrl, Shift or Alt (F13 to F24).",
            &[],
        ));
    }
    Ok(())
}

#[tauri::command]
pub fn save_general(app: AppHandle, payload: GeneralPayload) -> Result<(), String> {
    let shortcut = payload.shortcut;
    if payload.method == MuteMethod::Shortcut {
        check_shortcut(&shortcut.keys, shortcut.action)?;
    }
    let client_id = payload.client_id.trim().to_string();
    if !client_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(tr(
            "The Client ID is a number: copy it from the Discord Developer Portal.",
            &[],
        ));
    }
    let Some(oauth_port) = u16::try_from(payload.oauth_port)
        .ok()
        .filter(|p| *p >= 1024)
    else {
        return Err(tr("The port must be between 1024 and 65535.", &[]));
    };
    modify(&app, |settings| {
        let store = secrets();
        let secret = payload.client_secret.trim();
        let mut credentials_changed = false;
        if client_id != settings.rpc.client_id {
            store.set_token(None).map_err(save_error)?; // tokens of the previous application
        }
        if secret != store.client_secret() {
            store.set_client_secret(secret).map_err(save_error)?;
            credentials_changed = true;
        }
        if payload.language != settings.language {
            log::info!(
                "Interface language: {}",
                i18n::set_language(&payload.language)
            );
        }
        settings.language = payload.language;
        settings.method = payload.method;
        settings.shortcut = shortcut;
        settings.rpc.client_id = client_id;
        settings.rpc.oauth_port = oauth_port;
        settings.check_updates = payload.check_updates;
        log::info!("Settings saved");
        Ok(credentials_changed)
    })?;
    // Last: the settings are saved even if the system refuses this one.
    let launcher = app.autolaunch();
    if payload.autostart != launcher.is_enabled().unwrap_or(false) {
        let result = if payload.autostart {
            launcher.enable()
        } else {
            launcher.disable()
        };
        result.map_err(save_error)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn check_credentials(client_id: String, client_secret: String) -> (bool, String) {
    let check = discord_voice_rpc::auth::check_credentials_at(
        discord_voice_rpc::auth::API,
        client_id.trim(),
        client_secret.trim(),
    )
    .await;
    let text = match &check {
        CredentialCheck::Valid => tr("The Client ID and Client Secret are valid.", &[]),
        CredentialCheck::Rejected => tr(
            "Discord rejected this Client ID / Client Secret pair. Copy both values again.",
            &[],
        ),
        CredentialCheck::Unreachable(error) => tr(
            "Cannot reach Discord ({error}). Check your internet connection.",
            &[("error", error)],
        ),
        CredentialCheck::Http(status) => tr(
            "Discord answered HTTP {status}. Try again in a moment.",
            &[("status", &status.to_string())],
        ),
    };
    (check.is_valid(), text)
}

/// Types the shortcut after a delay, so the user can record it in Discord's
/// keybind settings (F13 and the like are missing from most keyboards). Returns once
/// it is sent.
#[tauri::command]
pub async fn send_shortcut(codes: Vec<String>) -> Result<(), String> {
    check_shortcut(&codes, ShortcutAction::ToggleMute)?;
    tokio::time::sleep(SEND_DELAY).await;
    crate::platform::send_keys(&codes, true)?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    crate::platform::send_keys(&codes, false)
}

/// Mutes Discord for three seconds with the chosen method.
#[tauri::command]
pub fn test_mute(host: tauri::State<'_, Host>) {
    host.send(Event::Test);
}

#[tauri::command]
pub fn check_updates(app: AppHandle) {
    crate::updates::check_now(app);
}

#[tauri::command]
pub fn reauthorize(host: tauri::State<'_, Host>) {
    host.send(Event::Reauthorize);
}

#[tauri::command]
pub fn open_external(app: AppHandle, url: String) {
    if url.starts_with("https://") {
        let _ = app.opener().open_url(url, None::<&str>);
    }
}

#[tauri::command]
pub fn open_log_folder(app: AppHandle) {
    let _ = app
        .opener()
        .open_path(log_dir().to_string_lossy(), None::<&str>);
}
