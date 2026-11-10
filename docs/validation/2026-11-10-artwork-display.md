# Artwork display — 10 November 2026

The test terminal inherited `NO_COLOR=1` from its launcher. Crossterm therefore
suppressed the RGB foreground/background commands used by half-block artwork.
The image downloaded and decoded successfully, but only its block glyphs reached
Kitty, producing monochrome noise. The earlier artwork test checked a Ratatui
buffer rather than the actual terminal byte stream and did not catch this.

The TUI now calls `crossterm::style::force_color_output(true)` before terminal
initialization. This does not change CLI output or the configured theme. It
preserves real cover pixels, including gray values in a monochrome photograph.

Regression: `scripts/verify-artwork-display.py` runs the real TUI in a 140×69 PTY
with `NO_COLOR=1`, isolated XDG paths, synthetic IPC metadata and one public Apple
image. Pyte interprets the emitted terminal stream and verifies that the cover
retains RGB values. Before the fix it failed with “NO_COLOR suppressed RGB
output”; after the fix it passed and exited cleanly. The test keeps draining
large color frames while waiting for shutdown so PTY backpressure cannot produce
a false timeout. It uses no credentials, browser, CDM or audio.

Validation: fmt check, clippy with `-D warnings`, all 145 tests (0 failures,
3 ignored), and release build passed. The separate image-display regression,
isolated albums/queue PTY smoke and idle measurement passed. Network artwork
validation is separate from the unit suite.

The actual Rustune test window was inspected before and after. It now displays
the correct cover. Only its TUI was refreshed: service PID, audio-worker PID and
queue revision were unchanged across that refresh. Playback remained in the
user's paused state. No real-song completion or new authentication test is
claimed. Window captures were kept outside the repository; no tokens or private
session contents were added to this report.

Fresh isolated idle measurement (TUI + service, not active audio):

```json
{
  "scenario": "isolated idle TUI + service, balanced, no playback engine",
  "native_rss_kib": 17616,
  "native_pss_kib": 6266,
  "native_cpu_percent_one_core": 0.0,
  "sample_seconds": 5.000381996000215,
  "first_tui_frame_ms": 10.33649600140052,
  "cold_cache": false,
  "browser_processes": 0,
  "browser_rss_kib": 0,
  "browser_pss_kib": 0,
  "help": true,
  "resizes": 4,
  "quit_exit": 0,
  "audio_tested": false
}
```

0% sampled CPU means no observed tick increase during the sample, not a guarantee of zero work. No owned browser or audio helper was running.
