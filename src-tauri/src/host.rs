//! Runs the engine on its own thread (single-threaded tokio runtime + LocalSet, like
//! the Discord client expects) and shares its state with the tray and the settings.
//! Key presses, the foreground process and Discord all arrive as `Event`s.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dam_core::config::{data_dir, MuteMethod, Settings, ShortcutAction};
use dam_core::engine::{Engine, Input, Status};
use discord_voice_rpc::secrets::SecretsFile;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use tokio::sync::mpsc;

use crate::discord::{self, DiscordState};
use crate::{platform, state};

const FOREGROUND_POLL: Duration = Duration::from_millis(250);
const RECENT_MAX: usize = 15;
/// Foreground programs never offered as a game executable.
const NOT_GAMES: &[&str] = &[
    "explorer.exe",
    "SnippingTool.exe",
    "ScreenClippingHost.exe",
    "SearchHost.exe",
    "SearchApp.exe",
    "StartMenuExperienceHost.exe",
    "ShellExperienceHost.exe",
    "ShellHost.exe",
    "ApplicationFrameHost.exe",
    "LockApp.exe",
    "Taskmgr.exe",
    "SystemSettings.exe",
    "Discord.exe",
    "DiscordPTB.exe",
    "DiscordCanary.exe",
    "Discord",
    "Vesktop",
];
/// How long a "Toggle Mute" shortcut stays down: long enough for Discord to see it.
const TAP: Duration = Duration::from_millis(40);
/// state.json key: Discord was muted by the application when it last stopped.
const MUTED_KEY: &str = "discord_muted_by_app";
/// How long quitting waits for Discord to be unmuted.
const QUIT_WAIT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub enum Event {
    Key(&'static str, bool),
    Foreground(Option<String>),
    Settings {
        settings: Settings,
        credentials_changed: bool,
    },
    Paused(bool),
    Reauthorize,
    /// Mutes Discord for a few seconds, to check the setup.
    Test,
    TestEnd,
    DiscordMuted(Option<bool>),
    Discord(DiscordState),
    /// Gives the microphone back, then answers.
    Quit(std::sync::mpsc::Sender<()>),
}

/// What the tray and the settings window read.
#[derive(Clone)]
pub struct Shared {
    pub status: Status,
    pub method: MuteMethod,
    /// The user confirmed the shortcut exists in Discord.
    pub shortcut_set: bool,
    pub discord: DiscordState,
    /// Why keys cannot be read (platform::linux::NO_PERMISSION or a system error).
    pub input_error: Option<String>,
    /// Executables recently in the foreground, newest first (to create profiles).
    pub recent: Vec<String>,
    /// Version of an available update (tray menu entry).
    pub update: Option<String>,
    /// Id and name of the last profile that was active (the tray menu opens with
    /// another program in the foreground).
    pub last_profile: Option<(String, String)>,
}

pub struct Host {
    tx: mpsc::UnboundedSender<Event>,
    shared: Arc<Mutex<Shared>>,
}

impl Host {
    pub fn send(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    pub fn snapshot(&self) -> Shared {
        self.shared.lock().unwrap().clone()
    }

    pub fn set_update(&self, version: Option<String>) {
        self.shared.lock().unwrap().update = version;
    }

    /// Unmutes Discord if the application muted it, and keeps it unmuted (paused).
    /// Waits at most QUIT_WAIT; a crash is recovered at the next start.
    pub fn quit(&self) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.send(Event::Quit(tx));
        let _ = rx.recv_timeout(QUIT_WAIT);
    }
}

pub fn secrets() -> SecretsFile {
    SecretsFile::new(data_dir(), b"discord-auto-mute")
}

pub fn show_notification(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::info!("Notification not shown ({e}): {title} - {body}");
    }
}

