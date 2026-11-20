# Architecture

The event-driven Ratatui client uses bounded input parsing, a private Unix socket
and a current-thread Tokio service. Queue operations are canonical in the service,
persist atomically, and publish status updates to subscribed clients. Queue
revisions invalidate visible pages after external edits without a feedback loop.
Invalid offsets reset after shrink.

The socket validates peer UID and the 64 KiB request length before reading the
body. Up to 16 new connections can read for five seconds. Parsed requests use
separate pools for four catalog lookups, eight subscriptions and 16 ordinary
commands. Each response frame has a 4 MiB limit and a two-second write deadline.

Token entry stays in the CLI/TUI process and uses zeroizing secret buffers.
Dots/count are derived locally; actual token content is never rendered or sent
through service IPC. A separate worker writes the private 0600 file; the result
only confirms storage. Recognized retired browser settings are migrated away.
There are no browser controllers, extension assets, native host or developer-token
transport paths.

Lyrics and artwork workers fetch only public metadata endpoints with identified,
bounded HTTPS requests, timeouts, limited caches and stale-result checks. Artwork
has strict host/decode bounds. Lyrics distinguish missing, instrumental, plain,
synced, throttled and error states. Silent queue previews do not borrow the clock
of an unrelated playing track. Rendering constructs visible rows only.

Native playback runs in the owned sibling `rustune-audio` executable, using inherited
private command/event pipes. Only public song IDs and bounded controls cross
those pipes. Both processes negotiate protocol 2 with a framed Hello before
Prepare. Each applied seek carries its request ID and playback position. The
service accepts acknowledgements from the current helper generation in order,
including the final applied request in a coalesced series. MPRIS emits Seeked
from these events. CanControl describes the writable desktop interface. CanPlay,
CanPause, CanSeek and the navigation properties report the service’s current
capabilities. The service probes the helper again when its executable changes.
Position and CanControl changes produce no PropertiesChanged notification.
See the [MPRIS Player specification](https://specifications.freedesktop.org/mpris/latest/Player_Interface.html).

The worker reads the private user token itself and retrieves the
public application authorization anonymously from allowlisted Apple HTTPS
origins. Authenticated playback/license requests never follow redirects and
credentials are never sent to artwork, lyrics or media CDN hosts.

A source-built pinned third-party CDM host drives an installed Widevine module
through its published ABI; no content keys are extracted. One thread owns one
CDM per worker process. Monotonic deadlines drive CDM callbacks; renewed license
requirements stop playback with an explicit error. FFmpeg demuxes encrypted AAC
packets and exposes bounded CENC metadata; the CDM decrypts those samples, then
native AAC decoding and bounded PCM conversion feed ALSA/PipeWire. Decrypted
audio remains in memory. The converter drains its tail and resets after seeking.

ALSA's submitted/queued frame counts provide the playback clock. EOF alone does
not establish completion: decoder errors abort, and the submitted end position
must match the full manifest duration. After the real output ring empties, the
worker exits and is reaped before queue advancement. Pause/seek remain active
during final draining. Active transport stays controllable after local logout.
Source codec, rate and channel count come from the decoder. Source bitrate
and bit depth remain unknown.

Public catalog metadata is separate from the playback capability. A bounded
Apple Music catalog client (with iTunes fallback) runs in IPC client tasks, so a slow metadata request
does not block canonical queue/status handling. Public catalog requests send no user credentials
and ignore preview audio URLs. Allowlisted provider/region/offset cursors cannot redirect requests. Cursors
pin both provider and storefront across page navigation; a failure on a pinned
page remains an error instead of switching result sets. Explicit previous
cursors preserve each backend’s actual page size.
Public records are bounded, sanitized, cached and attributed with store links.
Account region is verified through `me/storefront` and cached briefly. Library
listing/search use fixed private endpoints with the user token, invalidated on
credential-file replacement. Private errors never fall back to public results.

Keyboard menus keep separate navigation, content and player-control focus. Song
actions freeze the selected record; fresh queries invalidate old rows/cursors,
and a bounded back stack restores public browsing. Returning to Queue refetches
rows; queue-page revisions and checked identity/revision edits prevent stale
indices from playing/removing/moving a different track after an external change.

The UI/service do not link FFmpeg, ALSA or the CDM host. The optional slim audio
feature statically links only AAC/MOV and HTTPS support from a checksum-pinned
private FFmpeg build. All three owned processes are included in measurements.

Ten independent stereo peaking filters process decoded PCM before software
volume/output. Settings are bounded and validated in configuration and IPC.
A measured combined frequency response provides boost headroom; 20 ms transitions
smooth live changes. Bypass and flat settings preserve PCM exactly. The worker
reports applied settings and sample/RMS counters separately from requested settings.
