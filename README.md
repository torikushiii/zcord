# zcord

zcord is just a fun project that I created to see how far I can recreate Discord's UI
as closely as possible using [zui](https://github.com/zeronsh/zui), zeronsh's fork of
Zed's GPUI, and [gpui-component](https://github.com/zeronsh/gpui-component).

> [!WARNING]
> zcord logs in with a user account token. Third-party clients are against Discord's Terms of
> Service and accounts can be banned for using them. Use an account you can afford to lose.

## Build and run on Windows

Prerequisites: Visual Studio Build Tools with MSVC and the Windows SDK, plus rustup's stable
MSVC toolchain.

```powershell
$env:CARGO_TARGET_DIR = "C:\cargo-target\zcord"   # keep build output off network shares
$env:RUST_LOG = "zcord=info"                       # optional: connection logs in the console
cargo run
```

For scroll timing diagnostics, set `$env:RUST_LOG = "zcord=info,zcord::perf=debug"` before
launching. While smooth scrolling, the console reports callback gaps and scroll-event
dispatch times, summarized once per second and when scrolling settles. The same target also
reports per-view render rates (`renders/s`) for root, rail, channels, header, messages,
composer, members and overlays, once per second while views render. These are CPU-side
timings, not GPU timings; large gaps with short dispatch times need further frame profiling.

Paste your account token on the login screen. It is stored in Windows Credential Manager
(entry `zcord`) and sent only to Discord. The gear button in the user panel logs out and
removes it.

## Font

zcord uses Discord's font, gg sans, when it can find it, and the bundled Noto Sans otherwise.
gg sans can't be shipped with zcord; to use it, put its `.ttf`/`.otf` files in
`%APPDATA%\zcord\fonts` (`~/.config/zcord/fonts` on Linux) or install them system-wide.
Scripts neither font covers (CJK, Arabic, Thai, emoji, …) use the system's fallback fonts.

