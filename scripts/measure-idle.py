#!/usr/bin/env python3
"""Isolated idle TUI/service measurement. Never connects to the user's service."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary')
args = parser.parse_args()
binary = str(Path(args.binary).resolve())

def ticks(pid):
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return int(fields[11]) + int(fields[12])

def memory(pid):
    fields = {}
    for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
        if line.startswith(('Rss:', 'Pss:')):
            fields[line.split(':')[0].lower() + '_kib'] = int(line.split()[1])
    return fields

with tempfile.TemporaryDirectory(prefix='rustune-idle-') as directory:
    root = Path(directory)
    env = {**os.environ, 'TERM': 'xterm-256color'}
    for key, name in [('XDG_CONFIG_HOME','config'), ('XDG_DATA_HOME','data'),
                      ('XDG_CACHE_HOME','cache'), ('XDG_RUNTIME_DIR','run')]:
        env[key] = str(root / name)
        (root / name).mkdir(mode=0o700)
    bus = subprocess.Popen(['dbus-daemon','--session','--nofork','--print-address'],
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    daemon = tui = None
    master = slave = None
    try:
        env['DBUS_SESSION_BUS_ADDRESS'] = bus.stdout.readline().strip()
        subprocess.run([binary,'init'], env=env, stdout=subprocess.DEVNULL, check=True)
        daemon = subprocess.Popen([binary,'daemon'], env=env, stdout=subprocess.DEVNULL,
                                  stderr=subprocess.PIPE)
        for _ in range(100):
            if (root/'run/rustune/service.sock').exists(): break
            if daemon.poll() is not None: raise RuntimeError('isolated service exited')
            time.sleep(.02)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH',30,100,0,0))
        started = time.monotonic()
        tui = subprocess.Popen([binary], stdin=slave, stdout=slave, stderr=slave,
                               env=env, start_new_session=True)
        os.close(slave)
        slave = None
        captured = bytearray()
        def drain(seconds):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                if select.select([master],[],[],min(.05,max(0,end-time.monotonic())))[0]:
                    try: captured.extend(os.read(master,65536))
                    except OSError: break
        while b'RUSTUNE' not in captured and time.monotonic()-started < 5: drain(.01)
        assert b'RUSTUNE' in captured, 'TUI did not render'
        first_frame_ms = (time.monotonic()-started)*1000
        drain(.2)
        pids = [daemon.pid,tui.pid]
        before = sum(ticks(pid) for pid in pids)
        started = time.monotonic()
        drain(5)
        seconds = time.monotonic()-started
        cpu = (sum(ticks(pid) for pid in pids)-before)/os.sysconf('SC_CLK_TCK')/seconds*100
        values = [memory(pid) for pid in pids]
        status = json.loads(subprocess.check_output([binary,'status'], env=env))
        assert status['browser_pid'] is None
        assert status['playback_available'] is False
        os.write(master,b'?'); drain(.2)
        assert b'HELP' in captured
        for height,width in [(10,40),(4,20),(48,160),(30,100)]:
            fcntl.ioctl(master,termios.TIOCSWINSZ,struct.pack('HHHH',height,width,0,0))
            tui.send_signal(signal.SIGWINCH); drain(.1)
        os.write(master,b'q'); drain(.2)
        tui.wait(timeout=5)
        assert tui.returncode == 0
        print(json.dumps({'scenario':'isolated idle TUI + service, balanced, no playback engine',
            'native_rss_kib':sum(v['rss_kib'] for v in values),
            'native_pss_kib':sum(v['pss_kib'] for v in values),
            'native_cpu_percent_one_core':cpu,'sample_seconds':seconds,
            'first_tui_frame_ms':first_frame_ms,'cold_cache':False,
            'browser_processes':0,'browser_rss_kib':0,'browser_pss_kib':0,
            'help':True,'resizes':4,'quit_exit':0,'audio_tested':False}, indent=2))
    finally:
        for process in [tui,daemon,bus]:
            if process is not None and process.poll() is None:
                process.terminate()
                try: process.wait(timeout=5)
                except subprocess.TimeoutExpired: process.kill(); process.wait()
        for fd in [master,slave]:
            if fd is not None: os.close(fd)
