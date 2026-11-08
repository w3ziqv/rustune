# Installation — Linux only

Rustune currently ships as experimental source. A release build is not proof of
full-song DRM playback. No verified v0.1.0 binary release is advertised yet.

On Arch Linux, install Rust 1.99.1 (minimum 1.99), D-Bus and your normal
audio stack. Typical packages are `rustup`, `firefox`, `dbus`, `pipewire` and
`wireplumber`; PipeWire-Pulse may be needed by browser audio on your setup.
Do not replace a working audio stack merely to install this client.

```sh
rustup default stable
git clone https://github.com/w3ziqv/rustune.git
cd rustune
cargo build --locked --release
install -Dm755 target/release/rustune ~/.local/bin/rustune
rustune
```

Ensure `~/.local/bin` is in PATH. The default disables playback: a native Linux
Apple Music engine is not implemented. Optionally follow [Firefox setup](firefox.md)
to explicitly select the legacy browser controller. It requires Firefox. Normal TUI usage is
`rustune`; `q` quits. `daemon = true` keeps a newly started service running.
A TUI attached to an existing service leaves it running on exit.

Optional dedicated Chromium backends need an EME/Widevine-capable browser and
codecs. Chromium is not assumed to include a functioning CDM. Never disable
sandboxing, TLS checks or web security. The minimal MusicKit host additionally
requires an Apple Music developer JWT in `~/.config/rustune/developer-token`
with mode 0600, following Apple's official token-generation instructions.
Music User Tokens cannot substitute for developer JWTs.

For systemd, copy `packaging/rustune.service` to `~/.config/systemd/user/`, adjust
the executable path if needed, then use `systemctl --user daemon-reload` and
`systemctl --user enable --now rustune`. The Firefox backend still requires the
browser and tab to remain open. This is not an independent audio decoder daemon.

Use `rustune doctor` for dependency/readiness checks, `rustune stats` for process
measurements, `rustune ctl --help` for remote terminal controls. SSH to the same
host/user can control its service; no network control listener is enabled.

`packaging/PKGBUILD.example` is a starting point for a future AUR package,
not a published or verified AUR release. No package registry upload is performed.
