# Audio capabilities

| Feature | Current status |
|---|---|
| Native Apple Music playback | CDM → AAC → PCM → ALSA/PipeWire; complete subscribed song verified |
| Browser playback, extension, native messaging | Removed; no browser runtime |
| Catalog metadata | Apple Music songs/albums/artists and lookups; explicit iTunes fallback |
| Private song library | Accepted Media User Token enables listing/search and account region |
| Personal playlists/recent history | Not implemented |
| Transport | Play/pause, Next/Previous, seek, software volume/mute, sleep timer and MPRIS |
| Source quality | Decoder-reported codec, sample rate and channels;256kb/s is a reported target |
| Bit depth/lossless/Atmos | Unknown or unsupported; output format does not prove source format |
| Equalizer | Ten independent stereo bands, preamp, presets, bypass, headroom and live smoothing |
| Visualizer/crossfade/gapless guarantees | Unavailable |
| Lyrics | Free LRCLIB synced/plain/instrumental lookup; wrapping and keyboard scrolling |
| Artwork | Real bounded lazy Apple covers, halfblocks/Kitty/iTerm2 |
| Local token | Bounded masked entry, private storage, cancellation/logout |

A token identifies a subscriber; it does not supply a DRM playback engine.
The separate native helper, compatible local CDM and Apple license/subscription
checks are also required. Saving a token confirms local storage only. No previews
or generated audio stand in for full playback. Uploaded songs without a catalog
ID cannot be played by the catalog-song backend. Source quality fields stay
unknown unless the backend can report them reliably.

See [validation](validation.md) for actual subscribed complete-song and EQ tests,
separate synthetic/unit/PTY coverage, exact binary hashes and memory measurements.
Long-duration memory stability and unsupported formats remain unverified.
