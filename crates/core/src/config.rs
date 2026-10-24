//! settings.json: language, mute method and profiles. Secrets (Discord client secret,
//! tokens) are kept apart, in secrets.dat.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::profile::{self, Binding, Preset, Profile};

pub const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MuteMethod {
    /// Press the shortcut configured in Discord.
    #[default]
    Shortcut,
    /// Discord's local RPC API, with the user's own Discord application.
    Rpc,
}

/// Which Discord keybind action the shortcut is bound to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutAction {
    /// "Toggle Mute": tapped once to mute, once to unmute.
    #[default]
    ToggleMute,
    /// "Push to Mute": held down while Discord has to stay muted.
    PushToMute,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcut {
    /// Key codes pressed together, modifiers first. A key no game uses (F13–F24)
    /// avoids side effects in the game.
    pub keys: Vec<String>,
    pub action: ShortcutAction,
}

impl Default for Shortcut {
    fn default() -> Self {
        Shortcut {
            keys: vec!["F13".into()],
            action: ShortcutAction::default(),
        }
    }
}

/// Not the one of GoXLR Discord Sync (9543): both can authorize at the same time.
pub const DEFAULT_OAUTH_PORT: u16 = 9544;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rpc {
    pub client_id: String,
    /// Port of the local callback for the browser authorization fallback.
    pub oauth_port: u16,
}

impl Default for Rpc {
    fn default() -> Self {
        Rpc {
            client_id: String::new(),
            oauth_port: DEFAULT_OAUTH_PORT,
        }
    }
}

impl Rpc {
    /// To add to the Discord application's OAuth2 redirects.
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.oauth_port)
    }
}

