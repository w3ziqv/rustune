#!/usr/bin/env python3
"""Real TUI in PTY with synthetic concurrent IPC; no Apple account, audio or CDM.

Requires pyte in an isolated test venv. All XDG paths are temporary.
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
import pyte

binary = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='rustune-playlists-') as folder:
    root = Path(folder)
    env = {**os.environ, 'TERM': 'xterm-256color', 'DBUS_SESSION_BUS_ADDRESS': 'unix:path=/nonexistent-rustune-fixture'}
    for key, name in [('XDG_CONFIG_HOME', 'config'), ('XDG_DATA_HOME', 'data'), ('XDG_CACHE_HOME', 'cache'), ('XDG_RUNTIME_DIR', 'run')]:
        env[key] = str(root/name)
        (root/name/'rustune').mkdir(parents=True, mode=0o700)
    (root/'config/rustune/config.toml').write_text('artwork = false\nlyrics = false\n')
    status = {'playback': {'phase': 'stopped'}, 'queue_len': 3, 'queue_revision': 7, 'current': 0,
              'repeat': 'off', 'shuffle': False, 'profile': 'balanced', 'pid': os.getpid(),
              'browser_pid': None, 'backend': 'Synthetic playlist fixture', 'browser_shared': False,
              'uptime_secs': 0, 'cache_entries': 0, 'message': 'No account or audio', 'sleep_remaining': None}
    rows = [{'id': 'p.readonly', 'kind': 'library-playlists', 'title': 'Synthetic readonly', 'provider': 'music', 'storefront': 'pl', 'can_edit': False},
            {'id': 'p.editable', 'kind': 'library-playlists', 'title': 'Synthetic editable', 'provider': 'music', 'storefront': 'pl', 'can_edit': True}]
    writes = []
    stopping = threading.Event()
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(str(root/'run/rustune/service.sock'))
    listener.listen(16)
    listener.settimeout(.1)
    def client_work(client):
        try:
            client.settimeout(2)
            def read(n):
                data = b''
                while len(data) < n:
                    chunk = client.recv(n-len(data))
                    if not chunk:
                        raise OSError('closed')
                    data += chunk
                return data
            n = struct.unpack('>I', read(4))[0]
            assert n <= 65536
            request = json.loads(read(n))
            command = request['command']
            if command == 'browse':
                data = {'items': rows, 'next': None, 'storefront': 'pl'}
            else:
                if command == 'playlist_write':
                    writes.append(request)
                    status['playlist_write'] = {'id': request['args']['key'], 'phase': 'completed', 'tracks': 3,
                                                'result': {'playlist_id': 'p.created', 'tracks': 3}, 'message': 'Synthetic confirmation', 'can_retry': False}
                data = status
            response = json.dumps({'ok': True, 'data': data, 'error': None}).encode()
            client.sendall(struct.pack('>I', len(response))+response)
            if command == 'subscribe':
                stopping.wait(15)
        except (OSError, ValueError):
            pass
        finally:
            client.close()
    def serve():
        while not stopping.is_set():
            try:
                client, _ = listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            threading.Thread(target=client_work, args=(client,), daemon=True).start()
    threading.Thread(target=serve, daemon=True).start()
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 140, 0, 0))
    process = subprocess.Popen([binary], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
    screen = pyte.Screen(140, 40)
    stream = pyte.Stream(screen)
    decoder = codecs.getincrementaldecoder('utf8')('replace')
    def pump(seconds=.2):
        deadline = time.monotonic()+seconds
        while time.monotonic() < deadline:
            if select.select([master], [], [], .02)[0]:
                try:
                    stream.feed(decoder.decode(os.read(master, 65536)))
                except OSError:
                    break
    def send(keys):
        os.write(master, keys)
        pump()
    def expect(text):
        deadline = time.monotonic()+4
        while text not in '\n'.join(screen.display) and time.monotonic() < deadline:
            pump(.05)
        assert text in '\n'.join(screen.display), f'Missing {text!r}:\n'+ '\n'.join(screen.display)
    checks = []
    try:
        pump(.4)
        send(b':playlists\r')
        expect('MY PLAYLISTS')
        expect('Synthetic readonly [read-only]')
        expect('Synthetic editable [editable]')
        checks.append('playlist list and edit permissions visible')
        send(b'A')
        expect('read-only')
        assert not writes
        send(b'N')
        expect('NEW APPLE MUSIC PLAYLIST')
        send(b'Synthetic empty\r')
        expect('CONFIRM ACCOUNT WRITE')
        expect('Enter SEND to Apple')
        assert not writes
        send(b'\x1b')
        pump(.15)
        assert not writes
        checks.append('name and cancelled confirmation never write')
        send(b'S')
        send(b'Synthetic queue\r')
        expect('Include 3 tracks')
        assert not writes
        send(b'\r')
        deadline = time.monotonic()+3
        while not writes and time.monotonic() < deadline:
            pump(.05)
        assert len(writes) == 1
        args = writes[0]['args']
        assert args['source'] == {'source': 'queue', 'revision': 7}
        assert args['action'] == {'intent': 'create', 'name': 'Synthetic queue', 'description': ''}
        checks.append('one explicit confirmation emits one complete queue write')
        send(b'q')
        deadline = time.monotonic()+3
        while process.poll() is None and time.monotonic() < deadline:
            pump(.05)
        process.wait(timeout=1)
        assert process.returncode == 0
        print(json.dumps({'checks': checks, 'account_posts': 0, 'synthetic_write_requests': len(writes), 'actual_audio_verified': False}, indent=2))
    finally:
        stopping.set()
        listener.close()
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
        os.close(slave)
