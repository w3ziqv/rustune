#!/usr/bin/env python3
"""Optional online, keyboard-only PTY smoke; public Apple metadata, no audio/auth."""
import codecs,fcntl,hashlib,json,os,pty,select,socket,struct,subprocess,sys,tempfile,time,termios
from pathlib import Path
import pyte
binary=str(Path(sys.argv[1]).resolve())
report={'binary_sha256':hashlib.sha256(Path(binary).read_bytes()).hexdigest(),'checks':[],'actual_audio_verified':False,'apple_account_verified':False,'mouse_used':False}
names=['Home','Now Playing','Search','Library','Albums','Artists','Playlists','Queue','Lyrics','Equalizer','Audio Settings','Settings','Help']
with tempfile.TemporaryDirectory(prefix='rustune-keyboard-') as directory:
    root=Path(directory);env={**os.environ,'TERM':'xterm-256color','COLORTERM':'truecolor'}
    for key in ['XDG_CONFIG_HOME','XDG_DATA_HOME','XDG_CACHE_HOME','XDG_RUNTIME_DIR']:
        env[key]=str(root/key);(root/key).mkdir(mode=0o700)
    bus=subprocess.Popen(['dbus-daemon','--session','--nofork','--print-address'],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True)
    daemon=tui=None;master=None
    def check(value,label):
        if not value:raise AssertionError(label)
        report['checks'].append(label)
    try:
        env['DBUS_SESSION_BUS_ADDRESS']=bus.stdout.readline().strip()
        subprocess.run([binary,'init'],env=env,stdout=subprocess.DEVNULL,check=True)
        config=Path(env['XDG_CONFIG_HOME'])/'rustune/config.toml';token=config.with_name('music-user-token')
        endpoint=Path(env['XDG_RUNTIME_DIR'])/'rustune/service.sock'
        daemon=subprocess.Popen([binary,'daemon'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        end=time.monotonic()+5
        while not endpoint.exists() and time.monotonic()<end:time.sleep(.02)
        def rpc(command,args=None):
            with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as s:
                s.settimeout(12);s.connect(str(endpoint));request={'command':command}
                if args is not None:request['args']=args
                raw=json.dumps(request).encode();s.sendall(struct.pack('>I',len(raw))+raw)
                def read(n):
                    result=b''
                    while len(result)<n:
                        part=s.recv(n-len(result))
                        if not part:raise AssertionError('IPC response closed')
                        result+=part
                    return result
                size=struct.unpack('>I',read(4))[0];assert size<=4*1024*1024
                return json.loads(read(size))
        status=lambda:rpc('status')['data']
        master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',39,142,0,0))
        tui=subprocess.Popen([binary],stdin=slave,stdout=slave,stderr=slave,env=env,start_new_session=True);os.close(slave)
        screen=pyte.Screen(142,39);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder('utf8')('replace')
        text=lambda:'\n'.join(screen.display)
        def drain(seconds=.08):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                if select.select([master],[],[],.02)[0]:
                    try:stream.feed(decoder.decode(os.read(master,65536)))
                    except OSError:break
        def send(keys):os.write(master,keys);drain()
        def wait(predicate,label,seconds=15):
            end=time.monotonic()+seconds
            while not predicate() and time.monotonic()<end:
                if tui.poll() is not None:raise AssertionError('TUI exited before '+label)
                drain(.05)
            check(predicate(),label)
        def nav():
            for line in screen.display:
                part=line[:20].strip()
                if part.startswith('> '):
                    for i,name in enumerate(names):
                        if part[2:].startswith(name):return i
            return None
        def menu(target):
            for _ in range(3):
                if nav() is not None:break
                send(b'\x1b');drain(.3)
            wait(lambda:nav() is not None,'keyboard opens navigation')
            delta=target-nav();send((b'\x1b[B' if delta>=0 else b'\x1b[A')*abs(delta))
            wait(lambda:nav()==target,'arrows select '+names[target]);send(b'\r')
        wait(lambda:'RUSTUNE' in text(),'player renders')
        check(not token.exists(),'public search starts without saved token')
        send(b'\x1b[C\r');wait(lambda:'Native Linux playback' in text(),'focused player Enter reports unavailable audio')
        query='Lynyrd Skynyrd Simple Man';send(b'/'+query.encode()+b'\r')
        wait(lambda:' results ' in text() and 'Simple Man' in text(),'real public song search from player focus')
        page=rpc('browse',{'kind':'search:songs','term':query,'next':None})['data']
        matches=[i for i,t in enumerate(page['items']) if t['title'].lower()=='simple man' and 'pronounced' in t['album'].lower()]
        index=matches[0] if matches else next(i for i,t in enumerate(page['items']) if 'simple man' in t['title'].lower())
        song=page['items'][index];send(b'\x1b[B'*index+b'\r');wait(lambda:'SONG ACTIONS' in text(),'Enter opens song actions instead of old player control')
        send(b'\x1b[B\r');wait(lambda:status()['queue_len']==1,'arrows add real result to queue without audio')
        check(rpc('queue_page',{'offset':0})['data']['items'][0]['id']==song['id'],'queued identity matches selected public result')
        check(not status()['playback']['authorized'] and not status()['playback']['playing'],'public results do not claim authentication/playback')
        menu(4);album_query='Lynyrd Skynyrd Pronounced';send(album_query.encode()+b'\r')
        wait(lambda:' results ' in text(),'Albums menu performs a real catalog search')
        albums=rpc('browse',{'kind':'search:albums','term':album_query,'next':None})['data']['items'];check(bool(albums),'public album records are returned')
        album=albums[0];send(b'\r');wait(lambda:'ALBUM TRACKS' in text() and ' results ' in text(),'Enter opens album tracks from Apple')
        tracks=rpc('browse',{'kind':'open:albums:'+album['id'],'term':'','next':None})['data']['items'];check(bool(tracks) and all(t['kind']=='songs' for t in tracks),'lookup returns real songs instead of album container')
        send(b'\x1b[D');wait(lambda:'ALBUM TRACKS' not in text() and album_query in text(),'Left returns to previous album search')
        menu(5);artist_query='Lynyrd Skynyrd';send(artist_query.encode()+b'\r')
        wait(lambda:' results ' in text(),'Artists menu performs a real catalog search')
        artists=rpc('browse',{'kind':'search:artists','term':artist_query,'next':None})['data']['items'];check(bool(artists),'public artist records are returned')
        artist_index=next((i for i,t in enumerate(artists) if t['title'].lower()==artist_query.lower()),0);artist=artists[artist_index]
        send(b'\x1b[B'*artist_index+b'\r');wait(lambda:'ARTIST ALBUMS' in text() and ' results ' in text(),'Enter opens artist albums from Apple')
        artist_albums=rpc('browse',{'kind':'open:artists:'+artist['id'],'term':'','next':None})['data']['items'];check(bool(artist_albums),'artist lookup returns public albums')
        menu(7);wait(lambda:'/ Queue' in text() and song['title'] in text(),'arrows open populated Queue')
        send(b'\r\r');wait(lambda:'LYRICS PREVIEW' in text(),'queue actions open silent lyrics preview')
        wait(lambda:'LRCLIB' in text(),'lyrics provider produces an attributed outcome')
        send(b'\x1b[D');wait(lambda:'/ Queue' in text() and song['title'] in text(),'Left restores freshly loaded queue')
        menu(11);wait(lambda:'Enter/→ change' in text(),'settings expose arrow-selected actions')
        send(b'\x1b[B\x1b[B\r');wait(lambda:'tokyo_night' in config.read_text(),'Enter changes theme without command palette')
        send(b'\x1b[B\r');wait(lambda:status()['profile']=='enhanced','Enter changes resource profile')
        send(b'\r');wait(lambda:status()['profile']=='minimal','resource profile cycles to minimal')
        send(b'\x1b[B\r');wait(lambda:'artwork = false' in config.read_text(),'Enter toggles artwork')
        send(b'\x1b[B\r');wait(lambda:'lyrics = false' in config.read_text(),'Enter toggles lyrics')
        send(b'\x1b[B\r');wait(lambda:status()['catalog_country']=='US','Enter changes catalog country')
        menu(3);wait(lambda:'PERSONAL ACCOUNT' in text() and 'not implemented' in text(),'private library states authorization limit separately')
        menu(11);send(b'\r');wait(lambda:'Paste token' in text(),'settings Enter opens token entry')
        artificial=b'artificial-keyboard-token';send(b'\x1b[200~'+artificial+b'\x1b[201~')
        check(artificial.decode() not in text() and '•••' in text(),'keyboard login paste stays masked')
        send(b'\r');wait(lambda:token.exists(),'keyboard login confirms local save')
        check(token.read_bytes()==artificial and token.stat().st_mode&0o777==0o600,'isolated token value and private mode are correct')
        menu(2);send(b'Queen Bohemian Rhapsody\r');wait(lambda:' results ' in text() and 'Bohemian' in text(),'public search works after token save in changed storefront')
        check(not status()['playback']['authorized'] and status()['browser_pid'] is None,'saved token never falsely enables account/browser/audio')
        menu(11);send(b'\x1b[B'*11+b'\r');tui.wait(timeout=5);check(tui.returncode==0,'arrow-selected Quit exits cleanly')
        report['success']=True
    except Exception as error:report['success']=False;report['failure']=str(error)
    finally:
        for process in [tui,daemon,bus]:
            if process is not None and process.poll() is None:
                process.terminate()
                try:process.wait(timeout=5)
                except subprocess.TimeoutExpired:process.kill();process.wait()
        if master is not None:os.close(master)
        print(json.dumps(report,indent=2))
sys.exit(0 if report['success'] else 1)
