//! state.json: what the application remembers between runs, apart from the settings
//! (update checks, whether Discord was left muted by a crash).

use std::sync::Mutex;

use dam_core::config::state_file;
use serde_json::{Map, Value};

/// The engine and the update checks write from different threads.
static LOCK: Mutex<()> = Mutex::new(());

fn read_unlocked() -> Map<String, Value> {
    std::fs::read_to_string(state_file())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn get(key: &str) -> Option<Value> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read_unlocked().remove(key)
}

pub fn set(key: &str, value: Value) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = read_unlocked();
    if state.get(key) == Some(&value) {
        return;
    }
    state.insert(key.into(), value);
    let text = serde_json::to_string_pretty(&state).expect("serializable");
    if let Err(e) = discord_voice_rpc::fs::atomic_write(&state_file(), text.as_bytes()) {
        log::warn!("Cannot save state.json: {e}");
    }
}
