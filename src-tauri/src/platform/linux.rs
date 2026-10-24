//! Linux: keys from evdev (/dev/input, works under X11 and Wayland but needs the
//! `input` group), the foreground window from X11 (games, Proton included, run under
//! X11 or XWayland), synthetic keys with XTest.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dam_core::keys;
use evdev::{Device, EventType, KeyCode};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, Window, KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::{KeySink, NO_PERMISSION};

const RESCAN: Duration = Duration::from_secs(3);

fn is_keyboard_or_mouse(device: &Device) -> bool {
    device
        .supported_keys()
        .is_some_and(|keys| keys.contains(KeyCode::KEY_A) || keys.contains(KeyCode::BTN_MIDDLE))
}

fn read_device(
    mut device: Device,
    path: PathBuf,
    sink: Arc<KeySink>,
    open: Arc<Mutex<HashSet<PathBuf>>>,
) {
    loop {
        match device.fetch_events() {
            Ok(events) => {
                for event in events {
                    // value: 1 pressed, 0 released, 2 auto-repeat (ignored: the engine
                    // sees a key that is already down).
                    if event.event_type() != EventType::KEY || event.value() > 1 {
                        continue;
                    }
                    if let Some(key) = keys::from_evdev(event.code()) {
                        sink(key, event.value() == 1);
                    }
                }
            }
            Err(e) => {
                log::info!("Input device {} closed: {e}", path.display());
                open.lock().unwrap().remove(&path);
                return;
            }
        }
    }
}

/// Opens the devices not opened yet; Err(NO_PERMISSION) when none can be read.
fn scan(sink: &Arc<KeySink>, open: &Arc<Mutex<HashSet<PathBuf>>>) -> Result<(), String> {
    let entries = std::fs::read_dir("/dev/input").map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "no input device (/dev/input is missing)".to_string()
        } else {
            format!("/dev/input: {e}")
        }
    })?;
    let mut denied = false;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_event = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("event"));
        if !is_event || open.lock().unwrap().contains(&path) {
            continue;
        }
        let device = match Device::open(&path) {
            Ok(device) => device,
            Err(e) => {
                denied |= e.kind() == std::io::ErrorKind::PermissionDenied;
                continue;
            }
        };
        if !is_keyboard_or_mouse(&device) {
            continue;
        }
        log::info!(
            "Listening to {} ({})",
            path.display(),
            device.name().unwrap_or("unnamed")
        );
        open.lock().unwrap().insert(path.clone());
        let (sink, open) = (sink.clone(), open.clone());
        std::thread::Builder::new()
            .name("input-device".into())
            .spawn(move || read_device(device, path, sink, open))
            .map_err(|e| e.to_string())?;
    }
    if denied && open.lock().unwrap().is_empty() {
        return Err(NO_PERMISSION.into());
    }
    Ok(())
}

/// Reads every keyboard and mouse; devices plugged in later are picked up.
pub fn listen(sink: KeySink) -> Result<(), String> {
    let sink = Arc::new(sink);
    let open: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    let first = scan(&sink, &open);
    std::thread::Builder::new()
        .name("input-scan".into())
        .spawn(move || loop {
            std::thread::sleep(RESCAN);
            let _ = scan(&sink, &open);
        })
        .map_err(|e| e.to_string())?;
    first
}

struct X11 {
    conn: RustConnection,
    root: Window,
    active_window: u32,
    wm_pid: u32,
}

impl X11 {
    fn connect() -> Result<X11, Box<dyn std::error::Error>> {
        let (conn, screen) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen].root;
        let active_window = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")?
            .reply()?
            .atom;
        let wm_pid = conn.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
        Ok(X11 {
            conn,
            root,
            active_window,
            wm_pid,
        })
    }

    fn active_pid(&self) -> Result<Option<u32>, Box<dyn std::error::Error>> {
        let reply = self
            .conn
            .get_property(false, self.root, self.active_window, AtomEnum::WINDOW, 0, 1)?
            .reply()?;
        let Some(window) = reply
            .value32()
            .and_then(|mut v| v.next())
            .filter(|w| *w != 0)
        else {
            return Ok(None);
        };
        let reply = self
            .conn
            .get_property(false, window, self.wm_pid, AtomEnum::CARDINAL, 0, 1)?
            .reply()?;
        Ok(reply.value32().and_then(|mut v| v.next()))
    }

    fn send(&self, keycode: u8, down: bool) -> Result<(), Box<dyn std::error::Error>> {
        let kind = if down {
            KEY_PRESS_EVENT
        } else {
            KEY_RELEASE_EVENT
        };
        self.conn
            .xtest_fake_input(kind, keycode, 0, self.root, 0, 0, 0)?;
        Ok(())
    }
}

/// One connection, reopened after an error (X server restarted…).
fn with_x11<T>(
    work: impl FnOnce(&X11) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, String> {
    static X: Mutex<Option<X11>> = Mutex::new(None);
    let mut guard = X.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(X11::connect().map_err(|e| format!("X11 unavailable: {e}"))?);
    }
    let result = work(guard.as_ref().unwrap());
    if result.is_err() {
        *guard = None;
    }
    result.map_err(|e| e.to_string())
}

/// Wine names its processes after the Windows executable: argv[0] is "C:\…\game.exe".
fn process_name(pid: u32) -> Option<String> {
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let argv0 = cmdline.split(|b| *b == 0).next().unwrap_or_default();
    let argv0 = String::from_utf8_lossy(argv0);
    let name = super::file_name(argv0.trim());
    if !name.is_empty() {
        return Some(name.to_string());
    }
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim().to_string()).filter(|c| !c.is_empty())
}

pub fn foreground_executable() -> Option<String> {
    match with_x11(|x| x.active_pid()) {
        Ok(pid) => process_name(pid?),
        Err(e) => {
            log::debug!("Foreground window unknown: {e}");
            None
        }
    }
}

/// Presses (in order) or releases (in reverse order) the keys.
pub fn send_keys(codes: &[String], down: bool) -> Result<(), String> {
    let mut keycodes = codes
        .iter()
        .map(|code| {
            keys::to_evdev(code)
                .and_then(|c| u8::try_from(c + 8).ok())
                .ok_or_else(|| format!("cannot send {code}"))
        })
        .collect::<Result<Vec<u8>, String>>()?;
    if !down {
        keycodes.reverse();
    }
    with_x11(|x| {
        for keycode in &keycodes {
            x.send(*keycode, down)?;
        }
        x.conn.flush()?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_name() {
        let name = process_name(std::process::id()).unwrap();
        assert!(name.starts_with("discord_auto_mute"), "{name}");
    }
}
