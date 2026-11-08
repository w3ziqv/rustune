// Adapter contract with a controlled SDK fixture; no real authorization/audio.
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const source=readFileSync(new URL('../src/backend/musickit.js',import.meta.url),'utf8').replace('__RUSTUNE_INSTANCE__','sdk');
const sdk={bitrate:256,isAuthorized:true,addEventListener(){},removeEventListener(){},playbackState:2};
const context={sdk,window:{},MusicKit:{PlaybackStates:{playing:2,ended:8,completed:10}}};
await vm.runInNewContext(source,context);
assert.equal(context.window.rustune.snapshot().target_bitrate_kbps,256);
sdk.bitrate=12345;
assert.equal(context.window.rustune.snapshot().target_bitrate_kbps,null);
console.log('MusicKit quality contract passed (no live playback)');
