# Changelog

## Unreleased — experimental source

- Linux Ratatui terminal client and private Unix-socket service.
- Persistent queue, bounded pagination/cache, MPRIS and sleep timer.
- Latest stable Rust 1.99.1, zbus 6, TOML 1.2 and current compatible dependencies.
- Hidden token login in CLI/TUI, private local storage and explicit unverified outcomes.
- Bounded terminal paste/input with private cancellation pipes and credential buffers wiped on drop.
- Free LRCLIB synced/plain lyrics, safe matching, source attribution and wrapped scrolling.
- Large-cover player layout based on the selected amcli composition.
- Pure black background and white text/borders; optional Tokyo Night and other themes.
- Bounded lazy album covers and explicit terminal image protocols.
- MusicKit bitrate targets distinguished from unknown source quality.
- Native playback is unavailable; default backend disabled with no browser fallback.
- Explicit legacy Firefox/MusicKit and Chromium controllers retained.

Earlier interactive verification completed a subscribed song using Firefox,
not native playback. Updated-source checks are recorded in docs/validation.md.
No release tag is created for the requested native player.
