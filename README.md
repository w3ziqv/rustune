# Rustune

Experimental Linux Apple Music TUI, written in Rust 2024 with Ratatui.
Default appearance: black background, white text and borders; optional Tokyo Night
and other palettes. The player composition uses large real artwork, metadata,
centered lyrics, progress and mouse/keyboard controls.

Native playback uses your saved Media User Token, anonymous public Apple
application authorization, a locally installed Widevine CDM, native AAC decoding
and ALSA/PipeWire audio. No content keys are extracted. Apple subscription and
license requirements remain enforced. Personal library/playlists authorization
is still unavailable. A subscribed complete song has been verified through the
native service and TUI; see [live validation](docs/validation.md). No release tag
is published, and the native playback work remains local.

Browser playback, browser authorization, extensions and native-messaging setup
have been removed. Rustune does not start or connect to a browser. Music Assistant
is not a dependency.

## Install

Linux, stable Rust 1.99.1 (minimum 1.99), a C++ compiler, FFmpeg 9 development
headers, ALSA development headers, D-Bus, and a UTF-8 terminal:

```sh
cargo build --release
install -Dm755 target/release/rustune ~/.local/bin/rustune
rustune init
rustune login
rustune
```

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

- Official anonymous Apple/iTunes catalog search for songs, albums and artists,
  album tracks and artist albums. It works before or after local token storage;
  it does not establish Apple Music subscription availability. Country defaults
  to PL and can be changed in Settings. Store links are shown with result metadata.

- Persistent bounded queue: select, add, remove, reorder, repeat/shuffle state,
  pagination and updates from other clients.
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
EQ, visualizer, crossfade, gapless guarantees and lossless playback are unavailable.

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
| `l` / `R` | Preview selected song lyrics / reload |
| `J` / `K`, Delete | Reorder / remove queue item |
| `:theme monochrome` / `:theme tokyo_night` | Change theme |
| `:profile minimal` / `balanced` / `enhanced` | Resource profile |
| `?`, `q`, Ctrl-C | Help / quit |

Settings are selectable with arrows and Enter. Enter on a song opens Lyrics /
Add to queue / Play actions. Player buttons receive
visible keyboard focus and activate with Enter.

Transport controls operate the native audio worker. Source codec, sample rate
and channels come from the decoder; the 256 kb/s target is labeled as reported.
Source bit depth and lossless support remain unknown/unavailable.
`daemon = false` (default) stops an automatically created service when the TUI
closes. An already running service is preserved. Use `rustune ctl stop` to stop
it deliberately, or `rustune daemon` to run it in the foreground.

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

The public metadata endpoint is the official [Apple iTunes Search API](https://performance-partners.apple.com/search-api).
It receives search terms and country, never Music User Tokens. Previews are not
played. Private Apple Music account data is not substituted with public results.
