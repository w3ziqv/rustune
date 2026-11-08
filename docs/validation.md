# Validation — 2026-11-08

## Current implementation

Native Apple Music playback without a browser is **not implemented or verified**.
The default disables playback with an explicit explanation. It does not start a
browser or integrate Music Assistant. The user's supplied token remains private
outside the repository; it is not an audio engine. No release tag is created.

Local validation on Arch Linux x86_64, stable Rust 1.99.1:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test`: 36 tests passed; three browser smoke tests and one network
  artwork smoke test are opt-in and excluded from the normal suite.
- `cargo build --release`: passed; `cargo build --profile compact`: passed.
- `cargo audit`: no reported vulnerabilities or advisory warnings in this lockfile.
- `rustup check`: stable 1.99.1 up to date; `cargo update --dry-run`: zero updates
  within current stable-compatible dependency requirements.
- `node tests/extension.mjs` and `node tests/musickit.mjs`: passed. These use
  controlled SDK/extension fixtures and establish no live authentication/audio.
- `cargo test --test artwork -- --ignored`: passed separately; downloads an actual
  public Apple cover, decodes/encodes and renders it. No browser, credentials or
  audio are involved.
- `python3 scripts/measure-idle.py target/release/rustune`: real isolated Linux
  PTY rendered, opened Help, resized four times and quit with exit 0. Repeated
  successfully with the compact build. No playback controls were sent.

Coverage includes queue identity and bounds, atomic private state, config
validation, framed IPC/native messaging, startup cleanup, failed-playback queue
rollback, cache invalidation, real isolated D-Bus integration, layout resizing,
monochrome colors, lazy/stale artwork, unknown source quality, and disabled
engine commands/MPRIS capabilities. Sixel is rejected until terminal pixel
geometry is verified; it is not advertised as supported.

An independent review found Sixel geometry overflow and misleading disabled-mode
capabilities. Both were fixed with failing-then-passing regressions and reviewed
again. Source/history checks found no supplied-token or private-key matches.

## Earlier interactive browser verification

Before the requirement changed to browser-free native playback, a subscribed
Firefox 159.0.1 session on Arch Linux played a complete 113-second song through
the scoped WebExtension/native-messaging/MusicKit controller. Actual snapshots
showed progress through the end, preview false, then MusicKit completed state 10.
This was observed twice. Real TUI Space pause/resume and real catalog/personal
library, albums, artists, playlists and recently-played reads were observed.
Seek-to-start and a volume change were exercised. No passwords or page tokens
were extracted.

This evidence applies to that earlier browser-backed build, not native decoding
or the updated binary. A broader live-control script stopped at a queue-index
assumption while the user was editing the queue. It did **not** establish all
next/previous/mute/MPRIS/sleep controls end-to-end. Remaining coverage for those
features is automated protocol/state/D-Bus validation. No further automatic sound
tests are run while the user is listening.

The three real Chromium smoke tests were run separately in the earlier phase:
private pipe transport, official MusicKit host/token rejection, and official web
player/controller/catalog availability. They do not establish a subscribed
Chromium playback path. No long-duration listening/memory-growth guarantee,
lossless path, native DRM integration, live logout, cross-platform or permanent
Mozilla add-on signing validation is claimed.
