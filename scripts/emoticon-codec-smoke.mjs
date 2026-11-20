import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
const exe=process.argv[2];assert.ok(exe);
const fixture=readFileSync(fileURLToPath(new URL('../tests/fixtures/owned-received-emoticon.wxgf',import.meta.url)));
const meta=JSON.parse(readFileSync(fileURLToPath(new URL('../tests/fixtures/owned-received-emoticon.json',import.meta.url))));
assert.equal(createHash('sha256').update(fixture).digest('hex'),meta.sha256);
const word=n=>{const b=Buffer.alloc(4);b.writeUInt32LE(n);return b;};
const parts=[word(0x45525657),word(1),word(16),word(16),word(3)];let at=fixture[4];
for(let i=0;i<3;i++){at+=2;for(let j=0;j<2;j++){const n=fixture.readUInt32BE(at);at+=4;parts.push(word(n),fixture.subarray(at,at+n));at+=n;}}
assert.equal(at,fixture.length-24);const input=Buffer.concat(parts);
const run=bytes=>spawnSync(exe,[],{input:bytes,encoding:null,maxBuffer:4*1024*1024,timeout:10000,windowsHide:true});
const result=run(input);assert.equal(result.status,0);const out=result.stdout;assert.equal(out.length,20+3*16*16*4+8);assert.deepEqual(out.subarray(0,20),input.subarray(0,20));assert.equal(out.readUInt32LE(out.length-8),0x45525657);assert.equal(out.readUInt32LE(out.length-4),3);
let checks=1;
for(let f=0;f<3;f++)for(let y=0;y<16;y++)for(let x=0;x<16;x++){
 const p=out.subarray(20+((f*16+y)*16+x)*4,24+((f*16+y)*16+x)*4);
 assert.equal(p[3],x<8?255:0);if(x>=8)assert.deepEqual([...p],[0,0,0,0]);else for(let c=0;c<3;c++)assert.ok(Math.abs(p[c]-(c===f?255:0))<=3,`${f}/${c}: ${p[c]}`);
}checks++;
for(const [offset,value] of [[0,0],[4,2],[8,0],[8,1025],[12,0],[16,0],[16,121],[20,0],[20,1048577],[24,1],[25,1],[26,2]]){
 const b=Buffer.from(input);if(offset<24)b.writeUInt32LE(value,offset);else b[offset]=value;assert.notEqual(run(b).status,0);checks++;
}
for(const n of [0,4,19,23,24,43,input.length-1]){assert.notEqual(run(input.subarray(0,n)).status,0);checks++;}
assert.notEqual(run(Buffer.concat([input,Buffer.from([1])])).status,0);checks++;
console.log(JSON.stringify({passed:true,checks,decoded_frames:3,rgba_and_alpha_verified:true,source_fixture_sha256:meta.sha256,codec_sha256:createHash('sha256').update(readFileSync(exe)).digest('hex'),real_wechat_accessed:false,messages_sent:false}));
