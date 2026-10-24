//! Key codes: physical positions named like `KeyboardEvent.code`, so a game bound to
//! "K" by scan code is the same key on QWERTY and AZERTY. The settings window shows
//! the label of the user's layout.

/// (code, Windows set-1 scan code with 0xE000 for E0-prefixed keys, Linux evdev code).
/// X11 key codes are the evdev codes + 8.
#[rustfmt::skip]
const KEYS: &[(&str, u16, u16)] = &[
    ("Escape", 0x01, 1), ("Digit1", 0x02, 2), ("Digit2", 0x03, 3), ("Digit3", 0x04, 4),
    ("Digit4", 0x05, 5), ("Digit5", 0x06, 6), ("Digit6", 0x07, 7), ("Digit7", 0x08, 8),
    ("Digit8", 0x09, 9), ("Digit9", 0x0A, 10), ("Digit0", 0x0B, 11), ("Minus", 0x0C, 12),
    ("Equal", 0x0D, 13), ("Backspace", 0x0E, 14), ("Tab", 0x0F, 15),
    ("KeyQ", 0x10, 16), ("KeyW", 0x11, 17), ("KeyE", 0x12, 18), ("KeyR", 0x13, 19),
    ("KeyT", 0x14, 20), ("KeyY", 0x15, 21), ("KeyU", 0x16, 22), ("KeyI", 0x17, 23),
    ("KeyO", 0x18, 24), ("KeyP", 0x19, 25), ("BracketLeft", 0x1A, 26),
    ("BracketRight", 0x1B, 27), ("Enter", 0x1C, 28), ("ControlLeft", 0x1D, 29),
    ("KeyA", 0x1E, 30), ("KeyS", 0x1F, 31), ("KeyD", 0x20, 32), ("KeyF", 0x21, 33),
    ("KeyG", 0x22, 34), ("KeyH", 0x23, 35), ("KeyJ", 0x24, 36), ("KeyK", 0x25, 37),
    ("KeyL", 0x26, 38), ("Semicolon", 0x27, 39), ("Quote", 0x28, 40),
    ("Backquote", 0x29, 41), ("ShiftLeft", 0x2A, 42), ("Backslash", 0x2B, 43),
    ("KeyZ", 0x2C, 44), ("KeyX", 0x2D, 45), ("KeyC", 0x2E, 46), ("KeyV", 0x2F, 47),
    ("KeyB", 0x30, 48), ("KeyN", 0x31, 49), ("KeyM", 0x32, 50), ("Comma", 0x33, 51),
    ("Period", 0x34, 52), ("Slash", 0x35, 53), ("ShiftRight", 0x36, 54),
    ("NumpadMultiply", 0x37, 55), ("AltLeft", 0x38, 56), ("Space", 0x39, 57),
    ("CapsLock", 0x3A, 58), ("F1", 0x3B, 59), ("F2", 0x3C, 60), ("F3", 0x3D, 61),
    ("F4", 0x3E, 62), ("F5", 0x3F, 63), ("F6", 0x40, 64), ("F7", 0x41, 65),
    ("F8", 0x42, 66), ("F9", 0x43, 67), ("F10", 0x44, 68), ("NumLock", 0xE045, 69),
    ("ScrollLock", 0x46, 70), ("Numpad7", 0x47, 71), ("Numpad8", 0x48, 72),
    ("Numpad9", 0x49, 73), ("NumpadSubtract", 0x4A, 74), ("Numpad4", 0x4B, 75),
    ("Numpad5", 0x4C, 76), ("Numpad6", 0x4D, 77), ("NumpadAdd", 0x4E, 78),
    ("Numpad1", 0x4F, 79), ("Numpad2", 0x50, 80), ("Numpad3", 0x51, 81),
    ("Numpad0", 0x52, 82), ("NumpadDecimal", 0x53, 83), ("IntlBackslash", 0x56, 86),
    ("F11", 0x57, 87), ("F12", 0x58, 88), ("NumpadEnter", 0xE01C, 96),
    ("ControlRight", 0xE01D, 97), ("NumpadDivide", 0xE035, 98),
    ("PrintScreen", 0xE037, 99), ("AltRight", 0xE038, 100), ("Home", 0xE047, 102),
    ("ArrowUp", 0xE048, 103), ("PageUp", 0xE049, 104), ("ArrowLeft", 0xE04B, 105),
    ("ArrowRight", 0xE04D, 106), ("End", 0xE04F, 107), ("ArrowDown", 0xE050, 108),
    ("PageDown", 0xE051, 109), ("Insert", 0xE052, 110), ("Delete", 0xE053, 111),
    // Windows reports Pause with the NumLock scan code and no E0 prefix.
    ("Pause", 0x45, 119), ("MetaLeft", 0xE05B, 125), ("MetaRight", 0xE05C, 126),
    ("ContextMenu", 0xE05D, 127),
    ("F13", 0x64, 183), ("F14", 0x65, 184), ("F15", 0x66, 185), ("F16", 0x67, 186),
    ("F17", 0x68, 187), ("F18", 0x69, 188), ("F19", 0x6A, 189), ("F20", 0x6B, 190),
    ("F21", 0x6C, 191), ("F22", 0x6D, 192), ("F23", 0x6E, 193), ("F24", 0x76, 194),
];

