# Lyrics and player layout

Rustune uses the free, anonymous [LRCLIB API](https://lrclib.net/docs). No API key,
Apple token, cookie or account identifier is sent to the lyrics provider. Only
the visible track's title, artist, album and duration are sent. Set `lyrics = false`
to disable requests. Coverage is community-maintained and cannot include every
song or version. LRCLIB is attributed in the player.

Lookup tries the exact track signature, then a bounded search. Search results
must match normalized title and artist and, when known, duration within three
seconds. Matching album is preferred. An unrelated result is never presented as
the requested song. Instrumental, missing, network error and throttled states
are distinct. R retries; the provider's Retry-After still takes priority.

The identified HTTP client uses TLS, no redirects, eight-second request deadlines,
512 KiB response limits, sequential requests with at least 250 ms spacing, and
positive jitter on Retry-After (including HTTP-date values). Each lyric text is
capped at 128 KiB, 4096 source lines and 1024 characters per line, with a total
expanded-text limit. The watch channel keeps only the latest request, one lookup
runs, one result waits and the profile page-cache limit bounds cached records.
Results for old track metadata are rejected. No raw network errors or lyrics are
written to logs.

Timed LRC supports multiple timestamps, fractional seconds and global offsets.
The active verse follows the backend's reported position, at that backend's
snapshot frequency; no unverified millisecond playback precision is claimed.
Plain text is scrollable without timing. j/k scroll wrapped rows, R refreshes
and returns to automatic following, and l opens the dedicated Lyrics view.
Tokens and copyrighted song text are not included in fixtures or public logs;
parser/UI fixtures use independently written sample text.

The selected layout reference is [amcli](https://github.com/juntaochi/amcli): big
cover left, metadata and centered lyrics right, progress and seven bordered
controls below. Rustune writes this implementation independently in Ratatui.
amcli itself controls macOS Music using AppleScript; it is not evidence of a
Linux token-only playback engine. The terminal window's operating-system chrome
is outside the application and is not recreated.

The opt-in visual test exports a real Ratatui buffer with public Apple artwork,
synthetic text and an explicitly labeled example position. It verifies layout,
not audio. The separate real LRCLIB smoke uses the user-referenced song and reports
only line counts/sync status; it does not print the lyrics:

```sh
cargo test --test lyrics -- --ignored --nocapture
cargo test export_player_layout_for_visual_review -- --ignored
```
