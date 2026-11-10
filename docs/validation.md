# Validation

## RAM, native equalizer and account search — 2026-11-10

Current workspace checks passed on stable Rust1.99.1: format, Clippy with warnings
denied,105 Rust tests (3 network/visual tests remain opt-in), standard workspace
release and separately built slim audio release. A fresh independent review found four navigation/pagination issues; all received
failing-then-passing regressions and the whole suite passed afterward. Final
[67 real native control checks](validation/2026-11-10-eq-search-controls.json) also
verify EQ-enabled paused seek/resume, Alt-arrows within EQ and pinned-page return.
The final helper matches the complete-song helper exactly.

The genuine subscribed native [full-song test](validation/2026-11-10-eq-search-full-song.json)
passed58 checks:359.746757s output versus359.72787s manifest,15493 protected
packets/decoded frames, no preview or browser, real artwork/source fields and
Vocal EQ applied to a fresh full-song worker. It also exercised actual A&W playback,
Bass preset, rapid arrows, PCM attenuation, bypass and persistent config.

Real Apple Music catalog searches `A&W`, `A&W lana del rey` and `A&W lana del ray`
found the original1655349125 among the first five results. The user token was
accepted for account region and private song-library listing/search (25 rows in
the sampled page). Private song names are not recorded. A successful no-match
response returns an empty page; a local HTTP fixture verifies that catalog calls
omit the user token/cookies while account calls carry the private token.

Separate artificial-token PTY runs passed [52 keyboard](validation/2026-11-10-eq-search-keyboard.json) and [65 general checks](validation/2026-11-10-eq-search-functional.json) for
navigation, public searches, lyrics/artwork, themes, queue and masked login. They
do not prove account acceptance or audio; genuine playback evidence is separate.

Paired30s real EQ-off playback samples including all three processes showed
PSS141.23→110.46 MiB (−21.79%; prior run −20.04%). See [measurements](benchmarking.md). No pushes,
PRs, tags, releases, supplied credentials or proprietary CDM binaries were published.

The [installed pair](validation/2026-11-10-eq-search-installed.json) matches the
verified release hashes. The real user token (inode/mtime/mode/content), configuration
bytes and all39 queue records were preserved. Existing audio was inactive at
installation, and no playback was started. The new real service separately
accepted account-region search for the typo query and private-library listing.
Reopen an already-running older TUI to load the new interface.

## Native subscribed playback verified — earlier revision

The real native CDM/AAC/ALSA backend played the complete subscribed catalog song
“Simple Man”: 359.746757 seconds against a 359.72787-second full manifest,
15,493 encrypted packets decrypted and AAC frames decoded, and 31,517,516
nonzero output samples. This was full-stream audio, not a preview, with no owned
browser or Music Assistant runtime. The release service/TUI test passed 30 checks
including keyboard Play, pause/resume, volume/mute, seeking both directions,
Next/Previous, post-logout controls, real artwork and displayed source format.
It also fetched public application authorization anonymously from an empty cache.

[Full-song report](validation/2026-11-08-native-release-live.json) identifies that
release binary by SHA-256. Two final edge fixes subsequently corrected MPRIS
capabilities after queue clear/EOF and timer scheduling at capacity. The final
installed build separately passed 34 real native [control checks](validation/2026-11-08-native-controls.json),
including private-session D-Bus properties, post-logout transport, near-end
pause/resume and verified EOF. That control run seeks and explicitly does not
claim a second complete-song test. The [installation report](validation/2026-11-08-native-installed.json)
confirms the installed binary matches the final release, and preserves the real
user token (including inode/mtime), configuration and all 11 queue records.

During complete-song playback, all three owned processes (service, TUI, audio
worker) peaked at summed RSS 223,584 KiB and PSS 142,343 KiB. Sampled CPU was
2.816% of one core over the recorded interval. The CDM is included in these
figures; browser process count is zero. These are active measurements, distinct
from earlier idle numbers. Source AAC, 44.1 kHz and stereo come from the native
decoder; 256 kb/s is labeled as a reported target. Source bit depth/lossless
remain unknown/unavailable. Personal library and playlists are not implemented.

