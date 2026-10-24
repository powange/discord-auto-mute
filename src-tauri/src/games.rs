//! Games installed by Steam and Epic, and game icons, for the settings window.
//! Steam icons come from Steam's own image cache; on Windows, the icon of a game's
//! executable is extracted once (installed games, games seen in the foreground)
//! and kept in the data folder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use dam_core::config::data_dir;
use dam_core::library::{self, FoundExe, Store};
use dam_core::profile::{presets, Profile};
use serde::Serialize;

/// Folder depth and file count explored per game: enough for Unreal's
/// Game/Binaries/Win64, bounded for huge installs.
const MAX_DEPTH: usize = 6;
const MAX_ENTRIES: usize = 20_000;

#[derive(Clone, Debug, Serialize)]
pub struct InstalledGame {
    /// "steam:<app id>" or "epic:<app name>".
    pub id: String,
    pub store: Store,
    pub name: String,
    #[serde(skip)]
    pub steam_app_id: Option<u32>,
    /// Executable file names, most likely first.
    pub executables: Vec<String>,
    /// Preset id, when one matches.
    pub preset: Option<String>,
    #[serde(skip)]
    pub icon: Option<PathBuf>,
    /// Full path of the first executable (Windows icon extraction).
    #[serde(skip)]
    pub main_exe: Option<PathBuf>,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// Steam folders: the one of the official client, Flatpak and Snap on Linux.
fn steam_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut roots: Vec<PathBuf> =
            crate::platform::registry_string(r"Software\Valve\Steam", "SteamPath")
                .map(PathBuf::from)
                .into_iter()
                .collect();
        roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
        roots
    }
    #[cfg(not(windows))]
    {
        let home = home();
        vec![
            home.join(".local/share/Steam"),
            home.join(".steam/steam"),
            home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            home.join("snap/steam/common/.local/share/Steam"),
        ]
    }
}

/// Walks a game folder for .exe files (bounded).
fn find_exes(dir: &Path) -> Vec<(FoundExe, PathBuf)> {
    let mut found = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((folder, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return found;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if kind.is_file()
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
            {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let file_name = entry.file_name().to_string_lossy().into_owned();
                found.push((FoundExe { file_name, size }, path));
            }
        }
    }
    found
}

/// Executables of a folder, most likely first, with the full path of the first.
fn executables_of(dir: &Path) -> (Vec<String>, Option<PathBuf>) {
    let found = find_exes(dir);
    let names: Vec<FoundExe> = found.iter().map(|(f, _)| f.clone()).collect();
    let chosen = library::game_executables(&names);
    let main = chosen.first().and_then(|first| {
        found
            .iter()
            .find(|(f, _)| &f.file_name == first)
            .map(|(_, p)| p.clone())
    });
    (chosen, main)
}

/// The square icon Steam keeps for an app (old and 2024 cache layouts).
fn steam_icon(root: &Path, app_id: u32) -> Option<PathBuf> {
    let cache = root.join("appcache/librarycache");
    let old = cache.join(format!("{app_id}_icon.jpg"));
    if old.is_file() {
        return Some(old);
    }
    // New layout: librarycache/<id>/<sha1>.jpg is the icon, next to header.jpg & co.
    let folder = cache.join(app_id.to_string());
    std::fs::read_dir(&folder)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.extension().is_some_and(|e| e == "jpg")
                && p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()))
        })
}

fn steam_games() -> Vec<InstalledGame> {
    let mut games = Vec::new();
    let mut seen_libraries = Vec::new();
    for root in steam_roots() {
        let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) else {
            continue;
        };
        for library in library::steam_libraries(&text) {
            let steamapps = PathBuf::from(&library).join("steamapps");
            let key = std::fs::canonicalize(&steamapps).unwrap_or(steamapps.clone());
            if seen_libraries.contains(&key) {
                continue; // ~/.steam/steam is a link to ~/.local/share/Steam
            }
            seen_libraries.push(key);
            let Ok(entries) = std::fs::read_dir(&steamapps) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                    continue;
                }
                let Some(app) = std::fs::read_to_string(entry.path())
                    .ok()
                    .and_then(|t| library::steam_app(&t))
                else {
                    continue;
                };
                let dir = steamapps.join("common").join(&app.install_dir);
                if !dir.is_dir() {
                    continue;
                }
                let (executables, main_exe) = executables_of(&dir);
                games.push(InstalledGame {
                    id: format!("steam:{}", app.app_id),
                    store: Store::Steam,
                    name: app.name,
                    steam_app_id: Some(app.app_id),
                    executables,
                    preset: None,
                    icon: steam_icon(&root, app.app_id),
                    main_exe,
                });
            }
        }
    }
    games
}

