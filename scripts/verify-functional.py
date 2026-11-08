#!/usr/bin/env python3
"""Optional online PTY smoke: isolated XDG/D-Bus, artificial token, public lyrics/art.

Usage: python scripts/verify-functional.py /absolute/path/to/rustune
Requires pyte, dbus-daemon and busctl. Does not authenticate Apple or play audio.
Only check names are printed; lyric bodies and terminal captures are not retained.
"""
import termios,codecs,fcntl,hashlib,json,os,pty,select,signal,socket,struct,subprocess,sys,tempfile,time
from pathlib import Path
try:
    import pyte
except ImportError:
    sys.exit('This optional online smoke test requires pyte (install it in a test-only virtualenv).')

binary=str(Path(sys.argv[1]).resolve())
report={'binary_sha256':hashlib.sha256(Path(binary).read_bytes()).hexdigest(),'checks':[],'native_playback':'not implemented','actual_audio_verified':False}
with tempfile.TemporaryDirectory(prefix='rustune-functional-') as directory:
    root=Path(directory);env={**os.environ,'TERM':'xterm-256color','COLORTERM':'truecolor'};env.pop('NO_COLOR',None)
    for key,name in [('XDG_CONFIG_HOME','config'),('XDG_DATA_HOME','data'),('XDG_CACHE_HOME','cache'),('XDG_RUNTIME_DIR','run')]:
        env[key]=str(root/name);(root/name).mkdir(mode=0o700)
    bus=subprocess.Popen(['dbus-daemon','--session','--nofork','--print-address'],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True)
    daemon=tui=None;master=None
    def check(value,label):
        if not value:raise AssertionError(label)
        report['checks'].append(label)
    try:
        env['DBUS_SESSION_BUS_ADDRESS']=bus.stdout.readline().strip()
        subprocess.run([binary,'init'],env=env,stdout=subprocess.DEVNULL,check=True)
        config=Path(env['XDG_CONFIG_HOME'])/'rustune/config.toml'
        data=Path(env['XDG_DATA_HOME'])/'rustune';runtime=Path(env['XDG_RUNTIME_DIR'])/'rustune/service.sock'
        first={'id':'1445734224','kind':'songs','title':'Simple Man','artist':'Lynyrd Skynyrd','album':'Pronounced Leh-Nerd Skin-Nerd (Bonus Tracks)','duration_ms':357933,'artwork':'https://is1-ssl.mzstatic.com/image/thumb/Music115/v4/1e/5c/85/1e5c85bb-2b43-3057-da09-39446321a191/06UMGIM14960.rgb.jpg/{w}x{h}bb.jpg'}
        tracks=[first,{'id':'fixture-middle','kind':'songs','title':'Fixture Middle','artist':'Test artist','duration_ms':100000},{'id':'fixture-last','kind':'songs','title':'Fixture Last','artist':'Test artist','duration_ms':100000}]
        state=data/'state.json';state.write_text(json.dumps({'queue':{'tracks':tracks,'current':0,'repeat':'off','shuffle':False},'history':[],'dismissed':None}));state.chmod(0o600)
        token=Path(env['XDG_CONFIG_HOME'])/'rustune/music-user-token';token.write_text('artificial-token-for-isolated-functional-test');token.chmod(0o600)
        def start_daemon():
            process=subprocess.Popen([binary,'daemon'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            end=time.monotonic()+5
            while not runtime.exists() and time.monotonic()<end:
                if process.poll() is not None:raise AssertionError('isolated service exited')
                time.sleep(.02)
            return process
        daemon=start_daemon()
        def rpc(command,args=None):
            with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as s:
                s.settimeout(5);s.connect(str(runtime));value={'command':command}
                if args is not None:value['args']=args
                raw=json.dumps(value).encode();s.sendall(struct.pack('>I',len(raw))+raw)
                def read(n):
                    result=b''
                    while len(result)<n:
                        part=s.recv(n-len(result))
                        if not part:raise AssertionError('service closed response')
                        result+=part
                    return result
                size=struct.unpack('>I',read(4))[0];assert size<=4*1024*1024
                return json.loads(read(size))
        status=lambda:rpc('status')['data']
        check(status()['queue_len']==3,'restored queue')
        check(status()['browser_pid'] is None and not status()['playback_available'],'token does not falsely enable native playback or browser fallback')
        for command,args in [('play',None),('pause',None),('next',None),('previous',None),('volume',.2),('mute',None),('seek',10),('sleep',60),('browse',{'kind':'library','term':'','next':None})]:
            result=rpc(command,args);check(not result['ok'] and 'Native Linux playback' in result['error'],f'{command}: unsupported playback reported explicitly')
        check(status()['current']==0 and status()['queue_len']==3,'failed playback controls preserve queue')
        prop=subprocess.check_output(['busctl',f'--address={env["DBUS_SESSION_BUS_ADDRESS"]}','get-property','org.mpris.MediaPlayer2.rustune','/org/mpris/MediaPlayer2','org.mpris.MediaPlayer2.Player','CanControl'],env=env)
        check(prop.strip()==b'b false','MPRIS reports unavailable playback honestly')
        master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',39,142,0,0))
        tui=subprocess.Popen([binary],stdin=slave,stdout=slave,stderr=slave,env=env,start_new_session=True);os.close(slave)
        screen=pyte.Screen(142,39);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder('utf8')('replace')
        def drain(seconds=.1):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                if select.select([master],[],[],min(.05,max(0,end-time.monotonic())))[0]:
                    try:chunk=os.read(master,65536)
                    except OSError:break
                    stream.feed(decoder.decode(chunk))
        text=lambda:'\n'.join(screen.display)
        def wait(predicate,label,seconds=8):
            end=time.monotonic()+seconds
            while not predicate() and time.monotonic()<end:
                if tui.poll() is not None:raise AssertionError('TUI exited before '+label)
                drain(.05)
            check(predicate(),label)
        def send(keys):os.write(master,keys);drain(.08)
        def command(value):send(b':'+value.encode()+b'\r')
        wait(lambda:'RUSTUNE' in text(),'tested binary renders main player')
        check(screen.buffer[0][1].bg=='000000' and screen.buffer[0][1].fg=='ffffff','actual terminal black background and white text')
        for x in [9,30,50,70,91,111]:
            command('not-a-real-command');wait(lambda:'unknown command' in text(),'clear previous audio error before click '+str(x))
            send(f'\x1b[<0;{x};36M'.encode())
            wait(lambda:'Native Linux playback' in text(),f'mouse control at column {x} reports unsupported native audio')
        send(b'8');wait(lambda:'Fixture Last' in text(),'queue navigation loads persisted items')
        send(b'J');wait(lambda:rpc('queue_page',{'offset':0})['data']['items'][1]['id']==first['id'],'keyboard queue move down')
        send(b'K');wait(lambda:rpc('queue_page',{'offset':0})['data']['items'][0]['id']==first['id'],'keyboard queue move up')
        send(b'l');wait(lambda:'LYRICS PREVIEW' in text(),'selected queue song opens a silent lyrics preview')
        wait(lambda:'LRCLIB' in text() and 'synced' in text(),'real synchronized lyrics in tested TUI',15)
        wait(lambda:any('▀' in line[:60] for line in screen.display),'real Apple cover in tested TUI',10)
        before=text();send(b'jjjj');check(text()!=before,'lyrics keyboard scrolling changes visible rows')
        command('profile minimal');wait(lambda:status()['profile']=='minimal','minimal profile is persisted')
        wait(lambda:not any('▀' in line for line in screen.display),'minimal profile actually removes artwork')
        command('profile balanced');wait(lambda:status()['profile']=='balanced','balanced profile is restored')
        wait(lambda:any('▀' in line[:60] for line in screen.display),'artwork returns after leaving minimal profile',10)
        command('theme tokyo_night');wait(lambda:'tokyo_night' in config.read_text(),'optional Tokyo Night persists')
        wait(lambda:screen.buffer[0][1].fg!='ffffff','theme updates actual terminal colors')
        command('theme monochrome');wait(lambda:screen.buffer[0][1].fg=='ffffff' and screen.buffer[0][1].bg=='000000','monochrome returns live')
        send(b'L');wait(lambda:'Paste token' in text(),'L opens hidden token modal')
        send(b'\x1b[200~do-not-save-cancelled-test-token\x1b[201~');check('do-not-save' not in text(),'bracketed token paste is absent from screen')
        send(b'\x1b');wait(lambda:'Paste token' not in text(),'Esc cancels token modal')
        check(token.read_text()=='artificial-token-for-isolated-functional-test','cancel preserves existing local token')
        command('logout');wait(lambda:not token.exists(),'TUI logout removes only isolated local token')
        for w,h in [(40,10),(20,4),(160,48),(142,39)]:
            screen.resize(lines=h,columns=w);fcntl.ioctl(master,termios.TIOCSWINSZ,struct.pack('HHHH',h,w,0,0));tui.send_signal(signal.SIGWINCH);drain(.15);check(tui.poll() is None,f'live resize {w}x{h}')
        send(b'8');wait(lambda:'Fixture Last' in text(),'queue remains usable after resizes')
        added={'id':'external-test','kind':'songs','title':'External Test Song','artist':'Test artist','duration_ms':100000}
        check(rpc('enqueue',{'track':added,'next':False,'play':False})['ok'],'external CLI-style queue addition succeeds')
        wait(lambda:'External Test Song' in text(),'open TUI updates after external queue addition',3)
        check(rpc('move',{'from':3,'to':1})['ok'],'external same-length queue reorder succeeds')
        wait(lambda:'External Test Song' in text() and text().index('External Test Song')<text().index('Fixture Last'),'open TUI updates after external same-length reorder',3)
        for i in range(27):
            track={'id':f'page-fixture-{i}','kind':'songs','title':f'Queue Page Fixture {i:02}','artist':'Test artist'}
            assert rpc('enqueue',{'track':track,'next':False,'play':False})['ok']
        wait(lambda:status()['queue_len']==31,'external additions populate multiple queue pages')
        wait(lambda:'Queue Page Fixture 20' in text(),'first queue page is full before PgDn')
        send(b'\x1b[6~');wait(lambda:'Queue Page Fixture 21' in text(),'PgDn opens second queue page')
        for i in range(30,2,-1):
            assert rpc('remove',i)['ok']
        wait(lambda:'Simple Man' in text() and 'Queue Page Fixture 21' not in text(),'external shrink returns TUI to a valid queue page',3)
        send(b'l');wait(lambda:'LYRICS PREVIEW' in text(),'return to player before clicking Quit')
        send(b'\x1b[<0;131;36M');tui.wait(timeout=5);check(tui.returncode==0,'Quit mouse button exits cleanly')
        check(daemon.poll() is None,'attached TUI leaves preexisting service alive')
        rpc('stop');daemon.wait(timeout=5);daemon=start_daemon();check(status()['queue_len']==3,'queue survives service restart')
        check(rpc('clear')['ok'],'queue clear succeeds')
        rpc('stop');daemon.wait(timeout=5);daemon=start_daemon();check(status()['queue_len']==0,'cleared queue survives service restart')
        check(status()['browser_pid'] is None,'no browser was started during functional tests')
    except Exception as e:
        report['failure']=str(e);report['success']=False
    else:report['success']=True
    finally:
        for p in [tui,daemon,bus]:
            if p is not None and p.poll() is None:
                p.terminate()
                try:p.wait(timeout=5)
                except subprocess.TimeoutExpired:p.kill();p.wait()
        if master is not None:os.close(master)
        print(json.dumps(report,indent=2))
sys.exit(0 if report['success'] else 1)
