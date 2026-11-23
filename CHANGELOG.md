# Changelog

## v0.5.0 (local candidate)

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
