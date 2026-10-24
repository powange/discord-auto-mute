//! Windows: low-level keyboard and mouse hooks, GetForegroundWindow, SendInput.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::OnceLock;

use dam_core::keys;
use windows_sys::Win32::Foundation::{CloseHandle, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, OpenProcess, QueryFullProcessImageNameW, SetThreadPriority,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, MAPVK_VSC_TO_VK_EX,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, SetWindowsHookExW,
    KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use super::KeySink;

static SINK: OnceLock<KeySink> = OnceLock::new();

/// Marks the keys this application sends (dwExtraInfo): only those are ignored, so
/// keys sent by AutoHotkey, mouse software or Steam Input still count.
const OWN_INPUT: usize = 0x4441_4D21; // "DAM!"

/// Hook procedures run on the thread that installed them, between two messages:
/// they only forward the event and return at once (Windows drops slow hooks).
unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = &*(lparam as *const KBDLLHOOKSTRUCT);
        let own = info.flags & LLKHF_INJECTED != 0 && info.dwExtraInfo == OWN_INPUT;
        if !own {
            let down = match wparam as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
                WM_KEYUP | WM_SYSKEYUP => Some(false),
                _ => None,
            };
            let extended = info.flags & LLKHF_EXTENDED != 0;
            if let (Some(down), Ok(scan)) = (down, u16::try_from(info.scanCode)) {
                if let (Some(key), Some(sink)) = (keys::from_scancode(scan, extended), SINK.get()) {
                    sink(key, down);
                }
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = &*(lparam as *const MSLLHOOKSTRUCT);
        let own = info.flags & LLMHF_INJECTED != 0 && info.dwExtraInfo == OWN_INPUT;
        if !own {
            let x_button = || match info.mouseData >> 16 {
                1 => Some("Mouse4"),
                2 => Some("Mouse5"),
                _ => None,
            };
            let event = match wparam as u32 {
                WM_MBUTTONDOWN => Some(("MouseMiddle", true)),
                WM_MBUTTONUP => Some(("MouseMiddle", false)),
                WM_XBUTTONDOWN => x_button().map(|k| (k, true)),
                WM_XBUTTONUP => x_button().map(|k| (k, false)),
                _ => None,
            };
            if let (Some((key, down)), Some(sink)) = (event, SINK.get()) {
                sink(key, down);
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

/// Installs the hooks on a dedicated thread with its own message loop.
pub fn listen(sink: KeySink) -> Result<(), String> {
    if SINK.set(sink).is_err() {
        return Err("already listening".into());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("input-hooks".into())
        .spawn(move || unsafe {
            // Windows silently removes a hook that answers too slowly
            // (LowLevelHooksTimeout), e.g. while a game loads every core.
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
            let module = GetModuleHandleW(std::ptr::null());
            let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
            if keyboard.is_null() || mouse.is_null() {
                let _ = tx.send(Err(format!(
                    "SetWindowsHookEx failed ({})",
                    std::io::Error::last_os_error()
                )));
                return;
            }
            let _ = tx.send(Ok(()));
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
        })
        .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

pub fn foreground_executable() -> Option<String> {
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, &mut pid);
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let ok =
            QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..size as usize]))
    }
}

/// Presses (in order) or releases (in reverse order) the keys.
pub fn send_keys(codes: &[String], down: bool) -> Result<(), String> {
    let mut inputs = Vec::with_capacity(codes.len());
    for code in codes {
        let (scan, extended) =
            keys::to_scancode(code).ok_or_else(|| format!("cannot send {code}"))?;
        let full = if extended { 0xE000 | scan } else { scan };
        // Virtual key and scan code both: hooks (Discord's included) read one or the other.
        let vk = unsafe { MapVirtualKeyW(u32::from(full), MAPVK_VSC_TO_VK_EX) } as u16;
        let mut flags = if down { 0 } else { KEYEVENTF_KEYUP };
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: OWN_INPUT,
                },
            },
        });
    }
    if !down {
        inputs.reverse();
    }
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(format!(
            "SendInput failed ({})",
            std::io::Error::last_os_error()
        ))
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// A REG_SZ value of HKEY_CURRENT_USER.
pub fn registry_string(subkey: &str, value: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let (subkey, value) = (wide(subkey), wide(value));
    let mut buffer = vec![0u16; 1024];
    let mut bytes = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != 0 {
        return None;
    }
    let len = (bytes as usize / 2).saturating_sub(1); // without the final NUL
    Some(String::from_utf16_lossy(&buffer[..len]))
}

/// The icon of an executable, 64×64, as a PNG.
pub fn exe_icon_png(path: &Path) -> Option<Vec<u8>> {
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, GetIconInfo, PrivateExtractIconsW, ICONINFO,
    };
    const SIZE: i32 = 64;
    let file: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut icon = std::ptr::null_mut();
    let count = unsafe {
        PrivateExtractIconsW(
            file.as_ptr(),
            0,
            SIZE,
            SIZE,
            &mut icon,
            std::ptr::null_mut(),
            1,
            0,
        )
    };
    if count == 0 || count == u32::MAX || icon.is_null() {
        return None;
    }
    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
    let lines = unsafe {
        let mut info: ICONINFO = std::mem::zeroed();
        if GetIconInfo(icon, &mut info) == 0 {
            DestroyIcon(icon);
            return None;
        }
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let mut bitmap: BITMAPINFO = std::mem::zeroed();
        bitmap.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bitmap.bmiHeader.biWidth = SIZE;
        bitmap.bmiHeader.biHeight = -SIZE; // top-down rows
        bitmap.bmiHeader.biPlanes = 1;
        bitmap.bmiHeader.biBitCount = 32;
        bitmap.bmiHeader.biCompression = BI_RGB;
        let lines = GetDIBits(
            dc,
            info.hbmColor,
            0,
            SIZE as u32,
            pixels.as_mut_ptr().cast(),
            &mut bitmap,
            DIB_RGB_COLORS,
        );
        DeleteDC(dc);
        DeleteObject(info.hbmColor as _);
        DeleteObject(info.hbmMask as _);
        DestroyIcon(icon);
        lines
    };
    if lines == 0 {
        return None;
    }
    for pixel in pixels.chunks_mut(4) {
        pixel.swap(0, 2); // BGRA -> RGBA
    }
    if pixels.chunks(4).all(|p| p[3] == 0) {
        // Old icons have no alpha channel: opaque.
        for pixel in pixels.chunks_mut(4) {
            pixel[3] = 255;
        }
    }
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, SIZE as u32, SIZE as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .ok()?
            .write_image_data(&pixels)
            .ok()?;
    }
    Some(png)
}
