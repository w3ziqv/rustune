#!/usr/bin/env python3
"""Check that both termination signals restore the PTY and close the owned service."""
import os
import pty
import select
import signal
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path

binary = str(Path(sys.argv[1]).resolve())
for sig in (signal.SIGTERM, signal.SIGHUP):
    with tempfile.TemporaryDirectory(prefix="rustune-signals-") as folder:
        env = dict(os.environ, TERM="xterm-256color")
        for key in ("XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR"):
            path = Path(folder) / key
            path.mkdir(mode=0o700)
            env[key] = str(path)
        config = Path(env["XDG_CONFIG_HOME"]) / "rustune"
        config.mkdir(mode=0o700)
        (config / "config.toml").write_text("daemon=false\n")
        master, slave = pty.openpty()
        termios.tcsetwinsize(slave, (30, 100))
        before = termios.tcgetattr(slave)
        with subprocess.Popen(["dbus-run-session", "--", binary], env=env, stdin=slave,
                              stdout=slave, stderr=slave, start_new_session=True) as process:
            try:
                deadline = time.monotonic() + 8
                data = bytearray()
                while b"RUSTUNE" not in data and time.monotonic() < deadline:
                    if select.select([master], [], [], .1)[0]:
                        data.extend(os.read(master, 65536))
                assert b"RUSTUNE" in data
                children = Path(f"/proc/{process.pid}/task/{process.pid}/children")
                client = next(int(p) for p in children.read_text().split()
                              if os.path.realpath(f"/proc/{p}/exe") == binary)
                owned = Path(f"/proc/{client}/task/{client}/children").read_text().split()
                os.kill(client, sig)
                assert process.wait(timeout=6) == 0
                assert termios.tcgetattr(slave) == before
                assert owned and all(not Path(f"/proc/{p}").exists() for p in owned)
                print(sig.name, "restored PTY and reaped owned service")
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                os.close(master)
                os.close(slave)
