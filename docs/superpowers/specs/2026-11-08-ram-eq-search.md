# RAM, equalizer and search

User-authorized work: optimize RAM while preserving native playback and covers;
implement a real keyboard-controlled equalizer; find punctuation-heavy songs and
combined title/artist queries including `A&W lana del ray`.

Remain local: no pushes, PRs, tags or releases. Linux/Rust2024/stable1.99.1,
no unsafe application Rust, no browser processes/Music Assistant, no DRM bypass,
credentials outside repository/logs/argv. Preserve actual user's token, queue,
preferences and active playback until installation can be done without disruption.

Deliverables: independently built lightweight audio executable and measured RAM
comparison including all owned processes; ten stereo EQ bands with preamp,
bounded gains, bypass, presets and persistent settings; bounded, ranked search
that preserves special characters, tolerates artist spelling errors and keeps
pagination stable. Show source fields honestly; EQ does not change source quality.

Validation: format, Clippy, whole workspace tests, release build; synthetic DSP
frequency-response/bypass/stereo tests, actual online search and keyboard tests,
real native playback and before/after RSS/PSS/CPU measurements. No completion
claim for audio based solely on mocks or search results.
