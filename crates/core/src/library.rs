//! Games installed by Steam and Epic: reading their files, choosing the game's
//! executable among those of its folder, and finding the matching preset. The
//! application does the file system part; everything here is testable.

use serde::Serialize;

use crate::profile::{wildcard_match, Preset};

/// A value of Valve's KeyValues text format (libraryfolders.vdf, appmanifest_*.acf).
#[derive(Clone, Debug, PartialEq)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

impl Vdf {
    /// The value of a key, case-insensitive like Steam.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(entries) => entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Obj(entries) => entries,
            Vdf::Str(_) => &[],
        }
    }
}

/// Parses a KeyValues text; the whole file is one object. Lenient: an unexpected
/// token ends the object being read.
pub fn parse_vdf(text: &str) -> Vdf {
    let mut tokens = tokenize(text).into_iter().peekable();
    Vdf::Obj(parse_object(&mut tokens))
}

#[derive(Debug, PartialEq)]
enum Token {
    Str(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => out.push(Token::Open),
            '}' => out.push(Token::Close),
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(other) => s.push(other),
                            None => break,
                        },
                        other => s.push(other),
                    }
                }
                out.push(Token::Str(s));
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            _ => {
                // Unquoted token (rare in these files): up to the next separator.
                let mut s = String::from(c);
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                out.push(Token::Str(s));
            }
        }
    }
    out
}

fn parse_object(tokens: &mut std::iter::Peekable<std::vec::IntoIter<Token>>) -> Vec<(String, Vdf)> {
    let mut entries = Vec::new();
    while let Some(token) = tokens.next() {
        let Token::Str(key) = token else {
            break; // Close (or a stray Open)
        };
        match tokens.next() {
            Some(Token::Str(value)) => entries.push((key, Vdf::Str(value))),
            Some(Token::Open) => entries.push((key, Vdf::Obj(parse_object(tokens)))),
            _ => break,
        }
    }
    entries
}

/// Library folders listed in steamapps/libraryfolders.vdf (the Steam folder itself
/// is one of them).
pub fn steam_libraries(libraryfolders: &str) -> Vec<String> {
    let root = parse_vdf(libraryfolders);
    let Some(folders) = root.get("libraryfolders") else {
        return Vec::new();
    };
    folders
        .entries()
        .iter()
        // Libraries are numbered; other keys ("contentstatsid"…) are not.
        .filter(|(key, _)| key.parse::<u32>().is_ok())
        .filter_map(|(_, folder)| match folder {
            Vdf::Obj(_) => folder.str("path").map(str::to_string),
            // Very old format: "1" "D:\\SteamLibrary"
            Vdf::Str(path) => Some(path.clone()),
        })
        .collect()
}

/// A game from an appmanifest_<id>.acf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteamApp {
    pub app_id: u32,
    pub name: String,
    /// Folder name under steamapps/common.
    pub install_dir: String,
}

