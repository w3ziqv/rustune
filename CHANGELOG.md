# Changelog

## v0.6.0

- Scroll lists, text and dialogs under the pointer, including across result and
  queue pages. Long views keep their scroll position separate from the selection.
- Queue pages and catalog pages are fetched on a bounded metadata lane, so a slow
  queue page never delays transport control. One page fetch stays pending per
  context and superseded pages never apply.
- Click a song to open its actions. Scroll inside the menu to choose an action,
  or over the exposed song list to close the menu and scroll there. When the
  wheel waits for the next page, the menu stays open until the rows actually
  move, survives a failed fetch and `R` retry, and a late reply never closes a
  freshly opened menu.
- Queue responses for paged scrolling carry per-row upcoming ranks instead of
  the whole upcoming id list, bounding the wire at 10000 entries. `queue list`
  keeps the legacy full response.
- Access clipped panels and menu actions in short terminals. Paged navigation
  retains the previous rows if a load fails and can be retried with `R`.

## v0.5.1

- One-command, checksum-verified installation for x86_64 GNU/Linux with glibc 2.39 or newer.
- Managed updates switch the matching Rustune and audio helper executables together and retain the previous pair.

## v0.5.0

- Apple Music and iTunes allow two concurrent reads each. Identical requests share a read, and disconnecting the last caller cancels the fetch.
- Private page cursors expire when the account token changes.
- Playlists scroll continuously through a 25-track window. The next page loads in advance, with source order, duplicate occurrences and retry preserved.

## v0.4.0

- Confirmed native seeks, occurrence-checked MPRIS SetPosition and navigation that preserves pause and stop.
- Shared local diagnostics in Settings, `doctor` and `doctor --json`. The private helper protocol requires a matching pair of executables.
- Independent socket limits for incoming frames, controls, catalog requests and subscribers, with bounded response writes.

See the [v0.4 validation report](docs/validation/2026-11-19-transport.md) for offline, public network and subscribed playback results.

## v0.3.0 — 2026-11-17

- Start in Search and switch between seven views. Albums and artists are search categories. Escape restores the previous view and selection.
- Preview lyrics independently of playback. Choose pixelated covers or regular images in compatible terminals from Settings.
- Graphite is the default theme. Equalizer and decoder information are available in Settings.
- Reconnect to the service after disconnects and retry failed catalog pages. Preserve the final playback position after the last song.
- Mean playback PSS with pixelated artwork fell from 133.77 to 132.20 MiB in three resize runs. Idle PSS rose from 7.87 to 8.39 MiB.

See the [validation report](docs/validation/2026-11-15-clean-tui.md) for the complete-song and hour-long playback checks.

## v0.1.0 — 2026-11-10

First experimental source release for Linux.

- Native Apple Music playback using a locally supplied Media User Token and a
  compatible local CDM, with AAC decoding and ALSA/PipeWire output.
- Separate lightweight UI/service and audio helper, with a reproducible minimal
  FFmpeg build. Paired active playback measurements reduced total PSS by20–22%.
- Ten-band stereo equalizer, preamp, Flat/Bass/Vocal/Treble presets, persistent
  settings, boost headroom, live smoothing and bypass.
- Apple Music catalog search with account region, combined title/artist ranking
  and private-library search. Provider/region-pinned pagination and explicit fallback.
- Black/white default theme, optional themes, real bounded artwork, free LRCLIB
  lyrics, keyboard navigation, persistent queue, transport controls and MPRIS.
- Bounded masked token entry; no browser playback, extensions or Music Assistant.

Validation:105 Rust tests, format/Clippy/release checks,67 actual native control/search
checks,52 keyboard and65 general PTY checks. A complete359.75-second subscribed
song played with EQ enabled; source/helper hashes and measurements are recorded
in the validation reports. Credentials and the proprietary CDM are not distributed.

Build both executables with `scripts/build-slim-audio.sh`; see
[installation](docs/installation.md). This source release requires a separately
installed compatible CDM and active subscription for playback. Playlist browsing,
noncatalog uploads, lossless/Atmos, visualizer and crossfade are unavailable.