/// (code, Linux evdev button code). Windows reports them as mouse messages.
const MOUSE: &[(&str, u16)] = &[("MouseMiddle", 0x112), ("Mouse4", 0x113), ("Mouse5", 0x114)];

pub fn is_known(code: &str) -> bool {
    KEYS.iter().any(|k| k.0 == code) || MOUSE.iter().any(|m| m.0 == code)
}

pub fn is_mouse(code: &str) -> bool {
    MOUSE.iter().any(|m| m.0 == code)
}

/// From a Windows low-level hook: scan code and LLKHF_EXTENDED.
pub fn from_scancode(scan: u16, extended: bool) -> Option<&'static str> {
    let scan = if extended { 0xE000 | scan } else { scan };
    KEYS.iter().find(|k| k.1 == scan).map(|k| k.0)
}

/// For SendInput: (scan code, extended).
pub fn to_scancode(code: &str) -> Option<(u16, bool)> {
    let scan = KEYS.iter().find(|k| k.0 == code)?.1;
    Some((scan & 0xFF, scan & 0xE000 != 0))
}

/// From a Linux input event (keys and mouse buttons).
pub fn from_evdev(code: u16) -> Option<&'static str> {
    KEYS.iter()
        .find(|k| k.2 == code)
        .map(|k| k.0)
        .or_else(|| MOUSE.iter().find(|m| m.1 == code).map(|m| m.0))
}

pub fn to_evdev(code: &str) -> Option<u16> {
    KEYS.iter().find(|k| k.0 == code).map(|k| k.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes() {
        for code in [
            "KeyK", "Digit0", "Numpad7", "F1", "F24", "AltLeft", "Mouse5",
        ] {
            assert!(is_known(code), "{code}");
        }
        for code in ["K", "Keyk", "KeyKK", "F0", "F25", "F01", "Mouse6", ""] {
            assert!(!is_known(code), "{code}");
        }
    }

    #[test]
    fn tables_are_unique_and_complete() {
        for (i, a) in KEYS.iter().enumerate() {
            for b in &KEYS[i + 1..] {
                assert!(a.0 != b.0 && a.1 != b.1 && a.2 != b.2, "{a:?} / {b:?}");
            }
        }
        let letters = KEYS.iter().filter(|k| k.0.starts_with("Key")).count();
        let functions = KEYS
            .iter()
            .filter(|k| k.0.starts_with('F') && k.0[1..].parse::<u8>().is_ok())
            .count();
        assert_eq!((letters, functions), (26, 24));
    }

    #[test]
    fn conversions() {
        assert_eq!(from_scancode(0x38, false), Some("AltLeft"));
        assert_eq!(from_scancode(0x38, true), Some("AltRight"));
        assert_eq!(from_scancode(0x21D, false), None); // fake Ctrl sent with AltGr
        assert_eq!(to_scancode("ArrowUp"), Some((0x48, true)));
        assert_eq!(to_scancode("F13"), Some((0x64, false)));
        assert_eq!(from_evdev(37), Some("KeyK"));
        assert_eq!(from_evdev(0x113), Some("Mouse4"));
        assert_eq!(to_evdev("F13"), Some(183));
        assert_eq!(to_evdev("Mouse4"), None);
    }
}
