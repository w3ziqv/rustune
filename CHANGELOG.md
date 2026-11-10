# Changelog

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
