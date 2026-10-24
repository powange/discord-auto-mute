//! Discord Auto-Mute: tray application (Tauri 2) around the dam-core engine.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod discord;
mod host;
mod platform;
mod settings;
mod state;
mod tray;
mod updates;

use dam_core::config::{log_dir, settings_file, Settings};
use dam_core::i18n;
use dam_core::profile::presets;
use tauri::{Manager, RunEvent};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

fn main() {
    let app = tauri::Builder::default()
        // First plugin: a second launch opens the settings of the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            settings::open(app)
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::Folder {
                        path: log_dir(),
                        file_name: Some("app".into()),
                    }),
                    Target::new(TargetKind::Stdout),
                ])
                .level(log::LevelFilter::Info)
                .max_file_size(1_000_000)
                .rotation_strategy(RotationStrategy::KeepSome(3))
                .timezone_strategy(TimezoneStrategy::UseLocal)
                .build(),
        )
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::PendingUpdate::default())
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::get_state,
            settings::profile_action,
            settings::save_general,
            settings::check_credentials,
            settings::reauthorize,
            settings::check_updates,
            settings::send_shortcut,
            settings::test_mute,
            settings::open_external,
            settings::open_log_folder,
        ])
        // Only the first instance gets here: a second launch must not touch the files.
        .setup(|app| {
            log::info!("Discord Auto-Mute {} starting", env!("CARGO_PKG_VERSION"));
            let mut settings = Settings::load(&settings_file());
            i18n::set_language(&settings.language);
            if settings.refresh_presets(presets()) {
                // Not fatal: the profiles follow their presets in memory anyway.
                let _ = settings.save(&settings_file());
            }
            app.manage(settings::Store::new(settings.clone()));
            let handle = app.handle().clone();
            app.manage(host::start(handle.clone(), settings.clone()));
            tray::create(&handle)?;
            updates::announce_new_version(&handle);
            updates::start(handle.clone());
            if settings.profiles.is_empty() {
                settings::open(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Discord Auto-Mute could not start");

    app.run(|app, event| match event {
        // Closing the settings window must not quit the tray application.
        RunEvent::ExitRequested {
            api, code: None, ..
        } => api.prevent_exit(),
        // Never leave Discord muted behind; a crash is recovered at the next start.
        RunEvent::Exit => {
            if let Some(host) = app.try_state::<host::Host>() {
                host.quit();
            }
        }
        _ => {}
    });
}
