# Configuration

Run `rustune init` to write defaults, or `rustune firefox-setup` for the local
Firefox bridge. Existing config files are preserved by `init`. Paths follow XDG:

- Config: `$XDG_CONFIG_HOME/rustune` or `~/.config/rustune`.
- Data and browser profile: `$XDG_DATA_HOME/rustune` or `~/.local/share/rustune`.
- Cache directory: `$XDG_CACHE_HOME/rustune` or `~/.cache/rustune`.
- IPC: `$XDG_RUNTIME_DIR/rustune`, with a private data-directory fallback.

Example `config.toml`:

```toml
backend = "disabled"
performance = "balanced"
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
headless = true
bridge_port = 8973

[keybindings]
quit = "ctrl-x"
```

Backends: `disabled` (default; native playback unavailable, no browser fallback),
`firefox` (explicit existing-session controller), `music_kit` (minimal local host with your
private developer JWT), `apple_web` (dedicated Chromium official web player),
`auto` (minimal host when a JWT is present, otherwise the Chromium web player).
`browser` can specify a Chromium executable for the Chromium backends. `headless`
and `bridge_port` affect Chromium/local MusicKit only. Restart the service when
changing backend/browser settings.

Themes: `dark`, `light`, `catppuccin_mocha`, `catppuccin_latte`, `gruvbox`,
`nord`, `monochrome` (default), `tokyo_night` (`tokio` alias), `custom`. For a custom theme:

```toml
theme = "custom"
[colors]
background = "#16161e"
foreground = "#d8dee9"
muted = "#8892a6"
accent = "#88c0d0"
panel = "#24283b"
```

All default actions listed in Help can be rebound by action name. Key names
include a single character, `Space`, `Enter`, `Esc`, `Tab`, `Delete`, `PageDown`,
arrow names, `ctrl-x`, or `alt-x`. Duplicate bindings and unknown actions fail
validation. Ctrl-C always exits. Input mode accepts text separately from player
bindings. F2 chooses search resource type.

Resource profiles change actual metadata cache capacity, page size and active
browser snapshot interval:

| Mode | Cached pages | Items/page | Snapshot interval | Covers | Max cover pixels |
|---|---:|---:|---:|---:|---:|
| minimal | 1 | 15 | 2000 ms | 0 | 0 |
| balanced | 8 | 25 | 1000 ms | 4 | 256 × 256 |
| enhanced | 32 | 50 | 500 ms | 16 | 512 × 512 |

Artwork is fetched only for visible cover areas. Downloads are HTTPS-only from
Apple `*.mzstatic.com` hosts, without credentials or redirects, bounded to 2 MiB
and a 5-second request timeout. Decode limits cap dimensions at 2048 × 2048 and
then resize to the profile limit. `artwork = false` disables fetching. `auto` and
`halfblocks` need no terminal query; `kitty` and `iterm2` are explicit
protocol choices requiring terminal support. No chafa dependency is enabled.

Profiles do not invent visualizer or DSP support. Without a connected
browser or timer, the service has no refresh timer. The TUI subscribes to changes
and redraws on events. Page caches are bounded FIFO with replacement.

`:profile MODE`, `:theme NAME`, `:sleep MINUTES` (0 cancels), `:history`,
`:recent`, `:queue`, `:clear`, `:audio`, `:login`, `:logout` are available in the
command palette. The CLI sleep control uses seconds. Queue edits/search history
are persisted atomically in `state.json`; restoration does not auto-play audio.

Invalid TOML, unknown keys, out-of-range values and oversized files produce an
error and preserve the original file. Rustune does not overwrite invalid config
with defaults. Logs are opt-in and contain no credentials or raw CDP expressions.


Token entry: `rustune login` or TUI `L` / `:login` saves a Music User Token locally.
`rustune logout` / `:logout` removes it. Storage is private and unverified; no
native playback engine or browser authorization is implied. For the explicit
legacy controller use `rustune login --browser` / `:connect-browser`.

`lyrics = true` enables lazy anonymous LRCLIB lookup when Now Playing or Lyrics
is visible. It sends only song metadata, honors service throttling and caches a
bounded number of records according to the profile. R requests a fresh lookup;
`lyrics = false` prevents new lookups. Artwork rendering is capped to 96 × 48
terminal cells; the source download/decode still follows the profile pixel limit.
