# zcord

A native Discord client for Windows, written in Rust. It is not Electron and not a web view:
the whole UI is drawn on the GPU with [zui](https://github.com/zeronsh/zui), zeronsh's fork of
Zed's GPUI, and [gpui-component](https://github.com/zeronsh/gpui-component). It also builds on
Linux and macOS, but Windows is the platform it is made and tested for.

zcord aims to look and feel like the official desktop client while starting faster and using
far less memory. Text chat is the focus today; voice comes later.

> [!IMPORTANT]
> Third-party clients are against Discord's Terms of Service, and accounts can be disabled for
> using them. zcord is also early and incomplete, so expect bugs and missing features. It tries
> hard to look like the official client, but there is no guarantee.
> **Use it at your own risk**, ideally on an account you can afford to lose.

## Features

- **Servers:** folders, categories, channels you can see, threads, forum channels, server
  banners, unread and mention indicators.
- **Messages:** send, reply, edit, delete, typing indicators, and read markers. Full Discord
  markdown, mentions, custom and animated emoji, stickers, and reactions.
- **Rich content:** embeds, components v2, image and GIF attachments, invite previews,
  an image viewer, and inline video playback (Windows).
- **Pickers:** emoji picker with `:shortcode:` suggestions, and a GIF picker with your
  favorites synced from Discord.
- **People:** live member list with roles, presence and activities; profile popouts and full
  profiles, including avatar decorations, nameplates, profile effects and themes.
- **Home:** friends, direct messages, group DMs, and message requests.
- **Navigation:** quick switcher (`Ctrl K` / `Ctrl T`), smooth scrolling, `Esc` to jump to the
  latest message.
- **Login:** QR code from the Discord mobile app, or a token.
- **Comfort:** custom title bar, Discord's zoom levels (`Ctrl +` / `Ctrl -` / `Ctrl 0`), and the
  window size, position and zoom are remembered. Uses Discord's gg sans font if you place it in
  `%APPDATA%\zcord\fonts` (it can't be bundled), otherwise Noto Sans.

**Planned:** voice and video calls, and screen sharing. Until then, use the official client for those.

## Logging in

### QR code (recommended)

1. Start zcord. The login screen shows a QR code next to the token field.
2. In the Discord mobile app, open **Settings → Scan QR Code** and scan it.
3. Confirm on your phone.

This gives zcord its own login session, which appears in **Settings → Devices** and can be
removed there without affecting your other devices. The QR code is hidden when the window is
too narrow; widen it to see the code.

### Token

Paste a user token into the token field and press **Log In**. A token belongs to the session
it came from, so if you copy it from a browser and later log out of that browser, zcord is
logged out too. Prefer QR login when you can.

Either way, the token is stored in the system credential store (Windows Credential Manager,
entry `zcord`) and is only ever sent to Discord. The gear button in the user panel logs out and
removes it.

## Avoiding the spam filter

zcord identifies itself to Discord as the official Windows desktop app: same request headers,
same gateway handshake, and no client-mod flags. It only makes the background requests the
official client makes. Discord still flags third-party clients more often than its own, so on
accounts you care about, do these in the official client instead:

- Joining or leaving servers
- Starting new DMs with people you haven't talked to
- Adding or removing friends
- Editing your profile or account settings
- Repeatedly restarting or reconnecting in a short time

If your account does get disabled, you can usually
[appeal](https://support.discord.com/hc/en-us/requests/new?ticket_form_id=360000029731) and
get it back.

## Building

There are no prebuilt downloads yet. On Windows you need Visual Studio Build Tools (MSVC and
the Windows SDK) and Rust's stable MSVC toolchain from [rustup](https://rustup.rs).

```powershell
git clone https://github.com/torikushiii/zcord
cd zcord
cargo run --release
```

If the repository lives on a network share, point `CARGO_TARGET_DIR` at a local folder such as
`C:\cargo-target\zcord` to keep build output off the share.

## Troubleshooting

Run zcord from a terminal with logging enabled to see why it disconnected or failed:

```powershell
$env:RUST_LOG = "zcord=info"
cargo run --release
```

Logs never contain your token. "Invalid token" means Discord ended that login session, so log
in again. Please include the log output when reporting a bug.

For scrolling and rendering performance, `$env:RUST_LOG = "zcord=info,zcord::perf=debug"`
reports per-second scroll timing and per-view render rates. These are CPU-side timings only.

Settings are kept in `%APPDATA%\zcord\settings.json`, and downloaded images are cached in
`%LOCALAPPDATA%\zcord`.

## Project layout

- `src/main.rs`: startup, keyboard shortcuts, logging
- `src/app.rs`, `src/app/`: the root view and every UI panel
- `src/discord.rs`, `src/discord/`: the Discord protocol, including the gateway (zlib-stream,
  heartbeat, resume), REST with rate limits, QR login (`remote_auth.rs`), payload models,
  permissions and the settings protobuf
- `src/store.rs`, `src/store/`: the view model built from gateway events
- `src/media/`: image cache, animated images, and video playback
- `src/text/`: markdown, emoji, and composer tokens
- `src/credentials.rs`: token storage in the system credential store
- `src/fonts.rs`, `src/settings.rs`: font loading and saved settings
- `scripts/`: generators for the packed emoji assets and the app icon

## License

zcord is licensed under the [GNU General Public License v3.0 only](LICENSE).

zcord is not affiliated with or endorsed by Discord Inc.
