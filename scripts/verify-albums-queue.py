#!/usr/bin/env python3
"""Offline PTY + real isolated IPC smoke. Synthetic metadata, no account/CDM/audio."""
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
import time

try:
    import pyte
except ImportError:
    sys.exit('This isolated PTY smoke requires pyte in a test-only virtual environment.')
binary = str(Path(sys.argv[1]).resolve())
report = {'checks': [], 'actual_audio_verified': False}
with tempfile.TemporaryDirectory(prefix='rustune-albums-queue-') as directory:
    root = Path(directory)
    env = {**os.environ, 'TERM': 'xterm-256color'}
    for key, name in [('XDG_CONFIG_HOME', 'config'), ('XDG_DATA_HOME', 'data'),
                      ('XDG_CACHE_HOME', 'cache'), ('XDG_RUNTIME_DIR', 'run')]:
        (root/name).mkdir(mode=0o700)
        env[key] = str(root/name)
    bus = subprocess.Popen(['dbus-daemon', '--session', '--nofork', '--print-address'],
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    daemon = tui = None
    master = slave = None
    try:
        env['DBUS_SESSION_BUS_ADDRESS'] = bus.stdout.readline().strip()
        subprocess.run([binary, 'init'], env=env, check=True, stdout=subprocess.DEVNULL)
        config = root/'config/rustune/config.toml'
        config.write_text('artwork = false\nlyrics = false\ndaemon = true\n')
        data = root/'data/rustune'
        state = {'queue': {'tracks': [
            {'id': '11', 'kind': 'songs', 'title': 'Synthetic first'},
            {'id': '11', 'kind': 'songs', 'title': 'Synthetic duplicate'},
            {'id': '12', 'kind': 'songs', 'title': 'Synthetic last'}],
            'current': 0, 'repeat': 'all', 'shuffle': False}}
        (data/'state.json').write_text(json.dumps(state))
        (data/'state.json').chmod(0o600)
        runtime = root/'run/rustune/service.sock'
        def rpc(command, args=None):
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
                client.settimeout(5)
                client.connect(str(runtime))
                request = {'command': command}
                if args is not None:
                    request['args'] = args
                body = json.dumps(request).encode()
                client.sendall(struct.pack('>I', len(body))+body)
                def read(n):
                    value = b''
                    while len(value) < n:
                        chunk = client.recv(n-len(value))
                        assert chunk, 'IPC closed before reply'
                        value += chunk
                    return value
                length = struct.unpack('>I', read(4))[0]
                assert length <= 4*1024*1024
                reply = json.loads(read(length))
                assert reply['ok'], reply.get('error')
                return reply['data']
        def start_service():
            process = subprocess.Popen([binary, 'daemon'], env=env,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            end = time.monotonic()+5
            while time.monotonic() < end:
                try:
                    rpc('status')
                    return process
                except (OSError, AssertionError):
                    assert process.poll() is None, 'isolated daemon exited'
                    time.sleep(.02)
            raise AssertionError('isolated service did not start')
        daemon = start_service()
        entries = rpc('queue_page', {'offset': 0})['items']
        assert len({e['entry_id'] for e in entries}) == 3
        assert entries[0]['id'] == entries[1]['id'] == '11'
        assert rpc('status')['playback']['phase'] == 'stopped'
        report['checks'].append('legacy duplicates migrate to distinct occurrences without autoplay')
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 120, 0, 0))
        tui = subprocess.Popen([binary], stdin=slave, stdout=slave, stderr=slave,
                               env=env, start_new_session=True)
        screen=pyte.Screen(120,30)
        stream=pyte.Stream(screen)
        decoder=codecs.getincrementaldecoder("utf8")("replace")
        def text():
            return "\n".join(screen.display)
        def drain(seconds=.15):
            end = time.monotonic()+seconds
            while time.monotonic() < end:
                if select.select([master], [], [], min(.03, max(0, end-time.monotonic())))[0]:
                    try:
                        stream.feed(decoder.decode(os.read(master, 65536)))
                    except OSError:
                        break
        def wait(predicate):
            end = time.monotonic()+5
            while time.monotonic() < end:
                if predicate():
                    return
                drain(.03)
            raise AssertionError('PTY/service did not reach expected state')
        wait(lambda: 'RUSTUNE' in text())
        os.write(master, b':queue\r')
        wait(lambda: 'QUEUE' in text() and 'Synthetic duplicate' in text())
        os.write(master, b'\r')
        wait(lambda: 'SONG ACTIONS' in text())
        os.write(master, b'\x1b[B\r')
        wait(lambda: rpc('status')['playback']['phase'] == 'failed')
        wait(lambda: 'Failed:' in text())
        report['checks'].append('keyboard Queue Play uses occurrence; async Failed is visible')
        # Clear retains the selected current occurrence and repeat, without starting audio.
        os.write(master, b':clear\r')
        wait(lambda: rpc('status')['queue_len'] == 1)
        assert rpc('status')['repeat'] == 'all'
        os.write(master, b'u')
        wait(lambda: rpc('status')['queue_len'] == 3)
        assert rpc('status')['playback']['phase'] == 'stopped'
        report['checks'].append('clear upcoming and one-step undo preserve session and modes, no autoplay')
        before = rpc('queue_page', {'offset': 0})
        # Identity edits and retry keys preserve deliberate duplicates across clients.
        args = {'key': 'pty-move', 'revision': before['revision'],
                'action': {'intent': 'move_by', 'entry_id': before['items'][1]['entry_id'], 'delta': 1}}
        rpc('queue_action', args)
        rpc('queue_action', args)
        after = rpc('queue_page', {'offset': 0})
        assert len(after['items']) == 3
        assert after['items'][2]['entry_id'] == before['items'][1]['entry_id']
        try:
            rpc('queue_action', {'key': 'pty-stale-remove', 'revision': before['revision'],
                                'action': {'intent': 'remove_and_advance', 'entry_id': before['items'][0]['entry_id']}})
        except AssertionError as error:
            assert 'Queue changed' in str(error)
        else:
            raise AssertionError('stale client unexpectedly edited queue')
        report['checks'].append('second-client occurrence move, stale-edit rejection and idempotent retry')
        # Sample the exact owned TUI/daemon PIDs. There is no audio helper/browser in this smoke.
        def ticks(pid):
            fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
            return int(fields[11])+int(fields[12])
        def owned_processes():
            owned = {tui.pid, daemon.pid}
            pending = list(owned)
            while pending:
                parent = pending.pop()
                for task in Path(f'/proc/{parent}/task').glob('*'):
                    try:
                        children = (task/'children').read_text().split()
                    except FileNotFoundError:
                        continue
                    for child in map(int, children):
                        if child not in owned:
                            owned.add(child)
                            pending.append(child)
            return owned
        pids = owned_processes()
        first = {pid: ticks(pid) for pid in pids}
        started = time.monotonic()
        drain(2)
        seconds = time.monotonic()-started
        report['sample_seconds'] = seconds
        pids = owned_processes()
        report['cpu_percent'] = sum(ticks(pid)-first.get(pid, 0) for pid in pids)/os.sysconf('SC_CLK_TCK')/seconds*100
        report['owned_processes'] = len(pids)
        report['exited_during_sample'] = len(set(first)-pids)
        names = [Path(f'/proc/{pid}/comm').read_text().strip() for pid in pids]
        report['browser_processes'] = sum(any(browser in name.lower() for browser in ['chrom', 'firefox', 'msedge', 'brave']) for name in names)
        report['audio_processes'] = sum(name.startswith('rustune-audio') for name in names)
        assert report['browser_processes'] == report['audio_processes'] == 0
        report['rss_kib'] = report['pss_kib'] = 0
        for pid in pids:
            for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
                if line.startswith('Rss:'):
                    report['rss_kib'] += int(line.split()[1])
                elif line.startswith('Pss:'):
                    report['pss_kib'] += int(line.split()[1])
        s = rpc('status')
        assert s['browser_pid'] is None and s['audio_pid'] is None
        for height, width in [(10, 40), (4, 20), (48, 160), (30, 120)]:
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', height, width, 0, 0))
            screen.resize(height,width)
            tui.send_signal(__import__('signal').SIGWINCH)
            drain(.1)
            assert tui.poll() is None
        os.write(master, b'q')
        tui.wait(timeout=5)
        assert tui.returncode == 0
        restored_flags = termios.tcgetattr(slave)[3]
        assert restored_flags & termios.ECHO and restored_flags & termios.ICANON
        report['checks'].append('four PTY sizes; clean quit restores terminal')
        saved = rpc('queue_page', {'offset': 0})['items']
        rpc('shutdown')
        daemon.wait(timeout=5)
        daemon = start_service()
        assert rpc('queue_page', {'offset': 0})['items'] == saved
        assert rpc('status')['playback']['phase'] == 'stopped'
        assert (data/'state.previous.json').exists()
        report['checks'].append('durable ordered state, backup and restart without autoplay')
        rpc('shutdown')
        daemon.wait(timeout=5)
        print(json.dumps(report, indent=2))
    finally:
        for process in [tui, daemon, bus]:
            if process is not None and process.poll() is None:
                process.kill()
                process.wait()
        for descriptor in [master, slave]:
            if descriptor is not None:
                os.close(descriptor)
