#!/usr/bin/env python3
"""Isolated opt-in native playback / terminal measurements. No private metadata in reports.

Requires pyte; --terminal kitty additionally requires a graphical Linux session.
A PTY is not a graphics emulator. Smooth rendering is verified only with real Kitty.
"""
import argparse, codecs, fcntl, hashlib, json, os, pty, select, shutil, signal
import socket, struct, subprocess, tempfile, termios, time
from pathlib import Path
import pyte

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary'); p.add_argument('report')
p.add_argument('--live', action='store_true', required=True)
p.add_argument('--seconds', type=int, default=30)
p.add_argument('--style', choices=['pixelated', 'smooth', 'off'], default='pixelated')
p.add_argument('--terminal', choices=['pty', 'kitty'], default='pty')
p.add_argument('--legacy', action='store_true', help='Baseline before the navigation/style change')
p.add_argument('--cycle', action='store_true', help='Resize and switch views while measuring')
p.add_argument('--screenshots', type=Path)
a = p.parse_args()
assert 10 <= a.seconds <= 7200
assert a.style != 'smooth' or a.terminal == 'kitty', 'smooth requires an actual graphics terminal'
binary = Path(a.binary).resolve(); report = Path(a.report).resolve()
result = dict(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
              helper_sha256=hashlib.sha256(binary.with_name('rustune-audio').read_bytes()).hexdigest(),
              seconds=a.seconds, style=a.style, terminal=a.terminal, cycle=a.cycle,
              owned_browser_processes=0, live_playback_verified=False, samples=[])

