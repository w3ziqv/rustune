# Rustune

Experimental Linux Apple Music TUI, written in Rust 2024 with Ratatui.
Default appearance: black background, white text and borders; optional Tokyo Night
and other palettes. The player composition uses large real artwork, metadata,
centered lyrics, progress and mouse/keyboard controls.

Native playback uses your saved Media User Token, anonymous public Apple
application authorization, a locally installed Widevine CDM, native AAC decoding
and ALSA/PipeWire audio. No content keys are extracted. Apple subscription and
license requirements remain enforced. Private song-library listing and search use your accepted Media User Token;
playlist browsing, whole-playlist playback and explicit private playlist writes are available
(see [playlist actions](docs/playlists.md)). A subscribed complete song has been verified through the
native service and TUI; see [live validation](docs/validation.md). This remains
experimental; the current source release is `v0.2.0`.

Browser playback, browser authorization, extensions and native-messaging setup
have been removed. Rustune does not start or connect to a browser. Music Assistant
is not a dependency.

## Install

Linux, stable Rust 1.99.1 (minimum 1.99), a C++ compiler, make, pkg-config,
Python 3, libclang, OpenSSL/ALSA development headers, D-Bus, and a UTF-8 terminal:

```sh
scripts/build-slim-audio.sh
install -Dm755 target/release/rustune ~/.local/bin/rustune
install -Dm755 target/release/rustune-audio ~/.local/bin/rustune-audio
rustune init
rustune login
rustune
```

The script builds both executables and checksum-pinned minimal FFmpeg9.0.2
privately, preserving verified HTTPS and leaving system libraries unchanged.
Alternatively, `cargo build --locked --release` uses system FFmpeg9 development
libraries. The two executables must remain beside each other. See the
[installation guide](docs/installation.md) for distro dependencies and updates.

Install a compatible signed `libwidevinecdm.so` privately at
`~/.cache/rustune/drm/libwidevinecdm.so` (directory 0700, file 0600). The proprietary
module is not redistributed in this repository. The application token is
retrieved anonymously from Apple and cached privately; no browser session or
cookies are read. Only your Media User Token is entered by you.

Login shows bounded dots and a character count while you type or paste. Backspace
removes a character; Ctrl-U clears; Enter saves; Esc cancels. The token is never
echoed. The explicit outcome says whether private local storage succeeded; it
never claims remote authentication. `rustune logout` removes the saved local file.

Files follow XDG directories. The token is `~/.config/rustune/music-user-token`
with mode 0600. Old browser-specific configuration is discarded on load while
valid preferences, queue data and token files are preserved.

## Available features

- Apple Music catalog search for songs, albums and artists, album tracks and
  artist albums, with bounded ranking for combined title/artist queries. An
  accepted user token selects your account region and enables Library and
  Search → My library. Without one, public search uses the configured country
  (PL by default). Public catalog requests do not include your user token.
  An explicitly labeled iTunes fallback is used if the Music catalog fails.

- Library playlist browsing and complete ordered playback, private creation from an
  empty list or the full queue, and explicit append to editable playlists.
  [Playlist controls and limits](docs/playlists.md).

- Complete album playback and stable shuffled sessions across every track page;
  album actions replace, play next or append the whole album atomically.
- Persistent bounded queue with distinct occurrence IDs, FIFO play-next blocks,
  listening history, revision checks, clear-upcoming and one-step undo.
  See [album and queue semantics](docs/queue.md).
- Free LRCLIB lyrics without an API key: synced/plain/instrumental states,
  wrapping, manual scrolling, retry and a bounded cache. Select a saved queue
  song and press `l` for a silent lyrics/cover preview.
- Lazy bounded album artwork from Apple image URLs, rendered using half blocks
  or explicit Kitty/iTerm2 protocols. Minimal mode disables artwork.
- Private Unix-socket service, configurable keys/themes and MPRIS capabilities
  with native playback controls.

LRCLIB receives only song title, artist, album and duration. Coverage varies;
missing lyrics are reported honestly. No Apple credentials are sent to LRCLIB
or artwork hosts. No generated artwork or invented audio quality is substituted.
A real ten-band stereo equalizer provides preamp, Flat/Bass/Vocal/Treble presets,
click-smoothed live edits and automatic boost headroom. Bypass preserves the gains.
Visualizer, crossfade, gapless guarantees and lossless playback are unavailable.

## Controls

| Key | Action |
|---|---|
| `L` / `:login` | Enter a token with masked feedback |
| `:logout` | Remove the local token |
| `8` / `:queue` | Queue |
| ↑ / ↓, `j` / `k` | Move within the focused menu/list; scroll lyrics |
| Enter / → | Open a screen/result or confirm an action |
| ← / Esc | Back to previous results / navigation menu |
| Tab | Cycle menu, content and player-button focus; change search type while typing |
| PgUp / PgDn | Previous / next result or queue page |
| Alt-← / Alt-→ | Seek when a playback engine is available |
| `e` | Equalizer: ↑/↓ select, ←/→ edit, `b` bypass, `r` Flat |
| `l` / `R` | Preview selected song lyrics / reload |
| `a` on an album / its details | Whole-album actions (Enter still opens the album) |
| `J` / `K`, Delete | Reorder / remove occurrence and advance if current |
| `:clear`, `u` in Queue / `:undo` | Clear upcoming / undo last replacement or clear |
| `c` / `:cancel`, `R` / `:retry` | Cancel preparation / retry album or queue operation (R on Lyrics reloads lyrics) |
| `:stop`, `:shutdown` | Stop audio / close service |
| `:theme monochrome` / `:theme tokyo_night` | Change theme |
| `:profile minimal` / `balanced` / `enhanced` | Resource profile |
| `?`, `q`, Ctrl-C | Help / quit |

Settings are selectable with arrows and Enter. Enter on a song opens Lyrics /
Append / Play song (replaces queue) / Play next actions. Album song playback
retains its full album context. Player buttons receive
visible keyboard focus and activate with Enter.

Transport controls operate the native audio worker. Source codec, sample rate
and channels come from the decoder; the 256 kb/s target is labeled as reported.
Source bit depth and lossless support remain unknown/unavailable.
`daemon = false` (default) stops an automatically created service when the TUI
closes. An already running service is preserved. Use `rustune ctl shutdown` to close
it deliberately (`rustune ctl stop` stops only audio), or `rustune daemon` to run it in the foreground.

## Validation

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Optional network/PTY tests use isolated XDG paths, private D-Bus and artificial
tokens. They exercise real public lyrics/artwork, never Apple account acceptance
or live audio. See [validation](docs/validation.md), [configuration](docs/configuration.md),
[architecture](docs/architecture.md) and [measurements](docs/benchmarking.md).
MIT licensed.

Public metadata uses Apple Music catalog endpoints with anonymous application
authorization, plus the [iTunes Search API](https://performance-partners.apple.com/search-api)
as a fallback. Only fixed Apple account endpoints receive your private user token.
Private-library errors remain explicit; public results never substitute for your library.
Previews are not played.
