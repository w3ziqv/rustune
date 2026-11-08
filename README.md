# Rustune

A native Rust Apple Music terminal controller for Linux, built with Ratatui.

**Experimental. Native Apple Music playback without a browser is not implemented.**
The target is an independent Rust player using the listener's own credentials,
with no Music Assistant dependency. A Music User Token authenticates an account;
it does not supply a Linux DRM playback engine. The default backend is disabled
and never starts a browser fallback. See [native feasibility](docs/feasibility.md).

Optional legacy Firefox/MusicKit and Chromium controllers remain available when
explicitly selected. Earlier interactive testing completed a subscribed 113-second
song through Firefox 159.0.1 on Arch Linux, with preview mode false. That validates
the browser controller only, not native decoding or this updated build. TUI
pause/resume and authorized personal reads were also observed. Remaining controls
have automated coverage; live MPRIS/next/previous/sleep verification is incomplete.

## Implemented

- Native terminal UI with responsive large/compact layouts, visible list slices,
  search input, queue, progress/volume and explicit capability screens.
- Real catalog song/album/artist/playlist search and paginated library, album,
  artist, playlist and recent-content request paths. Personal data requires
  authorization; these request paths were verified on an authorized account.
- MusicKit full-song commands, authorization/preview guards, play/pause,
  next/previous, seek, volume and mute. Full-song playback was verified with the Firefox bridge.
- Persistent bounded queue, add/play-next/remove/reorder/clear, shuffle, repeat
  and sleep timer. Queue selection commits after playback success.
- Private playback service, reconnecting TUI/CLI, MPRIS metadata and controls,
  optional desktop notifications, XDG configuration and atomic private state.
- Pure black/white default, optional Tokyo Night and other themes, custom colors,
  configurable keybindings, mouse navigation,
  command palette and Help. Three real resource profiles vary cache/page/update
  limits; idle UI redraws are event-driven.
- Lazy real album artwork in Now Playing and wide layouts, with bounded image
  cache and halfblock/Kitty/iTerm2 rendering. Minimal mode disables artwork.
- Clearly labeled MusicKit bitrate target when reported; source format stays unknown.
- `doctor`, `stats`, `benchmark`, CLI/SSH controls and systemd example.
- Bounded IPC, private files/socket, peer UID checking, session locks, no own
  telemetry and credential-free logging.

## Install on Arch Linux

Use stable Rust 1.99.1 and D-Bus. Firefox, protected-content support, a subscription
and an audio stack are needed only for the optional browser controller. Rustune
itself does not decode DRM audio.

```sh
git clone https://github.com/w3ziqv/rustune.git
cd rustune
cargo build --locked --release
install -Dm755 target/release/rustune ~/.local/bin/rustune
rustune
```

Add `~/.local/bin` to PATH if needed. The current minimum Rust version is 1.99; the toolchain is pinned to 1.99.1.
See [installation](docs/installation.md).

## Token login

Run `rustune login`, paste your **Music User Token** into the hidden prompt, and
press Enter. In the TUI, press `L` or enter `:login`. Esc cancels; Ctrl-U clears.
The token is saved atomically to `~/.config/rustune/music-user-token` with mode
0600. It never enters command arguments, command history or service IPC.
`rustune logout` / `:logout` remove this local file.

**This implements local credential entry/storage, not verified account sign-in
or native playback.** The outcome says account acceptance is unverified; the
native Linux DRM engine is still unavailable. No browser fallback is started.

The main player follows the selected amcli-style composition: large album cover
left, metadata and centered lyrics right, progress and bordered controls below.
Tab opens navigation. Existing black/white and optional theme choices remain.

## Optional legacy Firefox setup

These steps explicitly select browser playback; they do not provide native playback.

1. Keep your signed-in `https://music.apple.com/` tab open in Firefox 128+.
2. Run `rustune firefox-setup`.
3. Open `about:debugging#/runtime/this-firefox`, choose **Load Temporary Add-on…**,
   and select `~/.local/share/rustune/firefox/manifest.json`. Ctrl+L in the file
   picker accepts its absolute path. Allow access to the Apple Music host when
   prompted.
4. Run `rustune login --browser`, then `rustune`.

The development add-on is removed by Firefox restart. A permanent add-on needs
Mozilla signing; signing enforcement is never disabled. After updating add-on
files, use **Reload** on its debugging card. Click its toolbar action to reconnect
if the pinned music tab was closed. See [Firefox details](docs/firefox.md).

