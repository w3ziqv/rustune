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
parser.add_argument("--page", type=int, default=241, help="responses per scroll sweep")
options = parser.parse_args()
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
                stream.connect(str(endpoint))
                body = json.dumps(payload).encode()
                stream.sendall(struct.pack(">I", len(body)) + body)
                length = stream.recv(4)
                if len(length) < 4:
                    length += stream.recv(4 - len(length))
                body = b""
                while len(body) < struct.unpack(">I", length)[0]:
                    body += stream.recv(65536)
                return body

        def data(reply):
            envelope = json.loads(reply)
            assert envelope.get("ok"), envelope.get("error")
            return envelope["data"]

        status = data(call({"command": "status", "args": None}))
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

        size = status["profile"].get("page_size", 15) if isinstance(status.get("profile"), dict) else 15
        offsets = [index * size for index in range(options.page)]
        totals = {"legacy": 0, "annotated": 0}
        samples = {}
        for mode, field in (("legacy", False), ("annotated", True)):
            for offset in offsets:
                reply = call({"command": "queue_page",
                              "args": {"offset": offset, "annotated": field}})
                totals[mode] += len(reply)
                if offset == 0:
                    samples[mode] = len(reply)

        # Rank annotations must stay correct while bounded: check every row of
        # three representative pages against the legacy upcoming order.
        def rows(offset, annotated):
            return data(call({"command": "queue_page",
                              "args": {"offset": offset, "annotated": annotated}}))
        legacy_upcoming = None
        for offset in (0, size * 66, size * 666):
            legacy = rows(offset, False)
            page = rows(offset, True)
            legacy_upcoming = legacy_upcoming or legacy["upcoming"]
            ranks = page["upcoming_ranks"]
            assert len(ranks) == len(page["items"]), "one annotation per row"
            expected = {str(entry_id): position + 1
                        for position, entry_id in enumerate(legacy_upcoming)}
            for row in page["items"]:
                entry_id = row["entry_id"]
                assert ranks[str(entry_id)] == expected.get(str(entry_id)), row["entry_id"]
        report = {
            "entries": options.entries,
            "page_size": size,
            "responses_per_sweep": options.page,
            "first_page_bytes": samples,
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