pub fn start(app: AppHandle, settings: Settings) -> Host {
    let (tx, rx) = mpsc::unbounded_channel();
    let key_tx = tx.clone();
    // Debug builds can simulate a game in the foreground, without reading the keys
    // (screenshots): DAM_DEMO=SoTGame.exe.
    let demo = std::env::var("DAM_DEMO")
        .ok()
        .filter(|_| cfg!(debug_assertions));
    let input_error = if demo.is_some() {
        None
    } else {
        platform::listen(Box::new(move |key, down| {
            let _ = key_tx.send(Event::Key(key, down));
        }))
        .err()
    };
    if let Some(e) = &input_error {
        log::error!("Keys cannot be read: {e}");
    }
    let shared = Arc::new(Mutex::new(Shared {
        status: Status::default(),
        method: settings.method,
        shortcut_set: settings.shortcut.set_in_discord,
        discord: DiscordState::Shortcut,
        input_error,
        recent: Vec::new(),
        update: None,
        last_profile: None,
    }));

    let foreground_tx = tx.clone();
    std::thread::Builder::new()
        .name("foreground".into())
        .spawn(move || {
            let mut last = None;
            loop {
                let now = match &demo {
                    Some(exe) => Some(exe.clone()),
                    None => platform::foreground_executable(),
                };
                if now != last {
                    log::debug!("Foreground: {now:?}");
                    if foreground_tx.send(Event::Foreground(now.clone())).is_err() {
                        return;
                    }
                    last = now;
                }
                std::thread::sleep(FOREGROUND_POLL);
            }
        })
        .expect("foreground thread");

    let thread_shared = shared.clone();
    let events = tx.clone();
    std::thread::Builder::new()
        .name("engine".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("engine runtime");
            let local = tokio::task::LocalSet::new();
            local.block_on(&runtime, run(app, settings, events, rx, thread_shared));
        })
        .expect("engine thread");
    Host { tx, shared }
}

/// Changes the Discord microphone with the configured method.
async fn apply(settings: &Settings, discord: &discord::Client, mute: bool) -> Result<(), String> {
    log::info!("{} Discord", if mute { "Muting" } else { "Unmuting" });
    let result = match settings.method {
        MuteMethod::Rpc => discord.set_mute(mute).await,
        MuteMethod::Shortcut => {
            let keys = &settings.shortcut.keys;
            match settings.shortcut.action {
                ShortcutAction::PushToMute => platform::send_keys(keys, mute),
                ShortcutAction::ToggleMute => match platform::send_keys(keys, true) {
                    Ok(()) => {
                        tokio::time::sleep(TAP).await;
                        platform::send_keys(keys, false)
                    }
                    Err(e) => Err(e),
                },
            }
        }
    };
    if let Err(e) = &result {
        log::error!(
            "Cannot {} Discord: {e}",
            if mute { "mute" } else { "unmute" }
        );
    }
    result
}

/// Applies a change and reports a failure to the engine (an unmute is retried later).
async fn apply_change(
    engine: &mut Engine,
    settings: &Settings,
    discord: &discord::Client,
    mut change: Option<bool>,
) {
    while let Some(mute) = change {
        change = match apply(settings, discord, mute).await {
            Ok(()) => None,
            Err(_) => engine.handle(Input::Failed(mute)),
        };
    }
}

/// After a crash with Discord muted: the RPC method unmutes once Discord answers;
/// a Push to Mute shortcut is released now. A Toggle Mute shortcut cannot be undone
/// blindly: the real state is unknown.
async fn recover(settings: &Settings, engine: Engine) -> Engine {
    log::warn!("The application stopped while Discord was muted: unmuting it");
    match (settings.method, settings.shortcut.action) {
        (MuteMethod::Rpc, _) => return engine,
        (MuteMethod::Shortcut, ShortcutAction::PushToMute) => {
            let _ = platform::send_keys(&settings.shortcut.keys, false);
        }
        (MuteMethod::Shortcut, ShortcutAction::ToggleMute) => {}
    }
    state::set(MUTED_KEY, false.into());
    Engine::new(settings.profiles.clone())
}

