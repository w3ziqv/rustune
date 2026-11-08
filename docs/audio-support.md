# Audio capabilities

| Feature | Current status |
|---|---|
| Native Apple Music playback | Not implemented |
| Browser playback, extension, native messaging | Removed |
| Public Apple/iTunes catalog metadata | Search songs/albums/artists, album tracks and artist albums; no token required |
| Transport, personal library/playlists, nonzero sleep timer | Distinct unavailable playback or account-authorization errors |
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
