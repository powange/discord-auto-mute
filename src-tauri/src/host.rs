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
use crate::platform;

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
    pub discord: DiscordState,
    /// Why keys cannot be read (platform::linux::NO_PERMISSION or a system error).
    pub input_error: Option<String>,
    /// Executables recently in the foreground, newest first (to create profiles).
    pub recent: Vec<String>,
    /// Version of an available update (tray menu entry).
    pub update: Option<String>,
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

    /// Unmutes Discord if the application muted it; waits at most one second.
    pub fn quit(&self) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.send(Event::Quit(tx));
        let _ = rx.recv_timeout(Duration::from_secs(1));
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
        discord: DiscordState::Shortcut,
        input_error,
        recent: Vec::new(),
        update: None,
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
async fn apply(settings: &Settings, discord: &discord::Client, mute: bool) {
    log::info!("{} Discord", if mute { "Muting" } else { "Unmuting" });
    match settings.method {
        MuteMethod::Rpc => discord.set_mute(mute).await,
        MuteMethod::Shortcut => {
            let keys = &settings.shortcut.keys;
            let result = match settings.shortcut.action {
                ShortcutAction::PushToMute => platform::send_keys(keys, mute),
                ShortcutAction::ToggleMute => platform::send_keys(keys, true).and_then(|()| {
                    std::thread::sleep(TAP);
                    platform::send_keys(keys, false)
                }),
            };
            if let Err(e) = result {
                log::error!("Cannot send the Discord shortcut: {e}");
            }
        }
    }
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
    let mut engine = Engine::new(settings.profiles.clone());
    let discord = discord::Client::start(app.clone(), events, settings.clone());
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
                if new.method != settings.method || new.shortcut != settings.shortcut {
                    // Undo with the old method before switching.
                    if let Some(mute) = engine.release() {
                        apply(&settings, &discord, mute).await;
                    }
                    if new.method == MuteMethod::Shortcut {
                        engine.handle(Input::DiscordMuted(None));
                    }
                }
                discord.configure(new.clone(), credentials_changed);
                settings = new;
                shared.lock().unwrap().method = settings.method;
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
                if let Some(mute) = engine.release() {
                    apply(&settings, &discord, mute).await;
                }
                let _ = done.send(());
                None
            }
        };
        if let Some(mute) = change {
            apply(&settings, &discord, mute).await;
        }
        let status = engine.status();
        let changed = {
            let mut s = shared.lock().unwrap();
            let changed = s.status != status;
            s.status = status;
            changed
        };
        if changed || !is_key {
            let tray_app = app.clone();
            let _ = app.run_on_main_thread(move || crate::tray::refresh(&tray_app));
        }
    }
}
