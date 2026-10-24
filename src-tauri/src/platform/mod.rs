//! What differs between systems: listening to the keyboard and mouse globally,
//! the process in the foreground and synthetic key presses for Discord's shortcut.

/// Receives (key code, down) for every key or mouse button press, from any thread.
pub type KeySink = Box<dyn Fn(&'static str, bool) + Send + Sync>;

/// `listen` error: the user may not read the input devices (Linux `input` group).
pub const NO_PERMISSION: &str = "no-permission";

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{foreground_executable, listen, send_keys};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{foreground_executable, listen, send_keys};

/// File name of a path written with / or \ (Wine processes report Windows paths).
pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_names() {
        assert_eq!(super::file_name(r"C:\Games\SoT\SoTGame.exe"), "SoTGame.exe");
        assert_eq!(super::file_name("/usr/bin/cs2"), "cs2");
        assert_eq!(super::file_name("Z:/a/b.exe"), "b.exe");
        assert_eq!(super::file_name("game"), "game");
    }
}
