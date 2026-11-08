# Validation — 2026-11-08

## Current implementation

Native Apple Music playback without a browser is **not implemented or verified**.
The default disables playback with an explicit explanation. It does not start a
browser or integrate Music Assistant. The user's supplied token remains private
outside the repository; it is not an audio engine. No release tag is created.

Local validation on Arch Linux x86_64, stable Rust 1.99.1:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test`: 56 tests passed; six opt-in browser/network/visual tests are
  excluded from the normal suite.
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

The token/lyrics/layout update adds real CLI and TUI PTY tests using artificial
tokens in isolated XDG directories: hidden input, terminal echo disabled before
the prompt, private storage, cancellation and no token in screen buffers/IPC.
An independent review identified an echo race, upstream unbounded paste buffering
and obsolete login hints. Fixes use Rustune's bounded raw-input parser/private
wakeup pipe, enable raw mode before the prompt and distinguish local token storage
from browser connection. A one-million-byte/unterminated paste test verifies the
16 KiB parser bound and rejects partial credentials. Ctrl-C cancels incomplete
sequences. Source has no unsafe application code.

Lyrics tests cover multiple timestamps, offsets, safe text, plain/instrumental
states, real HTTP contracts, matching, response limits and Retry-After. A separate
real LRCLIB lookup returned 37 synchronized lines for the user-referenced song;
its text was withheld from output. Wrapping/scrolling and stale lyrics/cover
rendering have regression coverage. A 142×39 actual Ratatui buffer was visually
inspected with public Apple artwork, synthetic lyrics and an explicit no-audio
preview label. A missing artwork upscale found by visual inspection was fixed
with a RED→GREEN encoded-cell-size regression.

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

A further regression verifies that a selected saved queue song opens the Lyrics
view without a backend or any playback command. Preview metadata/artwork is
separate from actual playback status; another song's clock is never used to sync
it. The GitHub CI matrix for the main token/lyrics/layout update passed on stable
and pinned 1.99.1.

## Installed binary functional check

The final installed release binary and `target/release/rustune` have identical
SHA-256 hashes, recorded in the [functional report](validation/2026-11-08-functional.json).
The optional `scripts/verify-functional.py` exercised the actual PTY input/output
through a pyte screen emulator with isolated XDG paths, a private D-Bus session
and an artificial local token. All 66 checks passed. It downloaded real public
LRCLIB lyrics and Apple artwork, but did not authenticate an Apple account or
play audio. Expected unsupported-control errors count among these checks;
passing this scenario does **not** establish a working native music player.

It checked black/white terminal colors, six mouse transport controls, queue
keyboard reordering, silent selected-song lyrics/cover preview, manual scrolling,
profiles, live theme changes, hidden token paste/cancellation/logout, four real
resizes, external queue additions/reorders/shrink after PgDn, mouse quit, service
lifetime, persistence across restart, and persistent clear. Native playback and
MPRIS capabilities correctly remained unavailable, with no browser fallback.

Real interaction found two defects: the first manual scroll did not move visible
lyrics, and external edits left the open queue stale. Failing-then-passing
regressions accompany both fixes. Independent review found two related edge
cases, now also covered: the first upward scroll after viewport growth and an
invalid page offset after queue shrink. Re-review found no remaining substantive
issue in these changes. Format, Clippy with warnings denied, all 56 tests and
the release build passed after the final source changes.

To repeat the optional online check, install pyte in a test-only virtualenv and
run its Python interpreter with `scripts/verify-functional.py /absolute/path/to/rustune`.
It requires `dbus-daemon`, `busctl` and access to LRCLIB/Apple artwork. It prints
only check names/outcomes; no lyric text, credentials or raw terminal capture
are retained. The ordinary unit suite does not require this network check.

An updated installed-binary idle measurement is recorded in the
[idle report](validation/2026-11-08-idle.json). It is a no-audio sample, not an
active-playback resource measurement. The legacy real browser smoke tests were
not rerun for this native-mode check. Apple acceptance of the user's token,
native decoding, live source quality and complete-song playback remain unverified
and unavailable in the default backend.

The earlier login-only PTY helper was also corrected to use `daemon = false`
and stop its private service in failure cleanup. Repeating it under
`dbus-run-session` passed hidden CLI/TUI input, private storage, cancellation and
the new no-leftover-service assertion. Eight abandoned login-test services from
already-deleted temporary directories were terminated after checking their UID,
command and isolated XDG path. The user's existing service was preserved; it
still runs an older executable until restarted.
