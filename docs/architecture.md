# Architecture

The event-driven Ratatui client uses bounded input parsing, a private Unix socket
and a current-thread Tokio service. Queue operations are canonical in the service,
persist atomically, and publish status updates to subscribed clients. Queue
revisions invalidate visible pages after external edits without a feedback loop.
Invalid offsets reset after shrink. MPRIS exposes honest capabilities.

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

Native playback runs in an owned child of the same executable, using inherited
private command/event pipes. Only public song IDs and bounded controls cross
those pipes. The worker reads the private user token itself and retrieves the
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
Source codec/rate/channels are decoder fields; target bitrate is explicitly
reported, and source bit depth/lossless support remain unknown/unavailable.

Public catalog metadata is separate from the playback capability. A bounded
Apple Search/Lookup client runs in IPC client tasks, so a slow metadata request
does not block canonical queue/status handling. It sends no account credentials
and ignores preview audio URLs. Numeric page cursors cannot redirect requests.
Public records are bounded, sanitized, cached and attributed with store links.
Private account data has a distinct unavailable-authorization result.

Keyboard menus keep separate navigation, content and player-control focus. Song
actions freeze the selected record; fresh queries invalidate old rows/cursors,
and a bounded back stack restores public browsing. Returning to Queue refetches
rows; queue-page revisions and checked identity/revision edits prevent stale
indices from playing/removing/moving a different track after an external change.
