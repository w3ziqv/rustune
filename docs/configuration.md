# Configuration

`rustune init` writes defaults and preserves an existing file. Configuration is
`$XDG_CONFIG_HOME/rustune/config.toml` (normally `~/.config/rustune/config.toml`).
Queue/history use XDG data; artwork/lyrics use bounded caches. Service IPC lives
under XDG runtime, with a private data-directory fallback.

```toml
performance = "balanced"
catalog_country = "PL"
theme = "monochrome"
artwork = true
lyrics = true
artwork_protocol = "auto"
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

Themes: `monochrome` (pure black/white default), `tokyo_night` (`tokio` alias),
`dark`, `light`, `catppuccin_mocha`, `catppuccin_latte`, `gruvbox`, `nord`, `custom`.
Custom `[colors]` accepts background, foreground, muted, accent and panel RGB hex
values. `[keybindings]` maps Help action names to keys such as `ctrl-x`, `Space`,
`PageDown` or an arrow. Unknown/duplicate bindings fail validation.

| Profile | Queue items/page | Cached lyrics | Covers | Source image limit |
|---|---:|---:|---:|---:|
| minimal | 15 | 1 | 0 | disabled |
| balanced | 25 | 8 | 4 | 256×256 |
| enhanced | 50 | 32 | 16 | 512×512 |

Artwork is lazy, limited to public HTTPS Apple image hosts, 2 MiB downloads,
5-second requests and bounded decode allocations. Rendering caps at 96×48 cells.
`auto`/`halfblocks` need no terminal query; `kitty`/`iterm2` require terminal
support. Sixel remains unavailable until pixel geometry is verified.

`L` / `:login` and `rustune login` show bounded dots/count and save a Music User
Token privately. Backspace edits, Ctrl-U clears, Esc cancels, Enter saves.
`:logout` / `rustune logout` removes only the local token. Local storage does not
verify Apple account acceptance. No browser login or native messaging remains.

Native playback requires the sibling rustune-audio helper, private token and
compatible CDM. Source quality is unknown until the decoder reports it. Accepted
account tokens enable Library and Search → My library; playlists remain unavailable.
Queue edits do not auto-play music. The service publishes changes through private
IPC; it does not poll a browser or keep an idle playback refresh timer.

`lyrics = false` disables anonymous LRCLIB lookup; `artwork = false` disables
cover fetching. Both use song metadata rather than credentials.

Keyboard navigation: ↑/↓ selects, Enter/→ opens/confirms, ← returns from album
tracks or a lyrics preview to previous results, Esc opens navigation. Tab cycles
focus; player buttons use ←/→ and Enter. Search opens a typing field; Tab/F2
changes songs/albums/artists/My library. PgUp/PgDn changes pages (public queries cap at 200
results). Settings exposes token entry/removal, themes, resource profiles,
artwork, lyrics, public catalog country, queue shuffle/repeat, Help and Quit.

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
