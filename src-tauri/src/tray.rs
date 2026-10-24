//! System tray icon and menu.

use dam_core::config::{log_dir, MuteMethod};
use dam_core::i18n::tr;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

use crate::discord::DiscordState;
use crate::host::{Event, Host, Shared};

const TRAY: &str = "main";
pub const REPO_URL: &str = "https://github.com/powange/discord-auto-mute";

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Paused,
    Warning,
    Muting,
    Ready,
    Idle,
}

/// A problem that stops the application from working, already translated.
fn problem(shared: &Shared) -> Option<String> {
    if let Some(e) = &shared.input_error {
        return Some(if e == crate::platform::NO_PERMISSION {
            tr(
                "Keys cannot be read: add your user to the 'input' group",
                &[],
            )
        } else {
            tr("Keys cannot be read: {error}", &[("error", e)])
        });
    }
    if shared.method != MuteMethod::Rpc {
        return None;
    }
    match &shared.discord {
        DiscordState::Connected | DiscordState::Shortcut => None,
        DiscordState::Connecting => Some(tr("Connecting to Discord…", &[])),
        DiscordState::Authorizing => Some(tr("Waiting for the Discord authorization", &[])),
        DiscordState::NotConfigured => {
            Some(tr("Discord application not configured (Settings)", &[]))
        }
        DiscordState::NeedsAuthorization(reason) => Some(tr(reason, &[])),
        DiscordState::Unavailable(_) => Some(tr("Discord is not connected", &[])),
    }
}

fn look(shared: &Shared) -> Look {
    let status = &shared.status;
    if status.paused {
        Look::Paused
    } else if status.muting {
        Look::Muting
    } else if problem(shared).is_some() {
        Look::Warning
    } else if status.profile.is_some() {
        Look::Ready
    } else {
        Look::Idle
    }
}

fn icon(look: Look) -> Image<'static> {
    let bytes: &[u8] = match look {
        Look::Paused => include_bytes!("../icons/tray/paused.png"),
        Look::Warning => include_bytes!("../icons/tray/warning.png"),
        Look::Muting => include_bytes!("../icons/tray/muting.png"),
        Look::Ready => include_bytes!("../icons/tray/ready.png"),
        Look::Idle => include_bytes!("../icons/tray/idle.png"),
    };
    Image::from_bytes(bytes).expect("bundled PNG")
}

/// First line of the menu and of the tooltip.
fn summary(shared: &Shared) -> String {
    let status = &shared.status;
    if status.paused {
        return tr("Paused", &[]);
    }
    match &status.profile {
        None => tr("No game in the foreground", &[]),
        Some((_, name)) if status.muting => {
            tr("{name}: Discord microphone muted", &[("name", name)])
        }
        Some((_, name)) => tr("Profile: {name}", &[("name", name)]),
    }
}

fn autostart_label() -> String {
    if cfg!(windows) {
        tr("Start with Windows", &[])
    } else {
        tr("Start on login", &[])
    }
}

fn build_menu(app: &AppHandle, shared: &Shared) -> tauri::Result<Menu<tauri::Wry>> {
    let item = |id: &str, text: String, enabled: bool| {
        MenuItem::with_id(app, id, text, enabled, None::<&str>)
    };
    let check = |id: &str, text: String, checked: bool| {
        CheckMenuItem::with_id(app, id, text, true, checked, None::<&str>)
    };
    let menu = Menu::new(app)?;
    menu.append(&item("summary", summary(shared), false)?)?;
    if let Some(problem) = problem(shared) {
        menu.append(&item("problem", problem, false)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    if shared.status.profile.is_some() {
        menu.append(&item(
            "disable_profile",
            tr("Disable this profile", &[]),
            true,
        )?)?;
    }
    menu.append(&check("pause", tr("Pause", &[]), shared.status.paused)?)?;
    menu.append(&item("settings", tr("Settings…", &[]), true)?)?;
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    menu.append(&check("autostart", autostart_label(), autostart)?)?;
    if shared.method == MuteMethod::Rpc {
        menu.append(&item(
            "reauthorize",
            tr("Authorize Discord again", &[]),
            true,
        )?)?;
    }
    menu.append(&item("log", tr("Open log", &[]), true)?)?;
    menu.append(&item(
        "about",
        tr(
            "About (v{version})",
            &[("version", env!("CARGO_PKG_VERSION"))],
        ),
        true,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item("quit", tr("Quit", &[]), true)?)?;
    Ok(menu)
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let shared = app.state::<Host>().snapshot();
    TrayIconBuilder::with_id(TRAY)
        .icon(icon(look(&shared)))
        .tooltip("Discord Auto-Mute")
        .menu(&build_menu(app, &shared)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::settings::open(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Icon, tooltip and menu from the current state (main thread).
pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY) else {
        return;
    };
    let shared = app.state::<Host>().snapshot();
    let mut tooltip = format!("Discord Auto-Mute - {}", summary(&shared));
    if let Some(problem) = problem(&shared) {
        tooltip = format!("{tooltip}\n{problem}");
    }
    // 127 characters: Windows tooltip limit.
    let tooltip: String = tooltip.chars().take(127).collect();
    // Rebuild only what changed: the menu flickers on some desktops.
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    let signature = format!(
        "{tooltip}|{:?}|{autostart}|{}",
        shared.method,
        dam_core::i18n::current()
    );
    static LAST: std::sync::Mutex<(String, Option<u8>)> =
        std::sync::Mutex::new((String::new(), None));
    let mut last = LAST.lock().unwrap();
    let look = look(&shared);
    if last.1 != Some(look as u8) {
        let _ = tray.set_icon(Some(icon(look)));
        last.1 = Some(look as u8);
    }
    if last.0 == signature {
        return;
    }
    let _ = tray.set_tooltip(Some(tooltip));
    match build_menu(app, &shared) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
            last.0 = signature;
        }
        Err(e) => log::warn!("Cannot rebuild the tray menu: {e}"),
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    let host = app.state::<Host>();
    match id {
        "disable_profile" => {
            if let Some((profile, _)) = host.snapshot().status.profile {
                if let Err(e) = crate::settings::set_profile_enabled(app, &profile, false) {
                    log::error!("Cannot disable the profile: {e}");
                }
            }
        }
        "pause" => host.send(Event::Paused(!host.snapshot().status.paused)),
        "settings" => crate::settings::open(app),
        "autostart" => {
            let launcher = app.autolaunch();
            let result = if launcher.is_enabled().unwrap_or(false) {
                launcher.disable()
            } else {
                launcher.enable()
            };
            if let Err(e) = result {
                log::error!("Cannot change autostart: {e}");
            }
            refresh(app);
        }
        "reauthorize" => host.send(Event::Reauthorize),
        "log" => {
            let _ = app
                .opener()
                .open_path(log_dir().join("app.log").to_string_lossy(), None::<&str>);
        }
        "about" => {
            let _ = app.opener().open_url(REPO_URL, None::<&str>);
        }
        "quit" => {
            log::info!("Quit requested");
            host.quit();
            app.exit(0);
        }
        _ => {}
    }
}
