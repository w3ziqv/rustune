# Configuration

`rustune init` writes defaults and preserves an existing file. Configuration is
`$XDG_CONFIG_HOME/rustune/config.toml` (normally `~/.config/rustune/config.toml`).
Queue/history use XDG data; artwork/lyrics use bounded caches. Service IPC lives
under XDG runtime, with a private data-directory fallback.

```toml
performance = "balanced"
catalog_country = "PL"
theme = "graphite"
artwork = true
lyrics = true
artwork_protocol = "auto"
artwork_style = "pixelated"
volume = 0.7
daemon = false
notifications = false
mouse = true
logging = false
restore_queue = true
max_queue = 10000
```

Retired `backend`, `browser`, `headless` and `bridge_port` settings are discarded
when an old valid configuration is loaded; other preferences are preserved and
the sanitized configuration is saved atomically with mode 0600. Unrecognized
keys, invalid values, files over 64 KiB and invalid TOML remain errors and are
not overwritten. Token and queue files are not changed by migration.

Themes: `graphite` (default), `monochrome` (pure black/white), `tokyo_night` (`tokio` alias),
`dark`, `light`, `catppuccin_mocha`, `catppuccin_latte`, `gruvbox`, `nord`, `custom`.
Custom `[colors]` accepts background, foreground, muted, accent and panel RGB hex
values. `[keybindings]` maps Help action names to keys such as `ctrl-x`, `Space`,
`PageDown` or an arrow. Unknown/duplicate bindings fail validation.

| Profile | Queue items/page | Cached lyrics | Covers | Source image limit |
|---|---:|---:|---:|---:|
| minimal | 15 | 1 | 0 | disabled |
| balanced | 25 | 8 | 1 | 256×256 |
| enhanced | 50 | 32 | 1 | 512×512 |

Artwork is lazy, limited to public HTTPS Apple image hosts, 2 MiB downloads,
5-second requests and bounded decode allocations. Rendering caps at 96×48 cells.
`artwork_style = "pixelated"` uses terminal halfblocks; `"smooth"` uses a regular
image through a supported Kitty/iTerm2 protocol with detected cell geometry.
Settings → Pixelated artwork applies the choice globally. A bounded startup
probe runs before the keyboard reader. Unsupported regular-image rendering
shows an explanation instead of silently falling back to pixels. Sixel and
unverified tmux/screen passthrough are unavailable.

When `artwork_style` is missing, old `auto`/`halfblocks` configurations retain
pixelated artwork; old explicit `kitty`/`iterm2` configurations select smooth.
An explicit style wins. `artwork_protocol` remains an advanced compatibility
constraint. Existing explicit themes are preserved.

The worker retains one bounded decoded source and the UI one encoded cover.
Resize reuses the source. Leaving artwork views or disabling artwork releases
the source and rendered resources. Minimal disables artwork regardless of style.
No profile claims to change a nonexistent refresh timer.
The fullscreen TUI explicitly enables RGB output even when `NO_COLOR` is inherited.
Half-block covers require both foreground and background colors to remain images;
the monochrome theme still uses black/white text and borders. CLI output is unaffected.

`L` / `:login` and `rustune login` show bounded dots/count and save a Music User
Token privately. Backspace edits, Ctrl-U clears, Esc cancels, Enter saves.
`:logout` / `rustune logout` removes only the local token. Local storage does not
verify Apple account acceptance. No browser login or native messaging remains.

Native playback requires the sibling `rustune-audio` helper, private token and
compatible CDM. Run `rustune doctor` for local checks and repair steps, or
`rustune doctor --json` for the same report as JSON. Settings → Audio information
shows the checks in the TUI. Use ↑/↓ to scroll and R to refresh. The checks
inspect token and CDM file presence, negotiate with the helper beside the
frontend and probe the local service and session D-Bus. A running service reports
the helper protocol it expects. If that version differs or is unreported,
restart the service with a matching pair of binaries. The checks run without
Apple requests or a playback attempt. Source quality is unknown until the decoder reports it. Accepted
account tokens enable Library, Search → My library and Playlists. Explicit account
writes, confirmations and journal limits are described in [playlists](playlists.md).
Queue edits do not auto-play music. The service publishes changes through private
IPC; it does not poll a browser or keep an idle playback refresh timer.

`lyrics = false` disables anonymous LRCLIB lookup; `artwork = false` disables
cover fetching. Both use song metadata rather than credentials.

Keyboard navigation: ↑/↓ selects, Enter/→ opens/confirms, ← returns from album
tracks, Settings or a lyrics preview to previous results; Esc does the same and
opens navigation when there is no previous view. Tab cycles
focus; player buttons use ←/→ and Enter. `/` opens the search field; Tab/F2
changes songs/albums/artists/My library. PgUp/PgDn changes pages (public queries cap at 200
results). Settings exposes token entry/removal, themes, resource profiles,
artwork visibility/style, lyrics, public catalog country, queue shuffle/repeat,
Equalizer, Audio information, Help and Quit. Audio information is read-only: the
backend selects stream quality. `e` opens Settings → Equalizer directly.

The seven numeric routes are Search (1), Library (2), Playlists (3), Queue (4),
Now Playing (5), Lyrics (6), Settings (7). Albums and Artists are Search categories.
`l` previews selected-song lyrics without playback; `6` follows the playing song.
On lyrics, scroll manually with arrows or the wheel, then press `f` (or click the
footer) to follow the current verse again. `R` reloads the lyrics. Lyrics has no
cover request or player-button focus.

`catalog_country` is a two-letter storefront code, normalized to uppercase.
Public requests use fixed HTTPS Apple Music catalog endpoints, anonymous
application authorization, no user token and no redirects, bounded 2 MiB
responses, at most 200 results, eight-second request timeouts and a 20-per-minute
request budget. Account region from me/storefront overrides this fallback country.
Private library requests send the token only to fixed Apple account endpoints.
The iTunes metadata fallback has a 512 KiB bound. Cached requests avoid network calls;
HTTP Retry-After is honored. A store link accompanies public result metadata.
Page cursors retain the original provider and storefront; retry a failed page
or start a new query to select a source again.

```toml
[equalizer]
enabled = false
preamp_db = 0.0
bands_db = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
```

Bands are 31 Hz, 63 Hz, 125 Hz, 250 Hz, 500 Hz, 1 kHz, 2 kHz, 4 kHz,
8 kHz and 16 kHz. All gains/preamp accept finite −12…+12 dB. Press `e`: ↑/↓
selects, ←/→ edits by 0.5 dB or cycles presets, Enter toggles/resets a row,
`b` bypasses and `r` restores Flat. Edits persist. Automatic headroom can lower
overall level for boosted bands; the UI reports the applied value. Bypass
preserves configured gains and source quality fields never change with EQ.
