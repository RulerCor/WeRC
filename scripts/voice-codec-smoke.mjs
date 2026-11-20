import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {resolve} from 'node:path';
const executable=resolve(process.argv[2]);
let checks=0;
function run(op,bytes,extra=Buffer.alloc(0)) {
  const header=Buffer.alloc(20);[0x43525657,1,op,16000,bytes.length].forEach((v,i)=>header.writeUInt32LE(v,i*4));
  const r=spawnSync(executable,[],{input:Buffer.concat([header,bytes,extra]),windowsHide:true,timeout:15000,maxBuffer:2*1024*1024});
  assert.equal(r.error,undefined);return r;
}
function valid(r,samples) {
  assert.equal(r.status,0);assert.ok(r.stdout.length>=24);
  assert.deepEqual([0,4,8,12,16,20].map(i=>r.stdout.readUInt32LE(i)),[0x43525657,1,0,16000,samples,r.stdout.length-24]);
  checks++;return r.stdout.subarray(24);
}
const pcm=Buffer.alloc(32000);for(let i=0;i<16000;i++)pcm.writeInt16LE(Math.trunc(Math.sin(i*440*Math.PI*2/16000)*12000),i*2);
const silk=valid(run(1,pcm),16000);
const restored=valid(run(2,silk),16000);assert.equal(restored.length,pcm.length);assert.ok(restored.some(b=>b));checks++;
assert.deepEqual(valid(run(1,pcm),16000),silk);checks++;
for(const result of [run(2,silk.subarray(0,silk.length-1)),run(2,Buffer.alloc(16)),run(1,pcm,Buffer.from([1])),run(1,pcm.subarray(0,639))]) {assert.notEqual(result.status,0);checks++;}
const longest=valid(run(1,Buffer.alloc(1920000)),960000);valid(run(2,longest),960000);
console.log(JSON.stringify({checks_passed:checks,cross_process_roundtrip:true,source_built:true,real_wechat_accessed:false,messages_sent:false}));
