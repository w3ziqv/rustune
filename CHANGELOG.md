# Changelog

## Unreleased — experimental source

- Public Apple/iTunes catalog search and lookup now work independently of token storage and the missing audio engine.
- Arrow/Enter menu navigation, keyboard settings, song actions, focused transport buttons and browsing back/pages.
- Search resets stale focus/results; checked queue edits reject stale track identities/revisions.
- Personal-library authorization is distinguished from public catalog search and playback availability.

- Linux Ratatui terminal client and private Unix-socket service.
- Persistent queue, bounded pagination/cache, MPRIS and sleep timer.
- Latest stable Rust 1.99.1, zbus 6, TOML 1.2 and current compatible dependencies.
- Hidden token login in CLI/TUI, private local storage and explicit unverified outcomes.
- Bounded terminal paste/input with private cancellation pipes and credential buffers wiped on drop.
- Free LRCLIB synced/plain lyrics, safe matching, source attribution and wrapped scrolling.
- Large-cover player layout based on the selected amcli composition.
- Pure black background and white text/borders; optional Tokyo Night and other themes.
- Bounded lazy album covers and explicit terminal image protocols.
- Reported bitrate targets distinguished from unknown source quality.
- Native playback is unavailable; default backend disabled with no browser fallback.
- Removed browser playback/login, extension/native-host assets and setup commands.
- CLI/TUI token input now shows bounded dots/count with editing and explicit local-save feedback.
- Retired browser configuration migrates while preferences, queue and tokens are preserved.
- Fixed immediate manual lyrics scrolling, including after terminal resizing.
- Open queue follows external additions/reordering and returns to a valid page after shrink.
- Reproducible isolated PTY functional smoke with public covers/lyrics and explicit no-audio scope.

Earlier interactive verification completed a subscribed song using Firefox,
not native playback. Updated-source checks are recorded in docs/validation.md.
No release tag is created for the requested native player.
