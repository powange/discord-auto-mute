//! Updates through the Tauri updater (latest.json on GitHub releases, signed with the
//! project's update key). Nothing is installed without the user's confirmation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dam_core::i18n::tr;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::host::{show_notification, Event, Host};
use crate::settings::Store;
use crate::state;

const CHECK_INTERVAL: f64 = 24.0 * 3600.0;
const APP: &str = "Discord Auto-Mute";

/// The update found by the last check, kept for "Install update".
#[derive(Default)]
pub struct PendingUpdate(pub Mutex<Option<Update>>);

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Tells the user once that an update was installed.
pub fn announce_new_version(app: &AppHandle) {
    let current = env!("CARGO_PKG_VERSION");
    let previous = state::get("last_run_version").and_then(|v| v.as_str().map(str::to_string));
    if previous.as_deref() == Some(current) {
        return;
    }
    if previous.is_some() {
        show_notification(
            app,
            &tr("Update installed", &[]),
            &tr(
                "{app} is now version {version}.",
                &[("app", APP), ("version", current)],
            ),
        );
    }
    state::set("last_run_version", json!(current));
}

/// Checks once a day when the setting allows it.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            let enabled = app.state::<Store>().get().check_updates;
            let last = state::get("last_update_check")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            if enabled && now() - last >= CHECK_INTERVAL {
                check(&app, false).await;
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

pub fn check_now(app: AppHandle) {
    tauri::async_runtime::spawn(async move { check(&app, true).await });
}

async fn check(app: &AppHandle, manual: bool) {
    let result = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    state::set("last_update_check", json!(now()));
    match result {
        Err(e) => {
            log::warn!("Update check failed: {e}");
            if manual {
                show_notification(
                    app,
                    &tr("Update check failed", &[]),
                    &tr("GitHub could not be reached. Try again later.", &[]),
                );
            }
        }
        Ok(None) => {
            set_available(app, None);
            if manual {
                show_notification(
                    app,
                    &tr("No update available", &[]),
                    &tr(
                        "You're running the latest version ({version}).",
                        &[("version", env!("CARGO_PKG_VERSION"))],
                    ),
                );
            }
        }
        Ok(Some(update)) => {
            let version = update.version.clone();
            let first_notice = state::get("notified_version")
                .as_ref()
                .and_then(Value::as_str)
                != Some(version.as_str());
            state::set("notified_version", json!(version));
            *app.state::<PendingUpdate>().0.lock().unwrap() = Some(update);
            set_available(app, Some(version.clone()));
            if manual {
                install(app.clone()); // the confirmation dialog is the answer
            } else if first_notice {
                let action = tr("Install update v{version}…", &[("version", &version)]);
                show_notification(
                    app,
                    &tr("Update available", &[]),
                    &tr(
                        "Version {version} is available. Choose “{action}” in the tray menu.",
                        &[("version", &version), ("action", &action)],
                    ),
                );
            }
        }
    }
}

fn set_available(app: &AppHandle, version: Option<String>) {
    app.state::<Host>().set_update(version);
    let tray_app = app.clone();
    let _ = app.run_on_main_thread(move || crate::tray::refresh(&tray_app));
}

/// Asks for confirmation, then downloads, installs and restarts.
pub fn install(app: AppHandle) {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return; // a dialog or a download is already there
    }
    tauri::async_runtime::spawn(async move {
        install_once(&app).await;
        RUNNING.store(false, Ordering::SeqCst);
    });
}

async fn install_once(app: &AppHandle) {
    let Some(update) = app.state::<PendingUpdate>().0.lock().unwrap().clone() else {
        return;
    };
    let notes: String = update
        .body
        .clone()
        .unwrap_or_default()
        .chars()
        .take(600)
        .collect();
    let mut question = tr(
        "{app} {version} is available (you have {current}).",
        &[
            ("app", APP),
            ("version", &update.version),
            ("current", env!("CARGO_PKG_VERSION")),
        ],
    );
    question.push_str("\n\n");
    if !notes.trim().is_empty() {
        question.push_str(notes.trim());
        question.push_str("\n\n");
    }
    question.push_str(&tr(
        "Install it now? The application closes for a few seconds and starts again on its own.",
        &[],
    ));
    let confirmed = app
        .dialog()
        .message(question)
        .title(tr("{app} update", &[("app", APP)]))
        .buttons(MessageDialogButtons::OkCancelCustom(
            tr("Install update", &[]),
            tr("Cancel", &[]),
        ))
        .blocking_show();
    if !confirmed {
        log::info!("Update to {} postponed by the user", update.version);
        return;
    }
    show_notification(
        app,
        &tr("Downloading update", &[]),
        &tr(
            "Downloading version {version}…",
            &[("version", &update.version)],
        ),
    );
    // The Windows installer closes the application as soon as it starts: give
    // the microphone back first, and keep it (quit pauses until the end).
    app.state::<Host>().quit();
    match update.download_and_install(|_, _| {}, || {}).await {
        Ok(()) => {
            log::info!("Update to {} installed, restarting", update.version);
            app.restart();
        }
        Err(e) => {
            log::error!("Update failed: {e}");
            app.state::<Host>().send(Event::Paused(false));
            let open_page = app
                .dialog()
                .message(format!(
                    "{}\n\n{e}\n\n{}",
                    tr("The update could not be installed:", &[]),
                    tr("Open the download page instead?", &[])
                ))
                .title(tr("{app} update", &[("app", APP)]))
                .buttons(MessageDialogButtons::YesNo)
                .blocking_show();
            if open_page {
                use tauri_plugin_opener::OpenerExt;
                let _ = app.opener().open_url(
                    format!("{}/releases/latest", crate::tray::REPO_URL),
                    None::<&str>,
                );
            }
        }
    }
}
