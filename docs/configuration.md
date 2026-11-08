# Configuration

`rustune init` writes defaults and preserves an existing file. Configuration is
`$XDG_CONFIG_HOME/rustune/config.toml` (normally `~/.config/rustune/config.toml`).
Queue/history use XDG data; artwork/lyrics use bounded caches. Service IPC lives
under XDG runtime, with a private data-directory fallback.

```toml
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

The native playback engine is not implemented. Source quality remains unknown;
transport, catalog, DSP and nonzero sleep timer requests report unavailability.
Queue edits do not auto-play music. The service publishes changes through private
IPC; it does not poll a browser or keep an idle playback refresh timer.

`lyrics = false` disables anonymous LRCLIB lookup; `artwork = false` disables
cover fetching. Both use song metadata rather than credentials.
