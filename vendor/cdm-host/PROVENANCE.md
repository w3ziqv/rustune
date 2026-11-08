Source: https://github.com/Kopuz-org/cdm-host
Revision: 0e0d76b6b276b8c750bb0cb277b10071de5bc661
License: MIT (Chromium ABI headers retain their BSD notices).

Rustune builds the C++ shim from source; no prebuilt archives, proprietary CDM
binaries, device credentials, challenges or license responses are distributed.
The optional downloader and example CLI are excluded from use. A single isolated
worker owns the process-global CDM. No application code uses unsafe Rust or
extracts content keys. Host and license security decisions remain with the CDM.

Local changes: compile exclusively from source; omit installer and example;
honor monotonic CDM timer deadlines and pump scheduled work during decryption.
Timer behavior is separately tested by compiling/running timers_test.cc.
