#!/usr/bin/env python3
"""Real TUI/PTY artwork regression with inherited NO_COLOR=1.

Public Apple image + synthetic IPC status; no account, credentials, CDM or audio.
Requires pyte in a test-only virtualenv. Does not connect to the user's service.
"""
import codecs
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time

try:
    import pyte
except ImportError:
    sys.exit('Requires pyte in a test-only virtualenv.')

binary = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='rustune-artwork-display-') as folder:
    root = Path(folder)
    env = {**os.environ, 'TERM': 'xterm-kitty', 'COLORTERM': 'truecolor', 'NO_COLOR': '1',
           'DBUS_SESSION_BUS_ADDRESS': 'unix:path=/nonexistent-rustune-artwork-test-bus'}
    for key, name in [('XDG_CONFIG_HOME', 'config'), ('XDG_DATA_HOME', 'data'),
                      ('XDG_CACHE_HOME', 'cache'), ('XDG_RUNTIME_DIR', 'run')]:
        env[key] = str(root/name)
        (root/name/'rustune').mkdir(parents=True, mode=0o700)
    (root/'config/rustune/config.toml').write_text('artwork = true\nlyrics = false\nperformance = "enhanced"\n')
    status = {'playback': {'phase': 'stopped', 'track': {
        'id': 'fixture', 'kind': 'songs', 'title': 'Synthetic artwork display',
        'artist': 'Fixture', 'album': 'Public artwork; no audio',
        'artwork': 'https://is1-ssl.mzstatic.com/image/thumb/Music112/v4/a9/52/4d/a9524d7e-631b-ece3-cf70-cf37023e8251/886443710016.jpg/{w}x{h}bb.jpg'}},
        'queue_len': 1, 'current': 0, 'repeat': 'off', 'shuffle': False,
        'profile': 'enhanced', 'pid': os.getpid(), 'browser_pid': None,
        'backend': 'Synthetic artwork fixture', 'browser_shared': False,
        'uptime_secs': 0, 'cache_entries': 0, 'message': 'Artwork rendering test; no audio',
        'sleep_remaining': None}
    reply = json.dumps({'ok': True, 'data': status, 'error': None}).encode()
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(str(root/'run/rustune/service.sock'))
    listener.listen(4)
    listener.settimeout(.2)
    stopping = threading.Event()
    def serve():
        while not stopping.is_set():
            try:
                client, _ = listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            client.settimeout(2)
            try:
                def read(n):
                    data = b''
                    while len(data) < n:
                        part = client.recv(n-len(data))
                        if not part:
                            raise OSError('closed')
                        data += part
                    return data
                n = struct.unpack('>I', read(4))[0]
                assert n <= 65536
                request = json.loads(read(n))
                client.sendall(struct.pack('>I', len(reply))+reply)
                if request['command'] == 'subscribe':
                    # Keep the subscribed socket alive while the TUI downloads the cover.
                    while not stopping.wait(.1):
                        pass
            except (OSError, ValueError):
                pass
            finally:
                client.close()
    server = threading.Thread(target=serve, daemon=True)
    server.start()
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 69, 140, 0, 0))
    process = subprocess.Popen([binary], stdin=slave, stdout=slave, stderr=slave,
                               env=env, start_new_session=True)
    screen = pyte.Screen(140, 69)
    stream = pyte.Stream(screen)
    decoder = codecs.getincrementaldecoder('utf8')('replace')
    block_count = colored = 0
    try:
        deadline = time.monotonic()+12
        while time.monotonic() < deadline:
            if select.select([master], [], [], .05)[0]:
                stream.feed(decoder.decode(os.read(master, 65536)))
            cells = [screen.buffer[y][x] for y in range(69) for x in range(1, 60)]
            block_count = sum(c.data in ('▀', '▄') for c in cells)
            colored = sum(any(v not in ('default', '000000', 'ffffff', 'black', 'white')
                              for v in (c.fg, c.bg)) for c in cells)
            if block_count > 100:
                break
            if process.poll() is not None:
                raise AssertionError('TUI exited before displaying artwork')
        assert block_count > 100, 'Public artwork was not downloaded and rendered'
        assert colored > 100, 'Cover became monochrome block noise: NO_COLOR suppressed RGB output'
        os.write(master, b'q')
        # A large truecolor frame exceeds PTY's output buffer. Keep reading while
        # the client finishes that frame and consumes q; otherwise the harness
        # itself blocks the renderer and falsely reports a failed shutdown.
        deadline = time.monotonic()+5
        while process.poll() is None and time.monotonic() < deadline:
            if select.select([master], [], [], .05)[0]:
                try:
                    os.read(master, 65536)
                except OSError:
                    break
        process.wait(timeout=1)
        assert process.returncode == 0
        print(json.dumps({'inherited_no_color': True, 'halfblock_cells': block_count,
                          'cells_preserving_image_colors': colored, 'rows': 69, 'columns': 140,
                          'actual_audio_verified': False}, indent=2))
    finally:
        stopping.set()
        listener.close()
        server.join(timeout=3)
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
        os.close(slave)
