//! Mute rules, without any I/O: the application feeds key presses, the foreground
//! process and the Discord state in, and applies the mute changes that come out.
//!
//! - A profile only applies while its game is in the foreground.
//! - Hold: Discord is muted while one of the profile's keys is down.
//!   Toggle: each press (not auto-repeat) flips it.
//! - Leaving the game, pausing or changing the profile restores Discord.
//! - The engine only unmutes what it muted: if Discord was already muted, or the
//!   user unmutes it by hand, it leaves the microphone alone.

use serde::Serialize;
use std::collections::HashSet;

use crate::profile::{Mode, Profile};

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Key {
        code: String,
        down: bool,
    },
    /// File name of the foreground process, None when unknown (desktop, lock screen…).
    Foreground(Option<String>),
    /// Discord's microphone state, when the mute method can read it.
    DiscordMuted(Option<bool>),
    Paused(bool),
    Profiles(Vec<Profile>),
}

/// What the tray shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Status {
    pub paused: bool,
    /// Id and name of the profile of the foreground game.
    pub profile: Option<(String, String)>,
    /// Discord is muted by the engine right now.
    pub muting: bool,
    pub discord_muted: Option<bool>,
}

#[derive(Default)]
pub struct Engine {
    profiles: Vec<Profile>,
    foreground: Option<String>,
    paused: bool,
    discord_muted: Option<bool>,
    /// Index of the profile of the foreground game.
    active: Option<usize>,
    pressed: HashSet<String>,
    toggled: bool,
    /// The rules want Discord muted.
    engaged: bool,
    /// The engine muted Discord and has to unmute it.
    ours: bool,
}

impl Engine {
    pub fn new(profiles: Vec<Profile>) -> Engine {
        Engine {
            profiles,
            ..Engine::default()
        }
    }

    /// Returns Some(mute) when Discord's microphone has to change.
    pub fn handle(&mut self, input: Input) -> Option<bool> {
        match input {
            Input::Key { code, down } => self.key(code, down),
            Input::Foreground(exe) => {
                self.foreground = exe;
                self.select_profile();
            }
            Input::DiscordMuted(muted) => {
                self.discord_muted = muted;
                if muted == Some(false) && self.ours {
                    // Unmuted by hand: give the microphone back to the user.
                    self.ours = false;
                    self.toggled = false;
                }
            }
            Input::Paused(paused) => {
                self.paused = paused;
                self.reset();
            }
            Input::Profiles(profiles) => {
                let before = self.active.map(|i| self.profiles[i].clone());
                self.profiles = profiles;
                self.active = self.find_active();
                if before.as_ref() != self.active.map(|i| &self.profiles[i]) {
                    self.reset();
                }
            }
        }
        self.apply()
    }

    /// Gives the microphone back before quitting or changing the mute method:
    /// Some(false) when Discord has to be unmuted. Held keys have to be pressed again.
    pub fn release(&mut self) -> Option<bool> {
        self.reset();
        self.engaged = false;
        std::mem::take(&mut self.ours).then_some(false)
    }

    pub fn status(&self) -> Status {
        Status {
            paused: self.paused,
            profile: self
                .active
                .map(|i| (self.profiles[i].id.clone(), self.profiles[i].name.clone())),
            muting: self.ours,
            discord_muted: self.discord_muted,
        }
    }

    fn key(&mut self, code: String, down: bool) {
        if !down {
            self.pressed.remove(&code);
            return;
        }
        let Some(profile) = self.active.map(|i| &self.profiles[i]) else {
            return;
        };
        if !profile.binding.keys.contains(&code) {
            return;
        }
        let first_press = self.pressed.insert(code);
        if first_press && profile.binding.mode == Mode::Toggle {
            self.toggled = !self.toggled;
        }
    }

    fn find_active(&self) -> Option<usize> {
        let exe = self.foreground.as_deref()?;
        self.profiles
            .iter()
            .position(|p| p.enabled && p.matches(exe))
    }

    fn select_profile(&mut self) {
        let active = self.find_active();
        if active != self.active {
            self.active = active;
            self.reset();
        }
    }

    fn reset(&mut self) {
        self.pressed.clear();
        self.toggled = false;
    }

    fn wanted(&self) -> bool {
        let Some(profile) = self.active.map(|i| &self.profiles[i]) else {
            return false;
        };
        !self.paused
            && match profile.binding.mode {
                Mode::Hold => profile
                    .binding
                    .keys
                    .iter()
                    .any(|k| self.pressed.contains(k)),
                Mode::Toggle => self.toggled,
            }
    }

