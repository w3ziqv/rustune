#!/usr/bin/env python3
"""Explicit opt-in live subscribed playback; private copied credential, isolated data.
Run: PYTHONPATH=/tmp/rustune-e2e-deps python scripts/verify-native-live.py --live BINARY REPORT
Never prints/records credentials, signed media URLs, challenges, licenses or lyrics.
"""
import argparse,codecs,fcntl,hashlib,json,os,pty,select,shutil,socket,struct,subprocess,sys,tempfile,termios,time
from pathlib import Path
import pyte

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--live',action='store_true',required=True)
parser.add_argument('--controls-only',action='store_true',help='Verify actual native controls without claiming complete-song playback')
parser.add_argument('binary');parser.add_argument('report')
args=parser.parse_args();binary=str(Path(args.binary).resolve());report_file=Path(args.report).resolve()
original_token=Path.home()/'.config/rustune/music-user-token'
original_cdm=Path.home()/'.cache/rustune/drm/libwidevinecdm.so'
report={'binary_sha256':hashlib.sha256(Path(binary).read_bytes()).hexdigest(),'checks':[],'live_subscribed_playback':False,'owned_browser_processes':0,'measurements':[]}
def check(value,label):
    if not value:raise AssertionError(label)
    report['checks'].append(label)
def save():report_file.write_text(json.dumps(report,indent=2))
with tempfile.TemporaryDirectory(prefix='rustune-native-live-') as directory:
    root=Path(directory);env={**os.environ,'TERM':'xterm-256color','COLORTERM':'truecolor'}
    env['PIPEWIRE_RUNTIME_DIR']=os.environ.get('PIPEWIRE_RUNTIME_DIR',os.environ.get('XDG_RUNTIME_DIR',f'/run/user/{os.getuid()}'))
    for key,name in [('XDG_CONFIG_HOME','config'),('XDG_CACHE_HOME','cache'),('XDG_DATA_HOME','data'),('XDG_RUNTIME_DIR','run')]:
        env[key]=str(root/name);(root/name/'rustune').mkdir(parents=True,mode=0o700)
    config=root/'config/rustune';cache=root/'cache/rustune';data=root/'data/rustune';runtime=root/'run/rustune/service.sock'
    token=config/'music-user-token'
    def restore_token():shutil.copyfile(original_token,token);token.chmod(0o600)
    restore_token();(cache/'drm').mkdir(mode=0o700);shutil.copyfile(original_cdm,cache/'drm/libwidevinecdm.so');(cache/'drm/libwidevinecdm.so').chmod(0o600)
    (config/'config.toml').write_text('volume = 0.35\ndaemon = true\nmouse = false\n')
    track={'id':'1440838314','kind':'songs','title':'Simple Man','artist':'Lynyrd Skynyrd','album':'Pronounced Leh-Nerd Skin-Nerd (Bonus Tracks)','duration_ms':359680,'artwork':'https://is1-ssl.mzstatic.com/image/thumb/Music115/v4/1e/5c/85/1e5c85bb-2b43-3057-da09-39446321a191/06UMGIM14960.rgb.jpg/{w}x{h}bb.jpg'}
    state=data/'state.json';state.write_text(json.dumps({'queue':{'tracks':[track,track],'current':0,'repeat':'off','shuffle':False,'revision':2}}));state.chmod(0o600)
    daemon=tui=bus=None;master=None
    def rpc(command,value=None):
        with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as s:
            s.settimeout(5);s.connect(str(runtime));payload={'command':command}
            if value is not None:payload['args']=value
            raw=json.dumps(payload).encode();s.sendall(struct.pack('>I',len(raw))+raw)
            def read(n):
                result=b''
                while len(result)<n:
                    part=s.recv(n-len(result))
                    if not part:raise AssertionError('service response closed')
                    result+=part
                return result
            size=struct.unpack('>I',read(4))[0];assert 0<size<=4*1024*1024
            result=json.loads(read(size));assert result['ok'],result.get('error','service rejected command');return result['data']
    status=lambda:rpc('status')
    screen=pyte.Screen(142,39);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder('utf8')('replace')
    def drain(seconds=.1):
        end=time.monotonic()+seconds
        while master is not None and time.monotonic()<end:
            if select.select([master],[],[],min(.05,max(0,end-time.monotonic())))[0]:
                try:chunk=os.read(master,65536)
                except OSError:break
                stream.feed(decoder.decode(chunk))
    text=lambda:'\n'.join(screen.display)
    def wait(predicate,label,seconds=30):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            if predicate():check(True,label);return
            drain(.05)
        check(False,label)
    def send(keys):os.write(master,keys);drain(.15)
    def active():
        s=status()
        if s['audio_pid'] is None and s['playback']['state']==0:raise AssertionError(s['message'])
        return s['playback']['playing'] and s['playback']['encrypted_packets']>0
    try:
        bus=subprocess.Popen(['dbus-daemon','--session','--nofork','--print-address'],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True)
        env['DBUS_SESSION_BUS_ADDRESS']=bus.stdout.readline().strip()
        daemon=subprocess.Popen([binary,'daemon'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        end=time.monotonic()+5
        while not runtime.exists() and time.monotonic()<end:time.sleep(.02)
        check(runtime.exists(),'isolated real native service starts')
        master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',39,142,0,0))
        tui=subprocess.Popen([binary],env=env,stdin=slave,stdout=slave,stderr=slave,start_new_session=True);os.close(slave)
        wait(lambda:'RUSTUNE' in text(),'actual release TUI renders')
        send(b'8');wait(lambda:'Simple Man' in text(),'keyboard opens real queue')
        send(b'\r');wait(lambda:'Lyrics and cover' in text(),'keyboard opens song actions')
        send(b'\x1b[B\r');wait(active,'keyboard Play starts native CDM audio')
        check((cache/'apple-app-token').is_file(),'public application authorization fetched anonymously without a prefilled cache')
        check((cache/'apple-app-token').stat().st_mode&0o777==0o600,'public application authorization cache is private')
        check(status()['playback']['authorized'] and not status()['playback']['preview'],'Apple full-stream authorization verified without preview fallback')
        send(b' ');wait(lambda:not status()['playback']['playing'],'keyboard pause reaches native worker')
        position=status()['playback']['position'];drain(1)
        check(abs(status()['playback']['position']-position)<.2,'paused device clock remains stable')
        send(b' ');wait(active,'keyboard resumes native audio')
        send(b'+');wait(lambda:abs(status()['playback']['volume']-.4)<.001,'keyboard raises software volume')
        send(b'm');wait(lambda:status()['playback']['volume']==0,'keyboard mutes native audio')
        send(b'm');wait(lambda:abs(status()['playback']['volume']-.4)<.001,'keyboard restores previous volume')
        rpc('seek',90.0);wait(lambda:status()['playback']['position']>=89.9,'native encrypted stream seeks forward')
        rpc('seek',10.0);wait(lambda:9.9<=status()['playback']['position']<20,'native encrypted stream seeks backward')
        subprocess.run([binary,'logout'],env=env,stdout=subprocess.DEVNULL,check=True)
        rpc('pause');wait(lambda:not status()['playback']['playing'],'active native audio can pause after local logout')
        rpc('volume',.3);check(abs(status()['playback']['volume']-.3)<.001,'active volume remains controllable after logout')
        check(status()['playback_available'],'active transport remains available after logout')
        for name,expected in [('CanControl',True),('CanPause',True),('CanSeek',True),('CanGoNext',False)]:
            value=subprocess.check_output(['busctl',f'--address={env["DBUS_SESSION_BUS_ADDRESS"]}','get-property','org.mpris.MediaPlayer2.rustune','/org/mpris/MediaPlayer2','org.mpris.MediaPlayer2.Player',name],env=env)
            check(value.strip()==(b'b true' if expected else b'b false'),f'MPRIS {name} follows active audio and start prerequisites after logout')
        restore_token();rpc('play');wait(active,'restored local token and active audio resume')
        old_pid=status()['audio_pid'];rpc('next');wait(lambda:status()['current']==1 and active(),'Next starts another native worker')
        check(not Path(f'/proc/{old_pid}').exists(),'replaced native worker is reaped')
        rpc('previous');wait(lambda:status()['current']==0 and active(),'Previous starts the preceding queued song')
        if args.controls_only:
            rpc('clear');duration=status()['playback']['duration'];rpc('seek',duration-.5);rpc('pause')
            wait(lambda:not status()['playback']['playing'],'pause is accepted near the end of the stream')
            drain(.8);check(not status()['playback']['ended'],'near-end pause prevents completion and queue advancement')
            can_play=subprocess.check_output(['busctl',f'--address={env["DBUS_SESSION_BUS_ADDRESS"]}','get-property','org.mpris.MediaPlayer2.rustune','/org/mpris/MediaPlayer2','org.mpris.MediaPlayer2.Player','CanPlay'],env=env)
            check(can_play.strip()==b'b true','MPRIS can resume active audio after queue clear')
            rpc('play');wait(lambda:status()['playback']['ended'] and status()['audio_pid'] is None,'resumed tail reaches verified EOF and worker is reaped')
            for name in ['CanPlay','CanPause','CanSeek']:
                value=subprocess.check_output(['busctl',f'--address={env["DBUS_SESSION_BUS_ADDRESS"]}','get-property','org.mpris.MediaPlayer2.rustune','/org/mpris/MediaPlayer2','org.mpris.MediaPlayer2.Player',name],env=env)
                check(value.strip()==b'b false',f'MPRIS {name} is disabled after EOF with an empty queue')
            report['controls_verified']=True;save();print(json.dumps({'controls_verified':True,'checks':len(report['checks']),'complete_song_verified':False}));sys.exit(0)
        rpc('clear');rpc('enqueue',{'track':track,'next':False,'play':True});wait(active,'complete-song test restarts at the beginning through the real service')
        check(status()['playback']['position']<3,'full-song verification begins at zero without seeking')
        send(b'2')
        wait(lambda:'AAC' in text() and '44.1' in text(),'TUI displays decoder-reported AAC sample rate')
        wait(lambda:any('▀' in line for line in screen.display),'real cover renders during native playback',20)
        full_start=time.monotonic();samples=[];last_measure=0
        while time.monotonic()-full_start<400:
            s=status();p=s['playback'];report['lyrics_loaded_during_playback']='LRCLIB' in text() and 'synced' in text();report['last_playback']=p;report['message']=s['message'];report['elapsed_full_test_seconds']=time.monotonic()-full_start
            if s['audio_pid'] is None and not p['ended']:raise AssertionError(s['message'])
            if time.monotonic()-last_measure>=5:
                measurements=[]
                for pid in [daemon.pid,tui.pid,s['audio_pid']]:
                    if pid is None:continue
                    try:
                        memory={line.split(':')[0]:int(line.split()[1]) for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines() if line.startswith(('Rss:','Pss:'))}
                        fields=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split();memory.update(pid=pid,cpu_ticks=int(fields[11])+int(fields[12]));measurements.append(memory)
                    except OSError:pass
                report['measurements'].append({'seconds':time.monotonic()-full_start,'processes':measurements,'total_rss_kib':sum(m['Rss'] for m in measurements),'total_pss_kib':sum(m['Pss'] for m in measurements)})
                last_measure=time.monotonic()
            save()
            if p['ended']:
                check(abs(p['position']-p['duration'])<.25,'real output reaches the complete manifest duration')
                check(p['position']>350 and p['encrypted_packets']>15000 and p['decoded_audio_frames']>15000 and p['nonzero_output_samples']>1000000,'entire subscribed song decrypted decoded and sent to real audio output')
                check(not p['preview'] and s['browser_pid'] is None,'complete playback uses no preview or owned browser')
                report['live_subscribed_playback']=True;report['source_quality']=p['source_codec'];save();break
            drain(.3)
        else:raise AssertionError('complete native song did not end within test duration')
        check(s['audio_pid'] is None,'completed worker is closed and reaped')
        print(json.dumps({'live_subscribed_playback':report['live_subscribed_playback'],'checks':len(report['checks']),'position':p['position'],'duration':p['duration'],'peak_total_rss_kib':max(m['total_rss_kib'] for m in report['measurements']),'peak_total_pss_kib':max(m['total_pss_kib'] for m in report['measurements'])}))
    finally:
        if tui is not None and tui.poll() is None:
            try:send(b'\x03');tui.wait(timeout=3)
            except (OSError,subprocess.TimeoutExpired):tui.terminate();tui.wait(timeout=3)
        if daemon is not None and daemon.poll() is None:
            try:rpc('stop');daemon.wait(timeout=3)
            except (OSError,AssertionError,subprocess.TimeoutExpired):daemon.terminate();daemon.wait(timeout=3)
        if master is not None:os.close(master)
        if bus is not None and bus.poll() is None:bus.terminate();bus.wait(timeout=3)
        save()