fn epic_games() -> Vec<InstalledGame> {
    let mut apps = Vec::new();
    #[cfg(windows)]
    {
        let data = std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".into());
        let manifests = PathBuf::from(data).join(r"Epic\EpicGamesLauncher\Data\Manifests");
        if let Ok(entries) = std::fs::read_dir(manifests) {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|e| e == "item") {
                    if let Some(app) = std::fs::read_to_string(entry.path())
                        .ok()
                        .and_then(|t| library::epic_app(&t))
                    {
                        apps.push(app);
                    }
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        for file in [
            ".config/heroic/legendaryConfig/legendary/installed.json",
            ".var/app/com.heroicgameslauncher.hgl/config/heroic/legendaryConfig/legendary/installed.json",
        ] {
            if let Ok(text) = std::fs::read_to_string(home().join(file)) {
                apps.extend(library::heroic_apps(&text));
            }
        }
    }
    apps.into_iter()
        .filter_map(|app| {
            let dir = PathBuf::from(&app.install_location);
            dir.is_dir().then(|| {
                let (executables, main_exe) = executables_of(&dir);
                InstalledGame {
                    id: format!("epic:{}", app.app_name),
                    store: Store::Epic,
                    name: app.name,
                    steam_app_id: None,
                    executables,
                    preset: None,
                    icon: None,
                    main_exe,
                }
            })
        })
        .collect()
}

/// Every installed game, presets matched, sorted by name. Slow (disk): call it
/// off the main thread.
pub fn scan() -> Vec<InstalledGame> {
    let mut games = steam_games();
    games.extend(epic_games());
    for game in &mut games {
        game.preset = library::matching_preset(presets(), game.steam_app_id, &game.executables)
            .map(|p| p.id.clone());
        if game.icon.is_none() {
            game.icon = game.main_exe.as_deref().and_then(exe_icon);
        }
    }
    games.sort_by_key(|g| g.name.to_lowercase());
    games.dedup_by(|a, b| a.id == b.id);
    log::info!("{} installed games found", games.len());
    games
}

fn icons_dir() -> PathBuf {
    data_dir().join("icons")
}

/// Cached icon of an executable file name.
fn cached_icon(exe_name: &str) -> PathBuf {
    // The file name is matched against the profiles' executables: keep it readable.
    let safe: String = exe_name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    icons_dir().join(format!("{safe}.png"))
}

/// The icon of an executable, extracted once (Windows only).
pub fn exe_icon(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let cached = cached_icon(&name);
    if cached.is_file() {
        return Some(cached);
    }
    #[cfg(windows)]
    {
        let png = crate::platform::exe_icon_png(path)?;
        std::fs::create_dir_all(icons_dir()).ok()?;
        discord_voice_rpc::fs::atomic_write(&cached, &png).ok()?;
        Some(cached)
    }
    #[cfg(not(windows))]
    None
}

fn data_url(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let mime = match path.extension()?.to_str()?.to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => return None,
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// The installed games, as the window shows them.
#[derive(Serialize)]
pub struct GameView {
    #[serde(flatten)]
    game: InstalledGame,
    icon: Option<String>,
}

pub fn views(games: &[InstalledGame]) -> Vec<GameView> {
    games
        .iter()
        .map(|g| GameView {
            game: g.clone(),
            icon: g.icon.as_deref().and_then(data_url),
        })
        .collect()
}

/// Icon of each profile: its installed game, or the icon kept from a run.
pub fn profile_icons(profiles: &[Profile], games: &[InstalledGame]) -> HashMap<String, String> {
    let mut icons = HashMap::new();
    let cached: Vec<PathBuf> = std::fs::read_dir(icons_dir())
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    for profile in profiles {
        let preset = profile.preset.as_ref().map(|p| p.id.as_str());
        let game = games.iter().find(|g| {
            (preset.is_some() && g.preset.as_deref() == preset)
                || g.executables.iter().any(|exe| profile.matches(exe))
        });
        let path = game.and_then(|g| g.icon.clone()).or_else(|| {
            cached
                .iter()
                .find(|p| {
                    p.file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|exe| profile.matches(exe))
                })
                .cloned()
        });
        if let Some(url) = path.as_deref().and_then(data_url) {
            icons.insert(profile.id.clone(), url);
        }
    }
    icons
}
