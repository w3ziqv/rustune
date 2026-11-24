# Installation

## Prebuilt release

On x86_64 GNU/Linux with glibc 2.39 or newer, run this command as your regular
user. It uses Bash, curl, Python 3.9 or newer, and a working HTTPS certificate
store.

```bash
bash -c 'set -eu
work=$(mktemp -d)
cleanup() { rm -f -- "$work/install.py"; rmdir -- "$work"; }
trap cleanup EXIT
curl --disable --fail --silent --show-error --location --proto "=https" --proto-redir "=https" --connect-timeout 15 --max-time 60 --max-filesize 65536 \
  https://raw.githubusercontent.com/w3ziqv/rustune/v0.5.1/scripts/install.py -o "$work/install.py"
python3 "$work/install.py"'
```

This installs `rustune` and its matching `rustune-audio` in `~/.local/bin`. The
installer downloads the archive from the v0.5.1 GitHub Release, checks its
SHA-256 against the value pinned in the installer, and tests both executables
before switching the active pair. Run `~/.local/bin/rustune doctor` after
installation. To use the short `rustune` command, put `~/.local/bin` before
any older installation on your PATH.

The Ubuntu 24.04 release uses glibc (ELF symbol requirement `GLIBC_2.39`),
`libgcc_s.so.1`, `libstdc++.so.6`, ALSA `libasound.so.2`, and OpenSSL 3
(`libssl.so.3`, `libcrypto.so.3`). On Ubuntu 24.04 the corresponding packages
include `libc6`, `libgcc-s1`, `libstdc++6`, `libasound2t64` and `libssl3t64`.
Use a UTF-8 terminal, a session D-Bus for desktop controls, an ALSA-compatible
output and the playback prerequisites below. The archive supports x86_64 glibc
only. For other Linux targets, try the source build below with target-native
development libraries and verify the result with `doctor`.

### Update or migrate a manual install

Close the TUI and run `rustune ctl shutdown` if the service is running. If you
use a systemd user unit, stop it with `systemctl --user stop rustune.service`
before upgrading. The installer refuses to proceed while a Rustune process is
active for your user. It leaves process control to you.

Run the install command from the release you want. Managed installs switch
both binaries together through `~/.local/bin/.rustune-managed/current`.
Previous pairs stay in `~/.local/bin/.rustune-managed/releases`.

If you copied the two binaries into `~/.local/bin` by hand, move both into
a backup directory before running the installer. This also applies to old
symlinks at those paths. For binaries installed elsewhere, check that
`command -v rustune` resolves to the install you intend to use.

The installer needs neither sudo nor a compiler. Your PATH, shell files,
configuration, token, queue and CDM stay as they were. Set up playback
separately using the steps below.

## Build from source

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
Run `rustune doctor` or `rustune doctor --json` to check the local installation.
Settings → Audio information displays the same checks. The protocol probe runs
before CDM loading or network access. Install a matching pair of binaries if
the helper is missing, incompatible or fails to answer within one second.
Never add tokens, private caches or the CDM to source control.

To update, close the old TUI, run `rustune ctl shutdown` if a service is running,
replace both executables, then reopen `rustune`. Use the install command above for managed installations.
Queue/configuration/token files
remain in their XDG directories. Already running processes use their old executable
until stopped. Avoid replacing an executable during active playback.

For explicit service operation, use `rustune daemon` or install
`packaging/rustune.service` as a systemd user unit. Packaging examples are
starting points, not a published AUR package.

## Package the release

On Ubuntu 24.04 x86_64, install the build dependencies above, then use
`scripts/build-slim-audio.sh` and `scripts/package-release.sh target/release dist`.
If you set `CARGO_TARGET_DIR`, pass its `release` directory instead. The packer
checks version, ELF architecture and slim linking. It fixes tar entry order,
timestamps, ownership and modes and omits the gzip timestamp and filename.
Packaging the same binaries with the same GNU tar and gzip produces identical
archive bytes. Run `sha256sum -c` from the output directory. The binary archive's
digest must match `SHA256` in `scripts/install.py` at the release tag.

The release's audio helper uses FFmpeg 9.0.2 under the LGPL (2.1 or later).
Publish the exact `ffmpeg-9.0.2.tar.xz` downloaded by the build script alongside
the binaries and checksum. The source archive includes the license. The tagged
Rustune source and `scripts/build-slim-audio.sh` contain the application source
and configure options needed to rebuild and relink with a compatible FFmpeg
build. To relink against changes, edit the unpacked FFmpeg source in a fresh
build cache after verifying and extracting the pinned source archive, then run
the build script with that cache before its FFmpeg install directory exists.
