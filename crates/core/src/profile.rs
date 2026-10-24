//! Game profiles and the built-in presets they can start from.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// How the game's voice key drives the Discord microphone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Discord is muted while a key is held down.
    #[default]
    Hold,
    /// Each press mutes or unmutes Discord.
    Toggle,
}

/// What a preset provides and what decides whether a profile was modified.
/// Keys are physical key codes, named like `KeyboardEvent.code` ("KeyK", "AltLeft",
/// "CapsLock"…), plus "MouseMiddle", "Mouse4" and "Mouse5".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// Process file names, compared case-insensitively; `*` matches any text.
    pub executables: Vec<String>,
    pub keys: Vec<String>,
    pub mode: Mode,
}

/// The preset a profile was created from, with the binding it had then.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetLink {
    pub id: String,
    pub base: Binding,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(flatten)]
    pub binding: Binding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<PresetLink>,
}

impl Profile {
    /// True when the user changed what the preset set (the name does not count).
    pub fn is_modified(&self) -> bool {
        self.preset
            .as_ref()
            .is_some_and(|link| link.base != self.binding)
    }

    pub fn matches(&self, executable: &str) -> bool {
        self.binding
            .executables
            .iter()
            .any(|pattern| wildcard_match(pattern, executable))
    }

    pub fn shares_executable(&self, other: &Profile) -> bool {
        self.binding.executables.iter().any(|a| {
            other
                .binding
                .executables
                .iter()
                .any(|b| a.eq_ignore_ascii_case(b))
        })
    }
}

/// Case-insensitive match where `*` stands for any text, including none.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let (pattern, text) = (pattern.to_lowercase(), text.to_lowercase());
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub binding: Binding,
    /// False when the default key still has to be checked in the game.
    pub key_verified: bool,
}

impl Preset {
    pub fn to_profile(&self, id: String) -> Profile {
        Profile {
            id,
            name: self.name.clone(),
            enabled: true,
            binding: self.binding.clone(),
            preset: Some(PresetLink {
                id: self.id.clone(),
                base: self.binding.clone(),
            }),
        }
    }
}

pub fn presets() -> &'static [Preset] {
    static PRESETS: OnceLock<Vec<Preset>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        serde_json::from_str(include_str!("../presets.json")).expect("valid presets.json")
    })
}

pub fn preset(id: &str) -> Option<&'static Preset> {
    presets().iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard_match("cs2.exe", "CS2.EXE"));
        assert!(!wildcard_match("cs2.exe", "cs2.exe.bak"));
        assert!(wildcard_match(
            "FiveM_*GTAProcess.exe",
            "FiveM_b3258_GTAProcess.exe"
        ));
        assert!(wildcard_match(
            "FiveM_*GTAProcess.exe",
            "FiveM_GTAProcess.exe"
        ));
        assert!(!wildcard_match("FiveM_*GTAProcess.exe", "FiveM.exe"));
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("a*b*c", "aXbYc"));
        assert!(!wildcard_match("ab*ba", "aba")); // prefix and suffix must not overlap
    }

    #[test]
    fn presets_are_consistent() {
        let all = presets();
        assert_eq!(all.len(), 50);
        let mut ids: Vec<&str> = all.iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "duplicate preset id");
        for p in all {
            assert!(!p.name.trim().is_empty(), "{}", p.id);
            assert!(
                !p.binding.executables.is_empty(),
                "{} has no executable",
                p.id
            );
            assert!(!p.key_verified || !p.binding.keys.is_empty(), "{}", p.id);
            for key in &p.binding.keys {
                assert!(crate::keys::is_known(key), "{}: unknown key {key}", p.id);
            }
        }
    }

    #[test]
    fn modified_ignores_the_name() {
        let mut profile = preset("sea-of-thieves").unwrap().to_profile("p1".into());
        assert!(!profile.is_modified());
        profile.name = "SoT".into();
        profile.enabled = false;
        assert!(!profile.is_modified());
        profile.binding.mode = Mode::Toggle;
        assert!(profile.is_modified());
    }
}