async fn run(
    app: AppHandle,
    mut settings: Settings,
    events: mpsc::UnboundedSender<Event>,
    mut rx: mpsc::UnboundedReceiver<Event>,
    shared: Arc<Mutex<Shared>>,
) {
    let own_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let crashed = state::get(MUTED_KEY).and_then(|v| v.as_bool()) == Some(true);
    let mut engine = if crashed {
        recover(&settings, Engine::recovering(settings.profiles.clone())).await
    } else {
        Engine::new(settings.profiles.clone())
    };
    let discord = discord::Client::start(app.clone(), events.clone(), settings.clone());
    let mut testing = false;
    while let Some(event) = rx.recv().await {
        // Keys of other games are most events: they change nothing the tray shows.
        let is_key = matches!(event, Event::Key(..));
        let change = match event {
            Event::Key(key, down) => engine.handle(Input::Key {
                code: key.into(),
                down,
            }),
            Event::Foreground(exe) => {
                let suggested = |e: &&String| {
                    Some(*e) != own_exe.as_ref()
                        && !NOT_GAMES.iter().any(|n| n.eq_ignore_ascii_case(e))
                };
                if let Some(exe) = exe.as_ref().filter(suggested) {
                    let mut s = shared.lock().unwrap();
                    s.recent.retain(|r| r != exe);
                    s.recent.insert(0, exe.clone());
                    s.recent.truncate(RECENT_MAX);
                }
                engine.handle(Input::Foreground(exe))
            }
            Event::Settings {
                settings: new,
                credentials_changed,
            } => {
                let shortcut_changed = (&new.shortcut.keys, new.shortcut.action)
                    != (&settings.shortcut.keys, settings.shortcut.action);
                if new.method != settings.method || shortcut_changed {
                    // Undo with the old method before switching.
                    let change = engine.release();
                    apply_change(&mut engine, &settings, &discord, change).await;
                    if new.method == MuteMethod::Shortcut {
                        engine.handle(Input::DiscordMuted(None));
                    }
                }
                discord.configure(new.clone(), credentials_changed);
                settings = new;
                {
                    let mut s = shared.lock().unwrap();
                    s.method = settings.method;
                    s.shortcut_set = settings.shortcut.set_in_discord;
                }
                engine.handle(Input::Profiles(settings.profiles.clone()))
            }
            Event::Paused(paused) => {
                log::info!("{}", if paused { "Paused" } else { "Resumed" });
                engine.handle(Input::Paused(paused))
            }
            Event::Reauthorize => {
                discord.reauthorize();
                None
            }
            Event::Test => {
                if !testing && !engine.status().muting {
                    testing = true;
                    if apply(&settings, &discord, true).await.is_ok() {
                        let events = events.clone();
                        tokio::task::spawn_local(async move {
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            let _ = events.send(Event::TestEnd);
                        });
                    } else {
                        testing = false;
                    }
                }
                None
            }
            Event::TestEnd => {
                testing = false;
                let _ = apply(&settings, &discord, false).await;
                None
            }
            Event::DiscordMuted(muted) => {
                if settings.method == MuteMethod::Rpc {
                    engine.handle(Input::DiscordMuted(muted))
                } else {
                    None
                }
            }
            Event::Discord(state) => {
                shared.lock().unwrap().discord = state;
                None
            }
            Event::Quit(done) => {
                // Paused: nothing mutes again until the process ends (installer…).
                engine.handle(Input::Paused(true));
                let change = engine.release();
                apply_change(&mut engine, &settings, &discord, change).await;
                if !engine.status().muting {
                    state::set(MUTED_KEY, false.into());
                }
                let _ = done.send(());
                None
            }
        };
        apply_change(&mut engine, &settings, &discord, change).await;
        let status = engine.status();
        let changed = {
            let mut s = shared.lock().unwrap();
            let changed = s.status != status;
            if changed && s.status.muting != status.muting {
                state::set(MUTED_KEY, status.muting.into());
            }
            if status.profile.is_some() {
                s.last_profile = status.profile.clone();
            }
            s.status = status;
            changed
        };
        if changed || !is_key {
            let tray_app = app.clone();
            let _ = app.run_on_main_thread(move || crate::tray::refresh(&tray_app));
        }
    }
}
