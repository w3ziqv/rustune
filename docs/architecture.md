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

The native playback engine is absent. Playback/catalog requests return a clear
capability error without changing the queue. No DRM bypass, fabricated stream or
preview playback is used. Source quality remains unknown. Future native playback
requires a genuine authorized engine and live complete-song verification before
a release tag.
