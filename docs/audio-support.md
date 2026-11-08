# Audio capabilities

| Feature | Current status |
|---|---|
| Native Apple Music playback | Not implemented |
| Browser playback, extension, native messaging | Removed |
| Transport, catalog, library, nonzero sleep timer | Explicit unavailable-capability errors |
| Source codec, bitrate, sample rate, depth, channels, buffer | Unknown |
| Lossless / Hi-Res, quality selection, EQ, DSP, visualizer | Unavailable |
| Lyrics | Free LRCLIB LRC/plain/instrumental lookup; wrapped, scrollable silent queue preview |
| Artwork | Real lazy Apple covers; bounded cache and decode, halfblocks/Kitty/iTerm2 |
| Local Music User Token | Masked entry, private storage, cancellation/logout; Apple acceptance unverified |

Saving a token does not establish a playback engine, subscription entitlement or
stream quality. No preview or fabricated audio signal is presented as native
playback. A complete subscribed song must play through a real authorized engine
before release tagging. PipeWire's output format does not establish the source
format; there is no PCM path for Rustune DSP in this build.
