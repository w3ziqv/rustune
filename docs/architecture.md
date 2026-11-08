# Architecture

Rustune targets Linux exclusively. A Ratatui/Crossterm TUI uses a private Unix
socket to control a current-thread Tokio service. The service owns the queue,
configuration, sleep timer and bounded page cache. Native Apple Music playback
is the target but is not implemented; the default disables playback without a
browser fallback. Explicit legacy backends can own a lazy Chromium process or
control the existing Firefox session. zbus publishes
MPRIS on the session bus. UI state is independent of browser automation.

```
TUI / CLI / SSH ── Unix socket ── service ── optional private pipes ── browser
                                      │                             │
                                 MPRIS / D-Bus                MusicKit / EME
                                                                    │
                                                            system audio output
```

`backend = "auto"` selects the minimal local MusicKit host when a private
MusicKit developer JWT is configured, otherwise the official Apple web player.
`apple_web` explicitly uses `https://music.apple.com`; `music_kit` requires the
user's developer token. Both use MusicKit's controller inside the page. Rustune
never reads developer tokens, Music User Tokens, cookies or protected stream URLs
from Apple's page. The web-player controller is an experimental integration,
not a stability guarantee from Apple.

The local MusicKit document loads Apple's official CDN library; only static HTML
is served on loopback, with Host validation and frame blocking. Its developer
token is supplied over inherited pipes, never HTTP or browser command arguments.
Chromium owns the dedicated persistent session profile. Apple ID and 2FA happen
inside Chromium. All normal player controls remain in the terminal.

Queue indexes refer to Rust's persisted bounded `Vec<Track>`. Songs are set in
the browser individually; automatic advance uses real playback state, so gapless
and crossfade are not promised. IDs and metadata are validated. Catalog data is
paginated; only one page is displayed and a bounded number of pages is cached.
The cache is FIFO with replacement of duplicate keys, not an unbounded library.

The service accepts at most eight client connections and 32 pending commands;
the TUI has eight pending jobs and one latest status update. Status streams push
changes; there is no idle TUI polling. Active browser snapshots run at the chosen
resource profile's interval. The service sleeps indefinitely without a browser
or timer. A browser request is serialized and has a deadline; navigation remains
responsive in the TUI while it waits.

XDG directories are private (0700), files and socket are 0600, and peer UID is
checked. An advisory lock prevents duplicate services and permits stale-socket
recovery. Atomic state writes use create-new temporary files and rename.

`rustune daemon` runs in the foreground for systemd. An automatically started
service is placed in a separate process group. A TUI that starts its own service
stops it on quit unless `daemon = true`; a TUI attaching to an existing service
leaves playback running. No network control API is exposed. Use SSH to run the
CLI on the host owning the socket.

Modules: `model`, `queue`, `config`, `cache`, `ipc`, `service`, `backend`,
`keys`, `ui`, `mpris`, `metrics`. Application Rust forbids unsafe code. The small
`command-fds` dependency handles descriptor mapping for Chromium's fd 3 and 4.

Local token login is independent of playback service IPC. CLI entry runs before
starting a service; TUI entry uses a separate local storage worker. SecretToken
has redacted Debug, no serialization and zeroized owned buffers. A bounded input
parser reads cloned stdin through private descriptors: 512-byte read blocks,
16 KiB bracketed paste, 64-byte escape sequences, a one-event channel and a private
UnixStream pair for immediate reader shutdown. Oversized paste is drained through
its closing marker and rejected, never accepted as a truncated token. No public
listener or unsafe application code is involved. CLI raw mode precedes the prompt;
guards restore the terminal on errors and cancellation. These are application
buffer protections, not a claim about kernel, terminal or filesystem cache wiping.

The LRCLIB worker is independent of authorization and browser automation. It sends
only track metadata, keeps one latest request, enforces HTTP/cache/text limits and
rejects stale results. See lyrics.md. The dedicated player module owns geometry,
metadata, wrapped synchronized text, progress and mouse/keyboard controls.

## Artwork and quality

The terminal client owns a lazy image worker, separate from the playback service.
A watch channel retains one latest request, with one active fetch/decode/encode
and a one-result channel. Screen/track/profile/theme changes invalidate old
results. A bounded FIFO cache shares encoded protocols via Arc. Halfblocks is the
portable default; Kitty and iTerm2 are explicit config options. No terminal
query blocks startup and no image decode/encoding occurs during render.

Apple image hosts receive no credentials. Responses are HTTPS-only, with no
redirects, capped at 2 MiB and 5 seconds. Decode dimensions are strictly capped at
2048 × 2048, with a best-effort 32 MiB decoder allocation limit, then resized to
256/512 pixels by resource profile. Minimal mode disables fetching. Background
jobs are cancelled on exit; an already running bounded blocking decode may finish.

MusicKit's public bitrate target is labeled as a target. It never establishes
codec, sample rate, bit depth, channel count, lossless or measured source bitrate.
