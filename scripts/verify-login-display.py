#!/usr/bin/env python3
"""Optional pyte PTY check: masked CLI editing never grows wrapped prompt rows."""
import codecs,fcntl,os,pty,select,struct,subprocess,sys,tempfile,time,termios
from pathlib import Path
import pyte
binary=str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='rustune-login-display-') as directory:
    env={**os.environ,'TERM':'xterm-256color'}
    for key in ['XDG_CONFIG_HOME','XDG_DATA_HOME','XDG_CACHE_HOME','XDG_RUNTIME_DIR']:
        env[key]=str(Path(directory)/key)
    for width in [80,40,20]:
        master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,width,0,0))
        process=subprocess.Popen([binary,'login'],stdin=slave,stdout=slave,stderr=slave,env=env,start_new_session=True);os.close(slave)
        screen=pyte.Screen(width,24);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder('utf8')('replace')
        def drain(seconds):
            end=time.monotonic()+seconds
            while time.monotonic()<end:
                if select.select([master],[],[],.02)[0]:
                    try:stream.feed(decoder.decode(os.read(master,65536)))
                    except OSError:break
        try:
            drain(.2);row=screen.cursor.y
            for keys,count in [(b'x',1),(b'y',2),(b'\x7f',1),(b'\x15',0)]:
                os.write(master,keys);drain(.08)
                assert screen.display[screen.cursor.y].count('•')==count,f'{width} columns: dot count does not follow typing/backspace/clear'
            os.write(master,b'x'*120);drain(.4)
            text='\n'.join(screen.display)
            assert screen.cursor.y==row,f'{width} columns: masked typing wrapped/scrolled the prompt'
            assert '•••' in text and 'xxx' not in text,f'{width} columns: masked feedback missing or plaintext leaked'
            os.write(master,b'\x7f\x15');drain(.15)
            assert screen.cursor.y==row,f'{width} columns: editing moved feedback row'
            os.write(master,b'\x1b');process.wait(timeout=3)
            assert process.returncode==0
        finally:
            if process.poll() is None:process.kill();process.wait()
            os.close(master)
    print('Masked CLI render/edit/cancel: passed at 80, 40 and 20 columns; no authentication/audio')
