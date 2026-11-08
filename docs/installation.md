# Installation

Linux only. Install stable Rust 1.99.1, D-Bus and a UTF-8 terminal.

```sh
cargo build --release
install -Dm755 target/release/rustune ~/.local/bin/rustune
rustune init
rustune login
rustune
```

Token entry shows dots and a character count. Enter saves privately; Esc cancels.
Local token storage does not verify Apple account acceptance. Native Apple Music
playback is not implemented; no browser, extension or Music Assistant is used.

To reload an installed update, close the TUI, run `rustune ctl stop` if a service
is running, then open `rustune` again. Queue state is preserved. Existing service
processes continue using their old executable until stopped.

For explicit service operation, use `rustune daemon` or install
`packaging/rustune.service` as a systemd user unit. This keeps the private queue/IPC
service running; it does not provide an audio engine. Packaging examples are
starting points, not a published AUR package.