    fn apply(&mut self) -> Option<bool> {
        let wanted = self.wanted();
        if wanted == self.engaged {
            return None;
        }
        self.engaged = wanted;
        if wanted {
            if self.discord_muted == Some(true) {
                return None; // already muted by the user: not ours to undo
            }
            self.ours = true;
            Some(true)
        } else if self.ours {
            self.ours = false;
            Some(false)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Binding;

    fn profile(id: &str, exe: &str, keys: &[&str], mode: Mode) -> Profile {
        Profile {
            id: id.into(),
            name: id.to_uppercase(),
            enabled: true,
            binding: Binding {
                executables: vec![exe.into()],
                keys: keys.iter().map(|k| k.to_string()).collect(),
                mode,
            },
            preset: None,
        }
    }

    fn key(code: &str, down: bool) -> Input {
        Input::Key {
            code: code.into(),
            down,
        }
    }

    fn fg(exe: &str) -> Input {
        Input::Foreground(Some(exe.into()))
    }

    #[test]
    fn hold_mutes_while_a_key_is_down() {
        let mut e = Engine::new(vec![profile(
            "sot",
            "SoTGame.exe",
            &["AltLeft", "Mouse4"],
            Mode::Hold,
        )]);
        assert_eq!(e.handle(key("AltLeft", true)), None); // game not in the foreground
        assert_eq!(e.handle(key("AltLeft", false)), None);
        assert_eq!(e.handle(fg("sotgame.EXE")), None);
        assert_eq!(e.handle(key("KeyW", true)), None);
        assert_eq!(e.handle(key("AltLeft", true)), Some(true));
        assert_eq!(e.handle(key("AltLeft", true)), None); // auto-repeat
        assert_eq!(e.handle(key("Mouse4", true)), None);
        assert_eq!(e.handle(key("AltLeft", false)), None); // Mouse4 still down
        assert!(e.status().muting);
        assert_eq!(e.handle(key("Mouse4", false)), Some(false));
        assert_eq!(e.status().profile, Some(("sot".into(), "SOT".into())));
    }

    #[test]
    fn toggle_flips_on_each_press_only() {
        let mut e = Engine::new(vec![profile("r", "game.exe", &["KeyV"], Mode::Toggle)]);
        e.handle(fg("game.exe"));
        assert_eq!(e.handle(key("KeyV", true)), Some(true));
        assert_eq!(e.handle(key("KeyV", true)), None); // auto-repeat
        assert_eq!(e.handle(key("KeyV", false)), None);
        assert_eq!(e.handle(key("KeyV", true)), Some(false));
        assert_eq!(e.handle(key("KeyV", false)), None);
    }

    #[test]
    fn leaving_the_game_restores_discord() {
        let mut e = Engine::new(vec![
            profile("hold", "a.exe", &["KeyT"], Mode::Hold),
            profile("toggle", "b.exe", &["KeyV"], Mode::Toggle),
        ]);
        e.handle(fg("a.exe"));
        assert_eq!(e.handle(key("KeyT", true)), Some(true));
        assert_eq!(e.handle(fg("browser.exe")), Some(false)); // Alt+Tab with the key down
        assert_eq!(e.handle(key("KeyT", false)), None);
        assert_eq!(e.handle(fg("a.exe")), None);

        e.handle(fg("b.exe"));
        assert_eq!(e.handle(key("KeyV", true)), Some(true));
        e.handle(key("KeyV", false));
        assert_eq!(e.handle(Input::Foreground(None)), Some(false));
        assert_eq!(e.handle(fg("b.exe")), None); // back in the game: press again
        assert_eq!(e.handle(key("KeyV", true)), Some(true));
    }

    #[test]
    fn switching_games_directly_resets_the_state() {
        let mut e = Engine::new(vec![
            profile("a", "a.exe", &["KeyT"], Mode::Toggle),
            profile("b", "b.exe", &["KeyT"], Mode::Toggle),
        ]);
        e.handle(fg("a.exe"));
        assert_eq!(e.handle(key("KeyT", true)), Some(true));
        assert_eq!(e.handle(fg("b.exe")), Some(false));
        assert_eq!(e.handle(key("KeyT", false)), None);
        assert_eq!(e.handle(key("KeyT", true)), Some(true));
    }

    #[test]
    fn disabled_profiles_are_ignored() {
        let mut off = profile("a", "a.exe", &["KeyT"], Mode::Hold);
        off.enabled = false;
        let mut e = Engine::new(vec![off.clone()]);
        e.handle(fg("a.exe"));
        assert_eq!(e.handle(key("KeyT", true)), None);
        assert_eq!(e.status().profile, None);
        e.handle(key("KeyT", false));
        off.enabled = true;
        assert_eq!(e.handle(Input::Profiles(vec![off.clone()])), None);
        assert_eq!(e.handle(key("KeyT", true)), Some(true));
        off.enabled = false;
        assert_eq!(e.handle(Input::Profiles(vec![off])), Some(false)); // disabled while muted
    }

    #[test]
    fn editing_the_active_profile_restores_discord() {
        let p = profile("a", "a.exe", &["KeyT"], Mode::Toggle);
        let mut e = Engine::new(vec![p.clone()]);
        e.handle(fg("a.exe"));
        assert_eq!(e.handle(key("KeyT", true)), Some(true));
        assert_eq!(e.handle(Input::Profiles(vec![p.clone()])), None); // unchanged
        let mut renamed = p.clone();
        renamed.binding.keys = vec!["KeyY".into()];
        assert_eq!(e.handle(Input::Profiles(vec![renamed])), Some(false));
    }

    #[test]
    fn pause_restores_and_blocks() {
        let mut e = Engine::new(vec![profile("a", "a.exe", &["KeyT"], Mode::Hold)]);
        e.handle(fg("a.exe"));
        assert_eq!(e.handle(key("KeyT", true)), Some(true));
        assert_eq!(e.handle(Input::Paused(true)), Some(false));
        assert_eq!(e.handle(key("KeyT", true)), None);
        assert!(e.status().paused);
        e.handle(key("KeyT", false));
        assert_eq!(e.handle(Input::Paused(false)), None);
        assert_eq!(e.handle(key("KeyT", true)), Some(true));
    }

    #[test]
    fn never_unmutes_a_mute_set_by_the_user() {
        let mut e = Engine::new(vec![profile("a", "a.exe", &["KeyT"], Mode::Hold)]);
        e.handle(fg("a.exe"));
        e.handle(Input::DiscordMuted(Some(true)));
        assert_eq!(e.handle(key("KeyT", true)), None);
        assert_eq!(e.handle(key("KeyT", false)), None);
        assert!(!e.status().muting);
    }

    #[test]
    fn unmuting_by_hand_hands_the_microphone_back() {
        let mut e = Engine::new(vec![
            profile("h", "h.exe", &["KeyT"], Mode::Hold),
            profile("t", "t.exe", &["KeyV"], Mode::Toggle),
        ]);
        e.handle(Input::DiscordMuted(Some(false)));
        e.handle(fg("h.exe"));
        assert_eq!(e.handle(key("KeyT", true)), Some(true));
        assert_eq!(e.handle(Input::DiscordMuted(Some(true))), None); // echo of our command
        assert_eq!(e.handle(Input::DiscordMuted(Some(false))), None); // user unmuted
        assert_eq!(e.handle(key("KeyT", false)), None);
        assert_eq!(e.handle(key("KeyT", true)), Some(true));
        e.handle(key("KeyT", false));
        e.handle(Input::DiscordMuted(Some(false)));

        e.handle(fg("t.exe"));
        assert_eq!(e.handle(key("KeyV", true)), Some(true));
        e.handle(key("KeyV", false));
        assert_eq!(e.handle(Input::DiscordMuted(Some(false))), None);
        assert_eq!(e.handle(key("KeyV", true)), Some(true)); // next press mutes again
    }

    #[test]
    fn release_gives_the_microphone_back() {
        let mut e = Engine::new(vec![profile("a", "a.exe", &["KeyT"], Mode::Toggle)]);
        e.handle(fg("a.exe"));
        assert_eq!(e.release(), None);
        assert_eq!(e.handle(key("KeyT", true)), Some(true));
        assert_eq!(e.release(), Some(false));
        assert_eq!(e.release(), None);
        e.handle(key("KeyT", false));
        assert_eq!(e.handle(key("KeyT", true)), Some(true));
    }

    #[test]
    fn first_enabled_matching_profile_wins() {
        let mut first = profile("one", "a.exe", &["KeyT"], Mode::Hold);
        first.enabled = false;
        let second = profile("two", "a.exe", &["KeyV"], Mode::Hold);
        let mut e = Engine::new(vec![first, second]);
        e.handle(fg("a.exe"));
        assert_eq!(e.status().profile.unwrap().0, "two");
        assert_eq!(e.handle(key("KeyT", true)), None);
        assert_eq!(e.handle(key("KeyV", true)), Some(true));
    }
}