The optional browser controllers keep their own official authorization.
`rustune login --browser` or `:connect-browser` explicitly connects them; local
token login does not. For the
optional `music_kit` backend, a MusicKit **developer JWT** is a separate credential
stored privately at `~/.config/rustune/developer-token` with mode 0600. Follow
[Apple's token documentation](https://developer.apple.com/documentation/applemusicapi/generating-developer-tokens);
never put a private key or Apple ID password in the repository or chat.

## Controls

| Key | Action |
|---|---|
| Space | Play/pause |
| n / p | Next / previous |
| / | Search; F2 selects resource type while typing |
| j / k, Enter | Navigate / play or open |
| Tab, Esc | Switch panel / navigation |
| + / -, m | Volume / mute |
| Left / Right | Seek 10 seconds |
| a / A | Add to queue / play next |
| Delete, J / K | Remove / reorder queue item |
| PgDn | Next result/queue page |
| s / r | Shuffle / repeat |
| L | Hidden local token login |
| l / R | Lyrics / reload lyrics |
| e / v | EQ / visualizer capability explanation |
| , / ? | Settings / Help |
| : | Command palette |
| q, Ctrl-C | Quit terminal client |

Commands include `:login`, `:logout`, `:profile minimal`, `:theme nord`,
`:sleep 30` (minutes), `:sleep 0`, `:recent`, `:history`, `:clear`, `:queue`.
All standard bindings are configurable. Help shows the action names.

```text
 RUSTUNE / Home    Balanced
──────────────────────────────────────────────────────────
 • Home          │ YOUR MUSIC. YOUR TERMINAL.
   Now Playing   │
   Search        │ Native MusicKit controls in your terminal
   Library       │
   Queue         │ / Search · Tab navigation · ? Help
──────────────────────────────────────────────────────────
 No track playing                              vol 70%
                     0:00 / 0:00
```

This compact terminal example illustrates the layout. `rustune render --width
100 --height 28` produces an actual current screen without invented track data.

## Configuration and service

Configuration lives at `~/.config/rustune/config.toml`. Data, private browser
profiles and search/queue state use XDG data directories. See
[configuration](docs/configuration.md).

```toml
backend = "disabled"
performance = "balanced"
theme = "monochrome"
artwork = true
lyrics = true
artwork_protocol = "auto"
volume = 0.7
daemon = true
```

`daemon = true` keeps an automatically started service alive when the TUI closes.
A TUI connected to an existing service leaves it running. `rustune daemon` runs
in the foreground for systemd. The Firefox tab/browser still needs to be open.

```sh
rustune doctor
rustune status
rustune ctl pause
rustune ctl volume 0.5
rustune ctl seek 20
rustune ctl sleep 1800  # CLI units are seconds
rustune stats --seconds 5
rustune benchmark --seconds 30
rustune ctl stop
```

SSH to the same host/user can run the CLI. MPRIS uses
`org.mpris.MediaPlayer2.rustune` for compatible media keys, playerctl and Waybar.
No network control API is enabled.

## Audio limits and planned features

Only **Auto** quality is enabled. Codec, source bitrate, sample rate, bit depth,
channels and buffer status remain **Unknown** without reliable information.
Lossless/Hi-Res Lossless and explicit quality presets are disabled. PipeWire's
output format does not establish the source encoding. MusicKit targets of 64/256
kb/s are labeled as targets, never measured source bitrate.

EQ, preamp, bass/treble, normalization, crossfade, gapless guarantees
and a real visualizer are not implemented. Their screens explain why.

Free LRCLIB lyrics work without an API key. Requests send the track title, artist,
album and duration to LRCLIB, never Apple credentials. Synced LRC follows reported
playback position; plain-text and instrumental records have distinct states.
Lyrics wrap, scroll and use a bounded cache. `lyrics = false` disables lookup.
Coverage depends on LRCLIB; no match is reported honestly, never substituted with
a different song. See [lyrics details](docs/lyrics.md).
Real artwork is rendered lazily from Apple image hosts when the connected backend
reports its URL. No generated artwork is substituted. `auto` uses portable
halfblocks; image protocols can be selected explicitly.
Favorites browsing has no verified public endpoint integration here. Direct
output-device/buffer controls are external PipeWire tools. These are limitations,
not working features behind decorative controls.

See [audio support](docs/audio-support.md), [feasibility research](docs/feasibility.md)
and [validation evidence](docs/validation.md).

## Resources and privacy

Chromium starts lazily and is reused. The Firefox backend attaches to a user-owned
session. `stats` reports native, browser and total RSS/PSS separately; **shared
Firefox includes all its tabs**, not just incremental Rustune cost. CPU is sampled
from `/proc`, not guessed. See [measurements](docs/benchmarking.md); active playback
and long-session numbers require actual playback and are not fabricated.

Rustune adds no telemetry, ads or analytics. Apple's website, MusicKit, browser
and DRM components have their own networking and privacy behavior. Account
cookies stay in Firefox or the dedicated Chromium profile. Protect these private
directories and exclude them from public backups. Logout uses MusicKit
unauthorization; it does not promise an Apple ID-wide sign-out.

## Develop

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
# Opt-in real browser/CDN/catalog checks; no account playback assertion:
cargo test --test browser -- --ignored --test-threads=1
# Firefox extension contract test (Node 18+; mocked extension APIs only):
node tests/extension.mjs
node tests/musickit.mjs
```

D-Bus tests need `dbus-daemon` and `busctl`. CI runs formatting, Clippy, tests and
release validation. See [architecture](docs/architecture.md),
[contributing](CONTRIBUTING.md), [skill record](docs/skills.md), and [AGENTS.md](AGENTS.md).
MIT-licensed application code; MusicKit, browsers and Widevine remain separate
components with their own terms. No third-party player application code is copied.
