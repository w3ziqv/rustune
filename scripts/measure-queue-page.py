#!/usr/bin/python3
"""Measure QueuePage response wire sizes on the real daemon (no network, no tokens)."""
import argparse
import json
import os
import shutil
import socket
import struct
import subprocess
import tempfile
import time
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=lambda path: Path(path).resolve())
parser.add_argument("--entries", type=int, default=10_000)
parser.add_argument("--page", type=int, default=241,
                    help="maximum responses per round trip (odd, including the starting page)")
options = parser.parse_args()
if options.entries < 1:
    parser.error("--entries must be positive")
if options.page < 3 or options.page % 2 == 0:
    parser.error("--page must be an odd number of at least 3")
binary = options.binary

with tempfile.TemporaryDirectory(prefix="rustune-queue-page-") as folder:
    test = Path(folder)
    env = dict(os.environ)
    for var, name in (("XDG_CONFIG_HOME", "c"), ("XDG_DATA_HOME", "d"),
                      ("XDG_CACHE_HOME", "a"), ("XDG_RUNTIME_DIR", "r")):
        (test / name / "rustune").mkdir(parents=True, mode=0o700)
        env[var] = str(test / name)
    (test / "c/rustune/config.toml").write_text("daemon=true\nartwork=false\nlyrics=false\n")
    endpoint = test / "r/rustune/service.sock"
    daemon = subprocess.Popen([binary, "daemon"], env=env,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(200):
            if endpoint.exists():
                break
            time.sleep(.05)
        assert endpoint.exists(), "daemon socket never appeared"

        def call(payload):
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
                stream.settimeout(10)
                stream.connect(str(endpoint))
                body = json.dumps(payload).encode()
                stream.sendall(struct.pack(">I", len(body)) + body)
                with stream.makefile("rb") as response:
                    length = struct.unpack(">I", response.read(4))[0]
                    body = response.read(length)
                    assert len(body) == length, "incomplete QueuePage response"
                    return body

        def data(reply):
            envelope = json.loads(reply)
            assert envelope.get("ok"), envelope.get("error")
            return envelope["data"]

        tracks = [{"id": str(i), "kind": "songs", "title": f"Track {i}",
                   "artist": "Fixture", "provider": "music", "storefront": "pl"}
                  for i in range(options.entries)]
        # Synthetic appends keep the queue fully offline; every entry is a
        # plain JSON track, so no credentials or HTTP are involved. Two limits
        # force small batches: the 64 KiB per-command frame limit, and the
        # 64 KiB operation-receipt fingerprint cap, which counts the whole
        # serialized Request. A unique key per batch keeps every append a
        # distinct operation.
        batch_size = 128
        for start in range(0, len(tracks), batch_size):
            revision = data(call({"command": "status", "args": None}))["queue_revision"]
            key = os.urandom(16).hex()
            data(call({"command": "queue_action",
                       "args": {"key": key, "revision": revision,
                                "action": {"intent": "append",
                                           "tracks": tracks[start:start + batch_size],
                                           "context": {"title": "Fixture"}}}}))
        status = data(call({"command": "status", "args": None}))
        assert status["queue_len"] == options.entries, status["queue_len"]

        profile = status["profile"]
        size = {"minimal": 15, "balanced": 25, "enhanced": 50}[profile]
        last_page = min(options.page // 2, (options.entries - 1) // size)
        forward = [index * size for index in range(last_page + 1)]
        backward = list(reversed(forward[:-1]))
        totals = {"legacy": 0, "annotated": 0}
        directional_bytes = {}
        samples = {}
        for mode, field in (("legacy", False), ("annotated", True)):
            directional_bytes[mode] = {}
            for direction, offsets in (("forward", forward), ("backward", backward)):
                byte_count = 0
                for offset in offsets:
                    reply = call({"command": "queue_page",
                                  "args": {"offset": offset, "annotated": field}})
                    page = data(reply)
                    assert page["offset"] == offset and page["total"] == options.entries, page["offset"]
                    expected_size = min(size, options.entries - offset)
                    assert len(page["items"]) == expected_size, (profile, offset, len(page["items"]))
                    byte_count += len(reply)
                    if offset == 0 and direction == "forward":
                        samples[mode] = len(reply)
                directional_bytes[mode][direction] = byte_count
                totals[mode] += byte_count

        # Rank annotations must stay correct while bounded: check every row of
        # three representative pages against the legacy upcoming order.
        def rows(offset, annotated):
            return data(call({"command": "queue_page",
                              "args": {"offset": offset, "annotated": annotated}}))
        legacy_upcoming = rows(0, False)["upcoming"]
        expected = {str(entry_id): position + 1
                    for position, entry_id in enumerate(legacy_upcoming)}
        for offset in dict.fromkeys((forward[0], forward[len(forward) // 2], forward[-1])):
            page = rows(offset, True)
            ranks = page["upcoming_ranks"]
            assert len(ranks) == len(page["items"]), "one annotation per row"
            for row in page["items"]:
                entry_id = row["entry_id"]
                assert ranks[str(entry_id)] == expected.get(str(entry_id)), row["entry_id"]
        report = {
            "entries": options.entries,
            "profile": profile,
            "page_size": size,
            "responses_per_sweep": len(forward) + len(backward),
            "responses_by_direction": {"forward": len(forward), "backward": len(backward)},
            "first_page_bytes": samples,
            "directional_bytes": directional_bytes,
            "sweep_bytes": totals,
            "sweep_saving_percent": round(100 * (totals["legacy"] - totals["annotated"])
                                          / totals["legacy"], 2),
        }
        print(json.dumps(report, indent=2))
    finally:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
            try:
                stream.connect(str(endpoint))
                body = json.dumps({"command": "shutdown", "args": None}).encode()
                stream.sendall(struct.pack(">I", len(body)) + body)
                struct.unpack(">I", stream.recv(4))
            except OSError:
                pass
        daemon.wait(timeout=10)
        shutil.rmtree(test, ignore_errors=True)