with tempfile.TemporaryDirectory(prefix='rustune-clean-measure-') as folder:
    root = Path(folder)
    env = dict(os.environ, TERM='xterm-256color', COLORTERM='truecolor')
    original_runtime = os.environ.get('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}')
    env['PIPEWIRE_RUNTIME_DIR'] = os.environ.get('PIPEWIRE_RUNTIME_DIR', original_runtime)
    if env.get('WAYLAND_DISPLAY') and not env['WAYLAND_DISPLAY'].startswith('/'):
        env['WAYLAND_DISPLAY'] = str(Path(original_runtime)/env['WAYLAND_DISPLAY'])
    for key, name in [('XDG_CONFIG_HOME','config'),('XDG_DATA_HOME','data'),('XDG_CACHE_HOME','cache'),('XDG_RUNTIME_DIR','run')]:
        env[key] = str(root/name); (root/name/'rustune').mkdir(parents=True, mode=0o700)
    config = root/'config/rustune'; cache = root/'cache/rustune'; runtime = root/'run/rustune/service.sock'
    for source, dest in [(Path.home()/'.config/rustune/music-user-token',config/'music-user-token'),
                         (Path.home()/'.cache/rustune/drm/libwidevinecdm.so',cache/'drm/libwidevinecdm.so'),
                         (Path.home()/'.cache/rustune/apple-app-token',cache/'apple-app-token')]:
        dest.parent.mkdir(parents=True, exist_ok=True, mode=0o700); shutil.copyfile(source,dest); dest.chmod(0o600)
    options = 'volume=0.08\ndaemon=true\nmouse=true\nperformance="balanced"\n'
    if a.style == 'off': options += 'artwork=false\n'
    elif a.legacy and a.style=='smooth': options += 'artwork_protocol="kitty"\n'
    elif not a.legacy: options += f'artwork_style="{a.style}"\n'
    (config/'config.toml').write_text(options)
    track = dict(id='1440838314',kind='songs',title='Simple Man',artist='Lynyrd Skynyrd',
        album='Pronounced Leh-Nerd Skin-Nerd (Bonus Tracks)',duration_ms=359680,
        artwork='https://is1-ssl.mzstatic.com/image/thumb/Music115/v4/1e/5c/85/1e5c85bb-2b43-3057-da09-39446321a191/06UMGIM14960.rgb.jpg/{w}x{h}bb.jpg')
    state = root/'data/rustune/state.json'
    state.write_text(json.dumps({'queue':{'tracks':[track], 'current':0,'repeat':'all','shuffle':False,'revision':1}})); state.chmod(0o600)
    bus = daemon = tui = terminal = None; master = None
    remote = f'unix:{root}/kitty.sock'
    screen = pyte.Screen(142,39); stream = pyte.Stream(screen); decoder = codecs.getincrementaldecoder('utf8')('replace')
    def rpc(command, args=None):
        with socket.socket(socket.AF_UNIX) as s:
            s.settimeout(8); s.connect(str(runtime)); payload={'command':command}
            if args is not None: payload['args']=args
            body=json.dumps(payload).encode(); s.sendall(struct.pack('>I',len(body))+body)
            def read(n):
                data=b''
                while len(data)<n:
                    part=s.recv(n-len(data))
                    if not part: raise RuntimeError('measurement socket closed')
                    data+=part
                return data
            n=struct.unpack('>I',read(4))[0]; assert n<=4*1024*1024
            reply=json.loads(read(n))
            if not reply.get('ok'): raise RuntimeError('measurement RPC rejected; private error withheld')
            return reply['data']
    def kitty(*args):
        return subprocess.check_output(['kitty','@','--to',remote,*args],stderr=subprocess.DEVNULL)
    def send(text):
        if a.terminal=='pty': os.write(master,text.encode())
        else: kitty('send-text','--',text)
    def drain(seconds):
        until=time.monotonic()+seconds
        while time.monotonic()<until:
            if master is None: time.sleep(min(.05,max(0,until-time.monotonic()))); continue
            if select.select([master],[],[],min(.05,max(0,until-time.monotonic())))[0]:
                try: stream.feed(decoder.decode(os.read(master,65536)))
                except OSError: return
    def text():
        return '\n'.join(screen.display) if a.terminal=='pty' else kitty('get-text').decode()
    def sample(s):
        ids=[('service',daemon.pid),('tui',tui.pid if tui else tui_pid)]
        if s.get('audio_pid'): ids.append(('audio',s['audio_pid']))
        if terminal: ids.append(('terminal',terminal.pid))
        rows=[]
        for role,pid in ids:
            try:
                mem={l.split(':')[0]:int(l.split()[1]) for l in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines() if l.startswith(('Rss:','Pss:'))}
                stat=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
                rows.append(dict(role=role,pid=pid,start_ticks=int(stat[19]),rss_kib=mem['Rss'],pss_kib=mem['Pss'],cpu_ticks=int(stat[11])+int(stat[12])))
            except (OSError,ValueError,KeyError):
                result['incomplete_samples']=result.get('incomplete_samples',0)+1
        return rows
    def screenshot(name):
        if not a.screenshots or not terminal: return
        windows=json.loads(subprocess.check_output(['niri','msg','--json','windows']))
        own=next((w for w in windows if w.get('title')=='Rustune validation'),None)
        if own:
            a.screenshots.mkdir(parents=True,exist_ok=True)
            subprocess.run(['niri','msg','action','screenshot-window','--id',str(own['id']),'--path',str((a.screenshots/name).resolve())],check=True,stdout=subprocess.DEVNULL)
    try:
        bus=subprocess.Popen(['dbus-daemon','--session','--nofork','--print-address'],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True)
        env['DBUS_SESSION_BUS_ADDRESS']=bus.stdout.readline().strip()
        daemon=subprocess.Popen([str(binary),'daemon'],env=env,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        deadline=time.monotonic()+8
        while not runtime.exists() and time.monotonic()<deadline: time.sleep(.02)
        assert runtime.exists(), 'isolated daemon unavailable'
        if a.seconds >= 3600:
            page=rpc('browse',dict(kind='search:songs',term='Lynyrd Skynyrd Free Bird',next=None))
            other=next((t for t in page['items'] if t['title'].lower()=='free bird' and t['artist']=='Lynyrd Skynyrd'),None)
            assert other, 'second public catalog song unavailable'
            rpc('enqueue',dict(track=other,next=False,play=False))
            deadline=time.monotonic()+10
            while rpc('status')['queue_len'] < 2 and time.monotonic()<deadline: drain(.05)
            assert rpc('status')['queue_len'] == 2
        if a.terminal=='pty':
            master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',39,142,0,0))
            tui=subprocess.Popen([str(binary)],env=env,stdin=slave,stdout=slave,stderr=slave,start_new_session=True);os.close(slave)
        else:
            terminal=subprocess.Popen(['kitty','--config','NONE','--listen-on',remote,'--title','Rustune validation',
                '-o','allow_remote_control=yes','-o','font_size=10','-o','initial_window_width=1100','-o','initial_window_height=680',str(binary)],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            deadline=time.monotonic()+10
            while not (root/'kitty.sock').exists() and time.monotonic()<deadline: time.sleep(.05)
            windows=json.loads(kitty('ls')); tui_pid=windows[0]['tabs'][0]['windows'][0]['pid']
        drain(.7); send('2' if a.legacy else '5'); drain(.3); rpc('play')
        deadline=time.monotonic()+40
        while time.monotonic()<deadline:
            s=rpc('status')
            if s['playback']['playing'] and s['playback']['encrypted_packets']>0: break
            if s.get('audio_error'): raise RuntimeError('native playback failed; private backend details withheld')
            drain(.2)
        else: raise RuntimeError('real playback did not start')
        drain(4)
        if a.style=='pixelated': assert '▀' in text(), 'pixelated cover not rendered'
        if a.style=='smooth':
            screenshot('smooth-diagnostic.png')
            visible=text(); result['graphics_diagnostic']={'unavailable':'Smooth artwork unavailable' in visible,'loading':'Loading artwork' in visible,'placeholders':visible.count('\U0010eeee')}
            assert '\U0010eeee' in visible and 'Smooth artwork unavailable' not in visible, 'Kitty image placeholders missing'
            result['real_kitty_graphics_verified']=True

        result['live_playback_verified']=True
        started=time.monotonic(); last_checkpoint=-1; last_cycle=-1; completed=0; previous_pid=s['audio_pid']; previous_position=0.; previous_duration=0.
        while time.monotonic()-started < a.seconds:
            s=rpc('status'); play=s['playback']
            if s.get('audio_error'): raise RuntimeError('native playback failed during measurement')
            if previous_pid != s.get('audio_pid'):
                if previous_position >= previous_duration-2 and previous_duration>300: completed+=1
                previous_pid=s.get('audio_pid'); previous_position=0.
            if play['playing']:
                previous_position=play['position']; previous_duration=play['duration']
            rows=sample(s); native=[r for r in rows if r['role']!='terminal']
            result['samples'].append(dict(seconds=time.monotonic()-started,processes=rows,
                rss_kib=sum(r['rss_kib'] for r in native),pss_kib=sum(r['pss_kib'] for r in native),
                position=play['position'],duration=play['duration'],phase=play.get('phase'),encrypted_packets=play.get('encrypted_packets'),decoded_frames=play.get('decoded_audio_frames')))
            checkpoint=int((time.monotonic()-started)//60)
            if checkpoint != last_checkpoint:
                last_checkpoint=checkpoint
                result['completed_songs_observed']=completed
                report.parent.mkdir(parents=True,exist_ok=True)
                report.with_suffix('.partial.json').write_text(json.dumps(result,indent=2))
            cycle=int((time.monotonic()-started)//5)
            if a.cycle and cycle != last_cycle:
                last_cycle=cycle
                if master is not None:
                    w,h=[(142,39),(120,32),(100,30),(160,42)][cycle%4]
                    screen.resize(h,w); fcntl.ioctl(master,termios.TIOCSWINSZ,struct.pack('HHHH',h,w,0,0));tui.send_signal(signal.SIGWINCH)
                send(('8' if a.legacy else '4') if cycle%3==1 else ('2' if a.legacy else '5'))
            drain(1)
        result['completed_songs_observed']=completed
        samples=result['samples']; result.update(peak_pss_kib=max(s['pss_kib'] for s in samples),
            peak_rss_kib=max(s['rss_kib'] for s in samples),mean_pss_kib=sum(s['pss_kib'] for s in samples)/len(samples),
            mean_rss_kib=sum(s['rss_kib'] for s in samples)/len(samples))
        ticks={'native':0,'terminal':0}; previous={}
        for entry in samples:
            current={(r['pid'],r['start_ticks']):r for r in entry['processes']}
            if previous:
                for identity,row in current.items():
                    delta=max(0,row['cpu_ticks']-previous.get(identity,{'cpu_ticks':0})['cpu_ticks'])
                    ticks['terminal' if row['role']=='terminal' else 'native']+=delta
            previous=current
        elapsed=samples[-1]['seconds']-samples[0]['seconds']
        result['cpu_percent_one_core']={key:value/os.sysconf('SC_CLK_TCK')/elapsed*100 for key,value in ticks.items()}
        result['pss_growth_kib']=samples[-1]['pss_kib']-samples[0]['pss_kib']
        terminal_rows=[r for s in samples for r in s['processes'] if r['role']=='terminal']
        if terminal_rows: result['terminal_peak_pss_kib']=max(r['pss_kib'] for r in terminal_rows)
        if a.screenshots:
            rpc('lyrics',False); send('5');drain(.5);screenshot(f'{a.style}-now-playing.png')
            rpc('lyrics',True);send('6');drain(1);screenshot('lyrics.png')
            send('7');drain(.3);screenshot('settings.png')
            send('1');drain(.3);screenshot('search.png')
        if a.seconds >= 3600: assert completed >= 4, 'hour session must observe at least four complete songs'
        result['success']=True
        print(json.dumps({k:v for k,v in result.items() if k!='samples'}))
    finally:
        try:
            if tui and tui.poll() is None: send('\x03');drain(.3);tui.wait(timeout=3)
            if terminal and terminal.poll() is None: send('\x03');terminal.wait(timeout=3)
        except (OSError,subprocess.SubprocessError): pass
        if daemon and daemon.poll() is None:
            try: rpc('shutdown');daemon.wait(timeout=5)
            except (OSError,RuntimeError,subprocess.SubprocessError): pass
        for process in [tui,terminal,daemon,bus]:
            if process and process.poll() is None:
                process.terminate()
                try: process.wait(timeout=5)
                except subprocess.TimeoutExpired: process.kill();process.wait()
        if master is not None: os.close(master)
        report.parent.mkdir(parents=True,exist_ok=True); report.write_text(json.dumps(result,indent=2))
