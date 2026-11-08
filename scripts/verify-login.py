#!/usr/bin/env python3
"""Real PTY login contract using an artificial token and isolated XDG paths."""
import fcntl,os,pty,re,select,struct,subprocess,sys,tempfile,time,termios
from pathlib import Path
binary=str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='rustune-login-') as d:
    env={**os.environ,**{k:str(Path(d)/k) for k in ['XDG_CONFIG_HOME','XDG_DATA_HOME','XDG_CACHE_HOME','XDG_RUNTIME_DIR']},'TERM':'xterm-256color'}
    token=b'artificial-rustune-token-for-pty-test'
    def run(data):
        master,slave=pty.openpty();process=subprocess.Popen([binary,'login'],stdin=slave,stdout=slave,stderr=slave,env=env,start_new_session=True);os.close(slave)
        captured=bytearray()
        def drain():
            if select.select([master],[],[],.05)[0]:
                try:captured.extend(os.read(master,65536))
                except OSError:pass
        try:
            end=time.monotonic()+5
            while b'Music User Token' not in captured and time.monotonic()<end and process.poll() is None:drain()
            assert b'Music User Token' in captured,'login did not ask for a token'
            import termios
            assert not (termios.tcgetattr(master)[3]&termios.ECHO),'hidden prompt appeared before terminal echo was disabled'
            os.write(master,data)
            while process.poll() is None and time.monotonic()<end:drain()
            process.wait(timeout=2);drain()
            assert process.returncode==0,'login exited unsuccessfully'
            assert token not in captured,'token was echoed to terminal output'
            assert b'not verified' in captured or b'cancelled' in captured,'missing honest login outcome'
        finally:
            if process.poll() is None:process.kill();process.wait()
            os.close(master)
    run(token+b'\r')
    file=Path(env['XDG_CONFIG_HOME'])/'rustune/music-user-token'
    assert file.read_bytes()==token
    assert file.stat().st_mode&0o777==0o600
    run(b'\x1b')
    assert file.read_bytes()==token,'cancel replaced existing token'
    assert not list(Path(d).rglob('service.pid')),'token login started a playback service'
    config=file.with_name('config.toml')
    config.write_text('daemon = false\n')
    config.chmod(0o600)
    runtime=Path(env['XDG_RUNTIME_DIR'])/'rustune'
    master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',30,100,0,0));process=subprocess.Popen([binary],stdin=slave,stdout=slave,stderr=slave,env=env,start_new_session=True);os.close(slave)
    captured=bytearray()
    def drain_tui(seconds):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            if select.select([master],[],[],.05)[0]:
                try:captured.extend(os.read(master,65536))
                except OSError:break
    try:
        drain_tui(.4);os.write(master,b':login\r');drain_tui(.2)
        assert b'Paste' in captured and b'(hidden)' in captured,'TUI login did not open a token prompt'
        replacement=b'artificial-replacement-from-tui'
        os.write(master,replacement+b'\r')
        deadline=time.monotonic()+5
        while file.read_bytes()!=replacement and time.monotonic()<deadline:drain_tui(.05)
        assert file.read_bytes()==replacement,'TUI token was not saved'
        assert replacement not in captured,'TUI exposed the token'
        assert replacement not in re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]',b'',captured),'TUI exposed the token across terminal escape sequences'
        os.write(master,b'q');drain_tui(.2);process.wait(timeout=5);assert process.returncode==0
        assert not (runtime/'service.pid').exists(),'TUI left its test service running'
    finally:
        if process.poll() is None:process.kill();process.wait()
        os.close(master)
        if (runtime/'service.sock').exists():
            subprocess.run([binary,'ctl','stop'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=6,check=False)
            deadline=time.monotonic()+5
            while (runtime/'service.pid').exists() and time.monotonic()<deadline:time.sleep(.02)
            assert not (runtime/'service.pid').exists(),'test service did not stop during cleanup'
    print('Hidden token login and cancellation: passed; no account or playback verification')
