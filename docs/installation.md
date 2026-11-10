# Installation

Linux only, stable Rust1.99.1 (minimum1.99), a UTF-8 terminal and D-Bus.
The source release does not contain credentials or a proprietary CDM.

On Debian/Ubuntu, install the native build dependencies:

```sh
sudo apt-get update
sudo apt-get install build-essential pkg-config python3 libasound2-dev libssl-dev libclang-dev dbus
```

On Arch Linux:

```sh
sudo pacman -S --needed base-devel python clang alsa-lib openssl dbus
```

Build from the repository root:

```sh
scripts/build-slim-audio.sh
install -Dm755 target/release/rustune ~/.local/bin/rustune
install -Dm755 target/release/rustune-audio ~/.local/bin/rustune-audio
rustune init
rustune login
rustune
```

The script downloads and verifies checksum-pinned FFmpeg9.0.2, builds a private
AAC/MOV/HTTPS subset and both Rust executables. It requires network access for
public source/dependencies. Artifacts live under `~/.cache/rustune-build` or
`RUSTUNE_BUILD_CACHE`; system FFmpeg is unchanged. To use installed FFmpeg9
libraries instead, install its development headers and use
`cargo build --locked --release`. Keep `rustune-audio` beside `rustune`.

Install your compatible signed `libwidevinecdm.so` at
`~/.cache/rustune/drm/libwidevinecdm.so` (directory0700, file0600). This proprietary
module is not redistributed or installed by the build script. Playback requires
an active Apple Music subscription and an accepted Media User Token. Application
authorization is fetched anonymously from Apple's public site and cached privately.
No browser or Music Assistant runtime is used.

Token entry shows bounded dots and a character count. Enter saves privately;
Esc cancels. Local token storage does not verify Apple account acceptance.
`rustune doctor` checks local prerequisites; Play checks real authorization.
Never add tokens, private caches or the CDM to source control.

To update, close the old TUI, run `rustune ctl stop` if a service is running,
replace both executables, then reopen `rustune`. Queue/configuration/token files
remain in their XDG directories. Already running processes use their old executable
until stopped. Avoid replacing an executable during active playback.

For explicit service operation, use `rustune daemon` or install
`packaging/rustune.service` as a systemd user unit. Packaging examples are
starting points, not a published AUR package.
