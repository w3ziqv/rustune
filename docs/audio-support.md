# Audio capabilities

| Feature | Status |
|---|---|
| Native Linux playback without browser | Not implemented; default backend disabled |
| MusicKit full-song playback commands (optional legacy backend) | Implemented; requires live subscription/DRM validation |
| Play, pause, next/previous, seek, volume, mute | Controller implementation; no fake audio or previews |
| Auto quality | Enabled; backend chooses the stream |
| Data Saver, Standard, High Quality presets | Disabled; no verified setter contract |
| Lossless / Hi-Res Lossless | Disabled; no verified ALAC path |
| Source codec, bitrate, sample rate, bit depth, channels | Unknown |
| MusicKit bitrate target | 64/256 kb/s when publicly reported; not measured source bitrate |
| Buffer occupancy | Unknown |
| Output device | Explicit legacy browser backend uses its system audio device |
| EQ, presets, preamp, bass, treble, normalization | Disabled; no verified protected PCM access |
| Crossfade / gapless | Not implemented or guaranteed |
| Lyrics | Free LRCLIB lookup, synchronized LRC/plain text/instrumental states, wrapped scrolling |
| Visualizer | Unavailable; no fabricated audio data |
| Artwork | Real lazy covers; bounded cache; halfblocks or explicit Kitty/iTerm2 protocol |

PipeWire/WirePlumber can route and process the browser's output. Rustune does not
claim that this changes the source encoding. The Audio Settings and Equalizer
screens explain these limits, rather than presenting ineffective sliders.

No browser is started by the default disabled backend. In the optional dedicated
Chromium backend, headless mode is the default. EME/audio support varies.
If it fails, set `headless = false`: the browser runs internally and is minimized
via CDP after startup/authorization. This mode may briefly show a window and is
experimental. Never enable `--no-sandbox`, ignore certificate errors or disable
web security to make playback work.