/// Settings, secrets and logs; DISCORD_AUTO_MUTE_HOME overrides it (tests, portable use).
pub fn data_dir() -> PathBuf {
    let dir = if let Ok(home) = std::env::var("DISCORD_AUTO_MUTE_HOME") {
        PathBuf::from(home)
    } else if cfg!(windows) {
        PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into()))
            .join("Discord Auto-Mute")
    } else {
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
        PathBuf::from(base).join("discord-auto-mute")
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn settings_file() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn log_dir() -> PathBuf {
    let dir = data_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    /// "auto" (system language) or a language code from i18n::LANGUAGES.
    pub language: String,
    pub method: MuteMethod,
    pub shortcut: Shortcut,
    pub rpc: Rpc,
    pub profiles: Vec<Profile>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: VERSION,
            language: "auto".into(),
            method: MuteMethod::default(),
            shortcut: Shortcut::default(),
            rpc: Rpc::default(),
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProfileError {
    UnknownProfile,
    UnknownPreset,
    EmptyName,
    NotFromPreset,
}

impl Settings {
    /// A missing or unreadable file gives the defaults (an unreadable one is kept aside).
    pub fn load(path: &Path) -> Settings {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
            Err(e) => {
                log::warn!("Cannot read {}: {e}", path.display());
                return Settings::default();
            }
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(mut settings) => {
                settings.normalize();
                settings
            }
            Err(e) => {
                log::warn!(
                    "{} is invalid ({e}), starting from the defaults",
                    path.display()
                );
                let _ = std::fs::rename(path, path.with_extension("json.invalid"));
                Settings::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).expect("serializable");
        discord_voice_rpc::fs::atomic_write(path, text.as_bytes())
    }

    /// Repairs what a hand-edited file may break: duplicate ids, empty names,
    /// several enabled profiles for the same game.
    pub fn normalize(&mut self) {
        self.version = VERSION;
        let mut seen = Vec::new();
        for i in 0..self.profiles.len() {
            if self.profiles[i].id.is_empty() || seen.contains(&self.profiles[i].id) {
                self.profiles[i].id = self.new_id("profile");
            }
            seen.push(self.profiles[i].id.clone());
            if self.profiles[i].name.trim().is_empty() {
                self.profiles[i].name = self.profiles[i].id.clone();
            }
            let current = &self.profiles[i];
            if current.enabled
                && self.profiles[..i]
                    .iter()
                    .any(|p| p.enabled && p.shares_executable(current))
            {
                self.profiles[i].enabled = false;
            }
        }
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    fn index(&self, id: &str) -> Result<usize, ProfileError> {
        self.profiles
            .iter()
            .position(|p| p.id == id)
            .ok_or(ProfileError::UnknownProfile)
    }

    fn new_id(&self, base: &str) -> String {
        (1..)
            .map(|n| format!("{base}-{n}"))
            .find(|id| self.profiles.iter().all(|p| &p.id != id))
            .unwrap()
    }

    /// Adds an enabled profile from a preset; returns its id.
    pub fn add_from_preset(&mut self, preset_id: &str) -> Result<String, ProfileError> {
        let preset = profile::preset(preset_id).ok_or(ProfileError::UnknownPreset)?;
        let id = self.new_id(&preset.id);
        self.profiles.push(preset.to_profile(id.clone()));
        self.set_enabled(&id, true)?;
        Ok(id)
    }

    /// Adds an empty, disabled profile: it has no game nor key yet.
    pub fn add_custom(&mut self, name: &str) -> Result<String, ProfileError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        let id = self.new_id("custom");
        self.profiles.push(Profile {
            id: id.clone(),
            name: name.into(),
            enabled: false,
            binding: Binding::default(),
            preset: None,
        });
        Ok(id)
    }

    /// Replaces a profile with the edited copy from the settings window.
    pub fn update(&mut self, edited: Profile) -> Result<(), ProfileError> {
        let i = self.index(&edited.id)?;
        let name = edited.name.trim().to_string();
        if name.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        let preset = self.profiles[i].preset.clone(); // not editable from the window
        let enabled = edited.enabled;
        self.profiles[i] = Profile {
            name,
            preset,
            enabled: false,
            ..edited
        };
        let id = self.profiles[i].id.clone();
        self.set_enabled(&id, enabled)
    }

    pub fn remove(&mut self, id: &str) -> Result<(), ProfileError> {
        let i = self.index(id)?;
        self.profiles.remove(i);
        Ok(())
    }

    /// Enabling a profile disables the other profiles of the same game.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), ProfileError> {
        let i = self.index(id)?;
        self.profiles[i].enabled = enabled;
        if enabled {
            let target = self.profiles[i].clone();
            for (j, other) in self.profiles.iter_mut().enumerate() {
                if j != i && other.shares_executable(&target) {
                    other.enabled = false;
                }
            }
        }
        Ok(())
    }

    /// Back to the preset's executables, keys and mode (the name stays).
    pub fn reset_to_preset(&mut self, id: &str) -> Result<(), ProfileError> {
        let i = self.index(id)?;
        let link = self.profiles[i]
            .preset
            .as_ref()
            .ok_or(ProfileError::NotFromPreset)?;
        let preset = profile::preset(&link.id).ok_or(ProfileError::UnknownPreset)?;
        let profile = &mut self.profiles[i];
        profile.binding = preset.binding.clone();
        profile.preset.as_mut().unwrap().base = preset.binding.clone();
        if profile.enabled {
            self.set_enabled(id, true)?;
        }
        Ok(())
    }

    /// After an update of the preset list: profiles the user did not modify follow
    /// their preset, modified ones are left alone. Returns whether anything changed.
    pub fn refresh_presets(&mut self, presets: &[Preset]) -> bool {
        let mut changed = Vec::new();
        for profile in &mut self.profiles {
            if profile.is_modified() {
                continue;
            }
            let Some(link) = profile.preset.as_mut() else {
                continue;
            };
            let Some(preset) = presets.iter().find(|p| p.id == link.id) else {
                continue;
            };
            if link.base != preset.binding {
                link.base = preset.binding.clone();
                profile.binding = preset.binding.clone();
                changed.push(profile.id.clone());
            }
        }
        // A preset that gained an executable may now overlap another enabled profile.
        for id in &changed {
            if self.profile(id).is_some_and(|p| p.enabled) {
                let _ = self.set_enabled(id, true);
            }
        }
        !changed.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{presets, Mode};

    #[test]
    fn roundtrip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
        let mut settings = Settings::default();
        settings.add_from_preset("sea-of-thieves").unwrap();
        settings.method = MuteMethod::Rpc;
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        // Fields added by later versions get their defaults.
        std::fs::write(&path, r#"{"language": "fr"}"#).unwrap();
        let partial = Settings::load(&path);
        assert_eq!(partial.language, "fr");
        assert_eq!(partial.shortcut, Shortcut::default());
    }

    #[test]
    fn invalid_file_is_kept_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        assert!(dir.path().join("settings.json.invalid").exists());
    }

    #[test]
    fn one_enabled_profile_per_game() {
        let mut s = Settings::default();
        let a = s.add_from_preset("sea-of-thieves").unwrap();
        let b = s.add_from_preset("sea-of-thieves").unwrap();
        assert_ne!(a, b);
        assert!(!s.profile(&a).unwrap().enabled);
        assert!(s.profile(&b).unwrap().enabled);
        s.set_enabled(&a, true).unwrap();
        assert!(!s.profile(&b).unwrap().enabled);
        let other = s.add_from_preset("rust").unwrap();
        assert!(s.profile(&a).unwrap().enabled && s.profile(&other).unwrap().enabled);
    }

    #[test]
    fn custom_profiles_need_a_name() {
        let mut s = Settings::default();
        assert_eq!(s.add_custom("  "), Err(ProfileError::EmptyName));
        let id = s.add_custom(" Mon jeu ").unwrap();
        let mut p = s.profile(&id).unwrap().clone();
        assert_eq!(p.name, "Mon jeu");
        assert!(!p.enabled);
        p.name = "".into();
        assert_eq!(s.update(p.clone()), Err(ProfileError::EmptyName));
        p.name = "Renamed".into();
        p.enabled = true;
        p.binding.executables = vec!["game.exe".into()];
        s.update(p).unwrap();
        assert!(s.profile(&id).unwrap().enabled);
        assert_eq!(s.reset_to_preset(&id), Err(ProfileError::NotFromPreset));
    }

    #[test]
    fn edits_keep_the_preset_link_and_reset_restores_it() {
        let mut s = Settings::default();
        let id = s.add_from_preset("counter-strike-2").unwrap();
        let mut p = s.profile(&id).unwrap().clone();
        p.binding.keys = vec!["Mouse5".into()];
        p.binding.mode = Mode::Toggle;
        p.preset = None; // the window cannot drop the link
        s.update(p).unwrap();
        let edited = s.profile(&id).unwrap();
        assert!(edited.is_modified());
        s.reset_to_preset(&id).unwrap();
        let reset = s.profile(&id).unwrap();
        assert!(!reset.is_modified());
        assert_eq!(reset.binding.keys, vec!["KeyK".to_string()]);
    }

    #[test]
    fn preset_updates_skip_modified_profiles() {
        let mut s = Settings::default();
        let untouched = s.add_from_preset("rust").unwrap();
        let modified = s.add_from_preset("dayz").unwrap();
        let mut p = s.profile(&modified).unwrap().clone();
        p.binding.keys = vec!["KeyB".into()];
        s.update(p).unwrap();

        let mut new_presets: Vec<Preset> = presets().to_vec();
        for preset in &mut new_presets {
            if preset.id == "rust" || preset.id == "dayz" {
                preset.binding.keys = vec!["KeyX".into()];
            }
        }
        assert!(s.refresh_presets(&new_presets));
        assert_eq!(
            s.profile(&untouched).unwrap().binding.keys,
            vec!["KeyX".to_string()]
        );
        assert!(!s.profile(&untouched).unwrap().is_modified());
        assert_eq!(
            s.profile(&modified).unwrap().binding.keys,
            vec!["KeyB".to_string()]
        );
        assert!(!s.refresh_presets(&new_presets));
    }

    #[test]
    fn normalize_repairs_hand_edits() {
        let mut s = Settings::default();
        let a = s.add_from_preset("rust").unwrap();
        let mut copy = s.profile(&a).unwrap().clone();
        copy.name = " ".into();
        s.profiles.push(copy);
        s.normalize();
        assert_ne!(s.profiles[0].id, s.profiles[1].id);
        assert!(!s.profiles[1].name.trim().is_empty());
        assert!(s.profiles[0].enabled && !s.profiles[1].enabled);
    }
}
