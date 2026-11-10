#!/usr/bin/env python3
"""Opt-in real audio measurement, isolated credentials/data and all owned PIDs."""
import argparse,codecs,fcntl,hashlib,json,os,pty,select,shutil,socket,struct,subprocess,tempfile,termios,time
from pathlib import Path
import pyte
p=argparse.ArgumentParser();p.add_argument('--live',action='store_true',required=True);p.add_argument('--seconds',type=int,default=30);p.add_argument('binary');p.add_argument('report');args=p.parse_args()
assert 10<=args.seconds<=120
binary=str(Path(args.binary).resolve());report=Path(args.report).resolve()
result={'binary_sha256':hashlib.sha256(Path(binary).read_bytes()).hexdigest(),'requested_seconds':args.seconds,'profile':'balanced','equalizer':'off','browser_processes':0,'samples':[],'complete_song_verified':False}
with tempfile.TemporaryDirectory(prefix='rustune-ram-') as folder:
 root=Path(folder);env=dict(os.environ,TERM='xterm-256color',COLORTERM='truecolor')
 env['PIPEWIRE_RUNTIME_DIR']=os.environ.get('PIPEWIRE_RUNTIME_DIR',os.environ.get('XDG_RUNTIME_DIR',f'/run/user/{os.getuid()}'))
 for key,name in [('XDG_CONFIG_HOME','config'),('XDG_DATA_HOME','data'),('XDG_CACHE_HOME','cache'),('XDG_RUNTIME_DIR','run')]:
  env[key]=str(root/name);(root/name/'rustune').mkdir(parents=True,mode=0o700)
 config=root/'config/rustune';cache=root/'cache/rustune';runtime=root/'run/rustune/service.sock'
 token=config/'music-user-token';shutil.copyfile(Path.home()/'.config/rustune/music-user-token',token);token.chmod(0o600)
 (cache/'drm').mkdir(mode=0o700);shutil.copyfile(Path.home()/'.cache/rustune/drm/libwidevinecdm.so',cache/'drm/libwidevinecdm.so');(cache/'drm/libwidevinecdm.so').chmod(0o600)
 shutil.copyfile(Path.home()/'.cache/rustune/apple-app-token',cache/'apple-app-token');(cache/'apple-app-token').chmod(0o600)
 (config/'config.toml').write_text('volume=0.15\ndaemon=true\nmouse=false\nperformance="balanced"\n')
 track={'id':'1440838314','kind':'songs','title':'Simple Man','artist':'Lynyrd Skynyrd','album':'Pronounced Leh-Nerd Skin-Nerd (Bonus Tracks)','duration_ms':359680,'artwork':'https://is1-ssl.mzstatic.com/image/thumb/Music115/v4/1e/5c/85/1e5c85bb-2b43-3057-da09-39446321a191/06UMGIM14960.rgb.jpg/{w}x{h}bb.jpg'}
 state=root/'data/rustune/state.json';state.write_text(json.dumps({'queue':{'tracks':[track],'current':0,'repeat':'off','shuffle':False,'revision':1}}));state.chmod(0o600)
 daemon=tui=bus=None;master=None
 def rpc(command):
  with socket.socket(socket.AF_UNIX) as s:
   s.settimeout(5);s.connect(str(runtime));body=json.dumps({'command':command}).encode();s.sendall(struct.pack('>I',len(body))+body)
   def read(n):
    b=b''
    while len(b)<n:
     chunk=s.recv(n-len(b))
     if not chunk:raise RuntimeError('closed measurement socket')
     b+=chunk
    return b
   n=struct.unpack('>I',read(4))[0];assert n<=4*1024*1024;reply=json.loads(read(n));assert reply['ok'],reply.get('error');return reply['data']
 screen=pyte.Screen(142,39);stream=pyte.Stream(screen);decoder=codecs.getincrementaldecoder('utf8')('replace')
 def drain(seconds):
  end=time.monotonic()+seconds
  while master is not None and time.monotonic()<end:
   if select.select([master],[],[],min(.05,max(0,end-time.monotonic())))[0]:
    try:chunk=os.read(master,65536)
    except OSError:break
    stream.feed(decoder.decode(chunk))
 try:
  bus=subprocess.Popen(['dbus-daemon','--session','--nofork','--print-address'],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True);env['DBUS_SESSION_BUS_ADDRESS']=bus.stdout.readline().strip()
  daemon=subprocess.Popen([binary,'daemon'],env=env,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
  deadline=time.monotonic()+5
  while not runtime.exists() and time.monotonic()<deadline:time.sleep(.02)
  assert runtime.exists(),'measurement daemon unavailable'
  master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',39,142,0,0))
  tui=subprocess.Popen([binary],env=env,stdin=slave,stdout=slave,stderr=slave,start_new_session=True);os.close(slave);drain(.3);rpc('play')
  deadline=time.monotonic()+30
  while time.monotonic()<deadline:
   s=rpc('status')
   if s['playback']['playing'] and s['playback']['encrypted_packets']>0:break
   if s['audio_pid'] is None and s['playback']['state']==0:raise AssertionError(s['message'])
   drain(.1)
  else:raise AssertionError('measurement could not start genuine native playback')
  # Same warm-up, cover, viewport, song and profile for before/after runs.
  drain(3);assert any('▀' in line for line in screen.display),'real cover missing in measurement'
  started=time.monotonic()
  while time.monotonic()-started<args.seconds:
   s=rpc('status');assert s['playback']['playing'];rows=[]
   for role,pid in [('service',daemon.pid),('tui',tui.pid),('audio',s['audio_pid'])]:
    fields={line.split(':')[0]:int(line.split()[1]) for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines() if line.startswith(('Rss:','Pss:'))}
    stat=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split();rows.append({'role':role,'pid':pid,'rss_kib':fields['Rss'],'pss_kib':fields['Pss'],'cpu_ticks':int(stat[11])+int(stat[12])})
   result['samples'].append({'seconds':time.monotonic()-started,'processes':rows,'rss_kib':sum(x['rss_kib'] for x in rows),'pss_kib':sum(x['pss_kib'] for x in rows)});drain(1)
  a,b=result['samples'][0],result['samples'][-1];elapsed=b['seconds']-a['seconds'];ticks=sum(x['cpu_ticks'] for x in b['processes'])-sum(x['cpu_ticks'] for x in a['processes'])
  result.update(peak_rss_kib=max(x['rss_kib'] for x in result['samples']),peak_pss_kib=max(x['pss_kib'] for x in result['samples']),mean_pss_kib=sum(x['pss_kib'] for x in result['samples'])/len(result['samples']),cpu_percent_one_core=ticks/os.sysconf('SC_CLK_TCK')/elapsed*100,real_encrypted_playback_verified=True)
  print(json.dumps({k:result[k] for k in ['peak_rss_kib','peak_pss_kib','mean_pss_kib','cpu_percent_one_core','real_encrypted_playback_verified']}))
 finally:
  if tui is not None and tui.poll() is None:
   try:os.write(master,b'\x03');tui.wait(timeout=3)
   except (OSError,subprocess.TimeoutExpired):tui.terminate();tui.wait(timeout=3)
  if daemon is not None and daemon.poll() is None:
   try:rpc('stop');daemon.wait(timeout=3)
   except (OSError,RuntimeError,subprocess.TimeoutExpired):daemon.terminate();daemon.wait(timeout=3)
  if bus is not None and bus.poll() is None:bus.terminate();bus.wait(timeout=3)
  if master is not None:os.close(master)
  report.write_text(json.dumps(result,indent=2))
