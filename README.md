# zcord

A native Discord client for Windows (also builds on Linux and macOS), written in Rust on
[zui](https://github.com/zeronsh/zui), zeronsh's fork of Zed's GPUI, and
[gpui-component](https://github.com/zeronsh/gpui-component).

> [!WARNING]
> zcord logs in with a user account token. Third-party clients are against Discord's Terms of
> Service and accounts can be banned for using them. Use an account you can afford to lose.

## Status

- Live, **read-only** connection: servers, channels you can see, message history, and live
  new/edited/deleted messages. It never sends messages, read markers, or typing indicators.
- Discord-style UI with a custom title bar and Discord's zoom levels (`Ctrl +` / `Ctrl -` /
  `Ctrl 0`).
- Offline demo data from the login screen.
- Not yet: images and avatars, markdown, sending, DMs, member list. Voice is out of scope.

## Build and run on Windows

Prerequisites: Visual Studio Build Tools with MSVC and the Windows SDK, plus rustup's stable
MSVC toolchain.

```powershell
$env:CARGO_TARGET_DIR = "C:\cargo-target\zcord"   # keep build output off network shares
$env:RUST_LOG = "zcord=info"                       # optional: connection logs in the console
cargo run
```

Paste your account token on the login screen. It is stored in Windows Credential Manager
(entry `zcord`) and sent only to Discord. The gear button in the user panel logs out and
removes it.

## Layout

- `src/main.rs` — bootstrap, zoom shortcuts, logging
- `src/app.rs`, `src/app/` — root view and UI panels
- `src/discord.rs`, `src/discord/` — Discord protocol: gateway (zlib-stream, heartbeat, resume),
  REST, payload models, permissions, settings proto
- `src/store.rs`, `src/store/` — view model built from READY and gateway events, demo data
- `src/images.rs` — CDN image disk cache
- `src/credentials.rs` — token storage in the OS credential store
- `assets/icons/` — Lucide icons (ISC) and the Discord logo (Simple Icons, CC0)
