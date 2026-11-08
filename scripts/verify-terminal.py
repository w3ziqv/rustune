#!/usr/bin/env python3
"""Real PTY keyboard/resize smoke test. --live-controls pauses/resumes current audio."""
import argparse,fcntl,json,os,pty,select,struct,subprocess,termios,time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('binary');p.add_argument('--live-controls',action='store_true');a=p.parse_args()
binary=str(Path(a.binary).resolve())
def status(): return json.loads(subprocess.check_output([binary,'status']))
master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',30,100,0,0))
process=subprocess.Popen([binary],stdin=slave,stdout=slave,stderr=slave,env={**os.environ,'TERM':'xterm-256color'},start_new_session=True);os.close(slave)
captured=bytearray()
def drain(seconds):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        if select.select([master],[],[],min(.1,max(0,end-time.monotonic())))[0]:
            try:captured.extend(os.read(master,65536))
            except OSError:break
try:
    drain(.5);assert b'RUSTUNE' in captured,'real terminal did not render header'
    os.write(master,b'?');drain(.2);assert b'HELP' in captured,'Help key did not render'
    for height,width in [(10,40),(4,20),(48,160),(30,100)]:
        fcntl.ioctl(master,termios.TIOCSWINSZ,struct.pack('HHHH',height,width,0,0));process.send_signal(__import__('signal').SIGWINCH);drain(.15)
    if a.live_controls:
        before=status();assert before['playback']['playing'],'need real active playback'
        os.write(master,b' ');drain(.7);assert not status()['playback']['playing'],'Space did not pause actual browser playback'
        os.write(master,b' ');drain(.7);assert status()['playback']['playing'],'Space did not resume actual browser playback'
    os.write(master,b'q');drain(.2);process.wait(timeout=5);assert process.returncode==0
    print(json.dumps({'terminal':'real Linux PTY','header':True,'help':True,'resizes':4,'quit_exit':0,'live_pause_resume':a.live_controls,'bytes_written':len(captured)}))
finally:
    if process.poll() is None:process.terminate();process.wait(timeout=5)
    os.close(master)
