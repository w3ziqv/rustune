// Contract test only: mocked extension APIs; no account/audio/playback claims.
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
let receiver, clicked, tabs=[{id:1,url:'https://music.apple.com/us/browse',audible:true},{id:2,url:'https://music.apple.com/us/browse',active:true}];
const targets=[],replies=[];
const port={onDisconnect:{addListener(){}},onMessage:{addListener(fn){receiver=fn;}},postMessage(v){replies.push(v);}};
const browser={permissions:{contains:async()=>true},runtime:{connectNative(name){assert.equal(name,'org.rustune.bridge');return port;}},action:{onClicked:{addListener(fn){clicked=fn;}}},tabs:{query:async()=>tabs,update:async()=>{}},windows:{update:async()=>{}},scripting:{executeScript:async options=>{assert.equal(options.world,'MAIN');targets.push(options.target.tabId);return [{result:{authorized:false}}];}}};
vm.runInNewContext(readFileSync(new URL('../extensions/firefox/background.js',import.meta.url),'utf8'),{browser});
await receiver({id:1,operation:'snapshot'});
tabs[0].audible=false;
await receiver({id:2,operation:'snapshot'});
assert.deepEqual(targets,[1,1,1,1],'pause/audibility changes must not redirect another tab');
tabs=tabs.slice(1);await receiver({id:3,operation:'snapshot'});assert.equal(replies.at(-1).ok,false,'closed pinned tab requires deliberate reconnect');
clicked();await receiver({id:4,operation:'snapshot'});assert.equal(replies.at(-1).ok,true);assert.equal(targets.at(-1),2);
await receiver({id:5,operation:'arbitrary-eval'});assert.equal(replies.at(-1).ok,false);
console.log('Firefox extension contract: passed (no live browser or playback)');
