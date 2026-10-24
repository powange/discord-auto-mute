# Discord Auto-Mute

Mutes your Discord microphone while you talk in a game's own voice chat, so the friends you play with do not hear you twice.

A tray application for Windows and Linux: each game has a profile with its voice key (push-to-talk). While the game is in the foreground, pressing that key mutes Discord.

## Features

- One profile per game: name, game executables, one or more voice keys (keyboard or mouse buttons middle, 4 and 5), and a mode:
  - **Hold**: Discord is muted while the key is held down;
  - **Toggle**: each press mutes or unmutes Discord.
- 50 presets for the most played multiplayer games with voice chat. Every field stays editable, and "Restore the defaults" brings the preset back.
- A profile only applies while its game is in the foreground. Leaving the game, pausing or quitting gives the microphone back.
- The application only unmutes what it muted: a mute you set yourself in Discord is left alone.
- Two ways to mute Discord:
  - **Keyboard shortcut**: a Discord keybind ("Toggle Mute" or "Push to Mute"), F13 by default, a key no game uses;
  - **Application**: Discord's local RPC API, with your own application from the [Discord Developer Portal](https://discord.com/developers/applications).
- English and French, the system language by default.

## Installation

Download the installer from the [releases](https://github.com/powange/discord-auto-mute/releases).

On Linux, the keys are read from `/dev/input`: add your user to the `input` group (`sudo usermod -aG input $USER`, then log out and back in). The game in the foreground is found through X11 (games under XWayland and Proton included). Discord shortcuts only work in an X11 session: under Wayland, use the Application method.

## Development

```sh
cargo test --workspace
cargo run -p discord-auto-mute
```

- `crates/core`: profiles, presets (`presets.json`), mute rules, settings, translations (`i18n/<code>.json`).
- `src-tauri`: the Tauri 2 application (tray, settings window in `ui/`, platform code).
- The Discord RPC client comes from [discord-voice-rpc](https://github.com/powange/discord-voice-rpc).

Debug builds accept `DISCORD_AUTO_MUTE_HOME` (settings folder) and `DAM_SETTINGS_TAB` (tab opened first).

## License

MIT
