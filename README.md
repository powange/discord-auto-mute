# Discord Auto-Mute

Mutes your Discord microphone while you talk in a game's own voice chat, so the friends you play with do not hear you twice.

A tray application for Windows and Linux: each game has a profile with its voice key (push-to-talk). While the game is in the foreground, pressing that key mutes Discord.

## Features

- One profile per game: name, game executables, one or more voice keys (keyboard or mouse buttons middle, 4 and 5), and a mode:
  - **Hold**: Discord is muted while the key is held down;
  - **Toggle**: each press mutes or unmutes Discord.
- Installed games detected: Steam (Windows, Linux, Flatpak, Snap) and Epic (Windows, Heroic on Linux), with their icons; games with a preset are ready in one click.
- 50 presets for the most played multiplayer games with voice chat. Every field stays editable, and "Restore the defaults" brings the preset back.
- A profile only applies while its game is in the foreground. Leaving the game, pausing or quitting gives the microphone back.
- The application only unmutes what it muted: with the Application method or the default "Push to Mute" keybind, a mute you set yourself in Discord is left alone. If the application stops while Discord is muted (crash, forced exit), Discord is unmuted at the next start.
- Two ways to mute Discord:
  - **Keyboard shortcut**: a Discord keybind, "Push to Mute" (recommended) or "Toggle Mute", F13 by default, a key no game uses. The application types it for you while Discord records it;
  - **Application**: Discord's local RPC API, with your own application from the [Discord Developer Portal](https://discord.com/developers/applications).
- English and French, the system language by default.

## Installation

Download the installer from the [releases](https://github.com/powange/discord-auto-mute/releases), or see the [website](https://powange.github.io/discord-auto-mute/).

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

## Releases and updates

**Release**: bump `version` in the root `Cargo.toml`, then push a matching tag (`v0.2.0`). CI checks that the tag matches the version, runs the tests, builds the Windows setup and the Linux packages, signs them for the updater and publishes the release with `SHA256SUMS.txt` and `latest.json`.

The application checks `latest.json` once a day (setting in the General tab, or "Check for updates" in the tray menu) and installs an update only after the user confirms it, on Windows and with the AppImage (the `.deb` is updated by reinstalling it).

Updates are only installed when they are signed with the project's update key, whose public part is in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). The private key and its password are the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Keep a backup of both: without them, installed versions can no longer update themselves.

## License

MIT