/// Tools Steam installs like games.
fn is_steam_tool(name: &str) -> bool {
    let name = name.to_lowercase();
    [
        "proton",
        "steam linux runtime",
        "steamworks",
        "steamvr",
        "steam audio",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

pub fn steam_app(manifest: &str) -> Option<SteamApp> {
    let root = parse_vdf(manifest);
    let state = root.get("AppState")?;
    let app = SteamApp {
        app_id: state.str("appid")?.parse().ok()?,
        name: state.str("name")?.to_string(),
        install_dir: state.str("installdir")?.to_string(),
    };
    (!is_steam_tool(&app.name)).then_some(app)
}

/// A game from an Epic Games Launcher manifest (Data/Manifests/*.item, JSON).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpicApp {
    pub app_name: String,
    pub name: String,
    pub install_location: String,
    /// Relative to the install location; often a launcher stub.
    pub launch_executable: String,
}

pub fn epic_app(item: &str) -> Option<EpicApp> {
    let value: serde_json::Value = serde_json::from_str(item).ok()?;
    if value["bIsIncompleteInstall"].as_bool() == Some(true) {
        return None;
    }
    // Add-ons and engine installs have no executable.
    let launch = value["LaunchExecutable"]
        .as_str()
        .filter(|s| !s.is_empty())?;
    Some(EpicApp {
        app_name: value["AppName"].as_str()?.to_string(),
        name: value["DisplayName"].as_str()?.to_string(),
        install_location: value["InstallLocation"].as_str()?.to_string(),
        launch_executable: launch.to_string(),
    })
}

/// Games installed through Heroic (Epic on Linux): legendary's installed.json.
pub fn heroic_apps(installed: &str) -> Vec<EpicApp> {
    let Ok(serde_json::Value::Object(apps)) = serde_json::from_str(installed) else {
        return Vec::new();
    };
    apps.values()
        .filter_map(|app| {
            Some(EpicApp {
                app_name: app["app_name"].as_str()?.to_string(),
                name: app["title"].as_str()?.to_string(),
                install_location: app["install_path"].as_str()?.to_string(),
                launch_executable: app["executable"].as_str().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// Executables of a game folder that are never the game's window.
pub fn is_helper_executable(file_name: &str) -> bool {
    let name = file_name.to_lowercase();
    [
        "unins",
        "crash",
        "redist",
        "vcredist",
        "vc_redist",
        "dxsetup",
        "dotnet",
        "setup",
        "install",
        "prereq",
        "update",
        "report",
        "helper",
        "easyanticheat",
        "eac_",
        "battleye",
        "be_service",
        "launcher",
        "bootstrap",
        "cefprocess",
        "webhelper",
        "overlay",
        "x86",
        "server",
        "benchmark",
        "editor",
        "touchup",
        "cleanup",
        "uploader",
        "bugsplat",
    ]
    .iter()
    .any(|word| name.contains(word))
}

/// An executable found in a game folder, with its size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundExe {
    pub file_name: String,
    pub size: u64,
}

/// The executables worth proposing for a game, most likely first: helpers left
/// out, Unreal "-Shipping" binaries first, then the biggest file.
pub fn game_executables(found: &[FoundExe]) -> Vec<String> {
    let mut candidates: Vec<&FoundExe> = found
        .iter()
        .filter(|e| !is_helper_executable(&e.file_name))
        .collect();
    candidates.sort_by_key(|e| {
        let shipping = e.file_name.to_lowercase().contains("-shipping");
        (!shipping, std::cmp::Reverse(e.size))
    });
    let mut out: Vec<String> = Vec::new();
    for exe in candidates {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&exe.file_name)) {
            out.push(exe.file_name.clone());
        }
    }
    out
}

/// Where an installed game comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Store {
    Steam,
    Epic,
}

/// The preset of an installed game: its Steam app, or one of its executables.
pub fn matching_preset<'a>(
    presets: &'a [Preset],
    steam_app_id: Option<u32>,
    executables: &[String],
) -> Option<&'a Preset> {
    if let Some(id) = steam_app_id {
        if let Some(preset) = presets.iter().find(|p| p.steam_app_ids.contains(&id)) {
            return Some(preset);
        }
    }
    presets.iter().find(|p| {
        p.binding
            .executables
            .iter()
            .any(|pattern| executables.iter().any(|exe| wildcard_match(pattern, exe)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::presets;

    const LIBRARIES: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"228980"		"0"
			"730"		"36454545"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps" { "1172620" "123" }
	}
}
"#;

    #[test]
    fn libraries_and_escapes() {
        assert_eq!(
            steam_libraries(LIBRARIES),
            vec![
                r"C:\Program Files (x86)\Steam".to_string(),
                r"D:\SteamLibrary".to_string()
            ]
        );
        assert_eq!(
            steam_libraries(
                "\"LibraryFolders\" { \"contentstatsid\" \"1\" \"1\" \"E:\\\\Games\" }"
            ),
            vec![r"E:\Games".to_string()]
        );
        assert!(steam_libraries("garbage {{{").is_empty());
    }

    #[test]
    fn app_manifests() {
        let manifest = r#"
"AppState"
{
	"appid"		"1172620"
	"name"		"Sea of Thieves"
	"StateFlags"		"4"
	"installdir"		"Sea of Thieves"
	// comment
	"UserConfig" { "language" "french" }
}"#;
        assert_eq!(
            steam_app(manifest),
            Some(SteamApp {
                app_id: 1172620,
                name: "Sea of Thieves".into(),
                install_dir: "Sea of Thieves".into()
            })
        );
        let proton = r#""AppState" { "appid" "1493710" "name" "Proton Experimental" "installdir" "Proton - Experimental" }"#;
        assert_eq!(steam_app(proton), None);
        assert_eq!(steam_app(r#""AppState" { "name" "x" }"#), None);
    }

    #[test]
    fn epic_manifests() {
        let item = r#"{"FormatVersion": 0, "bIsIncompleteInstall": false, "AppName": "Fortnite",
            "DisplayName": "Fortnite", "InstallLocation": "C:\\Program Files\\Epic Games\\Fortnite",
            "LaunchExecutable": "FortniteGame/Binaries/Win64/FortniteLauncher.exe"}"#;
        let app = epic_app(item).unwrap();
        assert_eq!(
            (app.app_name.as_str(), app.name.as_str()),
            ("Fortnite", "Fortnite")
        );
        assert_eq!(epic_app(&item.replace("false", "true")), None);
        assert_eq!(
            epic_app(&item.replace("FortniteGame/Binaries/Win64/FortniteLauncher.exe", "")),
            None
        );
        let heroic = r#"{"Fortnite": {"app_name": "Fortnite", "title": "Fortnite",
            "install_path": "/games/Fortnite", "executable": "FortniteGame/Binaries/Win64/FortniteLauncher.exe"}}"#;
        assert_eq!(heroic_apps(heroic)[0].install_location, "/games/Fortnite");
        assert!(heroic_apps("[]").is_empty());
    }

    #[test]
    fn choosing_the_game_executable() {
        let found = |list: &[(&str, u64)]| -> Vec<FoundExe> {
            list.iter()
                .map(|(n, s)| FoundExe {
                    file_name: n.to_string(),
                    size: *s,
                })
                .collect()
        };
        assert_eq!(
            game_executables(&found(&[
                ("unins000.exe", 3_000_000),
                ("UnityCrashHandler64.exe", 1_000_000),
                ("Lethal Company.exe", 650_000),
            ])),
            vec!["Lethal Company.exe".to_string()]
        );
        assert_eq!(
            game_executables(&found(&[
                ("PioneerGame.exe", 400_000),
                ("PioneerGame-Win64-Shipping.exe", 150_000_000),
                ("CrashReportClient.exe", 20_000_000),
                ("EasyAntiCheat_EOS_Setup.exe", 5_000_000),
            ])),
            vec![
                "PioneerGame-Win64-Shipping.exe".to_string(),
                "PioneerGame.exe".to_string()
            ]
        );
    }

    #[test]
    fn presets_of_installed_games() {
        let all = presets();
        assert_eq!(
            matching_preset(all, Some(1172620), &[]).unwrap().id,
            "sea-of-thieves"
        );
        let exes = ["FortniteClient-Win64-Shipping.exe".to_string()];
        assert_eq!(matching_preset(all, None, &exes).unwrap().id, "fortnite");
        assert!(matching_preset(all, Some(1), &["notepad.exe".into()]).is_none());
    }
}