Final format, Clippy with warnings denied, 84 Rust tests, release build and
RustSec audit passed. Three public-network/visual tests remain intentionally
opt-in. The source-built CDM host has a separate passing C++ timer test, including
monotonic deadlines and reentrant scheduling at the 128-timer bound. An
independent reviewer found the PCM, EOF, logout, drain, queue identity and timer
issues; fixes have failing-then-passing regression coverage. Credential matching
found no supplied user/public application token in repository files.

No push, PR, tag or release was made for this work. Credentials and the proprietary
CDM binary remain outside the repository.

The installed final binary also passed all 52 [keyboard-only checks](validation/2026-11-08-native-keyboard.json)
and 65 [general functional checks](validation/2026-11-08-native-functional.json).
These two use artificial tokens and establish catalog/UI/lyrics/artwork behavior;
they are separate from the genuine subscribed playback evidence above.

## Catalog and keyboard revision

All 71 Rust tests, format, Clippy with warnings denied and release build passed.
The installed binary passed 52 keyboard-only online PTY checks (actual Apple/iTunes
song/album/artist search and lookup before/after artificial token storage, queue,
lyrics, actionable settings and quit) and 70 general PTY checks. The checks use
private XDG/D-Bus and artificial tokens; no Apple account authentication or audio
is established. Reports: [keyboard](validation/2026-11-08-catalog-keyboard.json),
[general](validation/2026-11-08-catalog-functional.json),
[idle](validation/2026-11-08-catalog-idle.json).

New queue edits verify revision and identity; delayed pages require the current
screen/offset/revision. Independent review findings on focus, stale queries and
queue-page provenance were fixed with failing-then-passing regressions. The user's
actual queue and token file were preserved during installation/service restart.

The records below describe earlier revisions before native playback was added.

## Earlier token-storage-only revision

Browser controllers, Firefox extension/native-host assets, setup commands and
browser authorization have been removed. Recognized old browser configuration
migrates atomically while preferences, the queue and the token file remain intact.
Token entry in CLI/TUI shows at most 32 dots plus an accurate count; Backspace,
Ctrl-U, paste, save and cancel have real PTY coverage. Local storage remains
unverified by Apple; **native Apple Music playback is not implemented**.

Final local checks on stable Rust 1.99.1 passed: formatting, Clippy with warnings
denied, all 54 tests and release build. Three public-network/visual tests remain
opt-in; obsolete browser tests were removed with the feature. The installed binary
matches the release SHA-256 in the [native-only functional report](validation/2026-11-08-native-only-functional.json).
All 70 actual TUI checks passed, including real public LRCLIB/Apple artwork,
masked typing/paste/editing, cancellation, queue changes and service restart.
This includes expected unsupported playback errors and verifies no Apple account
acceptance, subscription entitlement or actual audio.

`verify-login-display.py` first reproduced CLI wrapping at 80 columns. The fix
uses static instructions and one width-bounded redraw row. The enhanced regression
then reproduced an incorrect initial dot count; the final version passes visible
1→2→1→0 dots and long input at 80, 40 and 20 columns. The CLI/TUI login helper also
passed hidden input, private save, editing, cancel and no leftover test service.
These scripts use artificial credentials and retain no plaintext secret captures.
Independent review found no remaining substantive issue after these fixes.

The installed user's idle old service was replaced with the native-only service.
The verified Rustune native-host registration, generated bridge assets and unused
bridge lock were removed. The user's queue hash and token inode/mtime/size/mode
were unchanged. Firefox profiles were not touched. The updated isolated idle
[sample](validation/2026-11-08-native-only-idle.json) recorded RSS 17,272 KiB,
PSS 5,878 KiB, zero sampled CPU ticks over 5 seconds and zero owned browsers.
These are idle measurements, not active playback measurements.

Everything below records earlier revisions and historical browser work; it does
not describe current functionality or provide supported browser setup steps.


## Earlier revision records (before browser removal)

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
