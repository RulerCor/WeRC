// Exercise the shipped normal-identity binary with an explicit, fresh profile.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {spawn} from 'node:child_process';
import {createServer} from 'node:net';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join,relative,toNamespacedPath} from 'node:path';
import {setTimeout as sleep} from 'node:timers/promises';
const root=resolve(import.meta.dirname,'..'),mode=process.argv[2];
const version=JSON.parse(await readFile(join(root,'package.json'),'utf8')).version;
assert.ok(['portable','installer'].includes(mode));
const executable=mode==='portable'?join(root,`target/release-stage/WeRC-v${version}/werc-desktop.exe`):process.argv[3];
assert.ok(executable);
const audit=join(root,`target/release-audit/WeRC-v${version}`),profile=join(audit,'owned-profiles',mode+'-'+Date.now());
await mkdir(profile,{recursive:true});
const scratch=join(profile,'临时 转写');await mkdir(scratch,{recursive:true});
async function port(){const s=createServer();await new Promise(ok=>s.listen(0,'127.0.0.1',ok));const p=s.address().port;await new Promise(ok=>s.close(ok));return p;}
async function until(action,label){for(let i=0;i<300;i++){const value=await action();if(value)return value;await sleep(100);}throw Error('Timed out: '+label);}
class Cdp{
 constructor(socket){this.socket=socket;this.next=0;this.pending=new Map();socket.addEventListener('message',e=>{const m=JSON.parse(e.data),p=this.pending.get(m.id);if(!p)return;this.pending.delete(m.id);clearTimeout(p.timer);m.error?p.no(Error(m.error.message)):p.ok(m.result);});}
 call(method,params={}){return new Promise((ok,no)=>{const id=++this.next,timer=setTimeout(()=>{this.pending.delete(id);no(Error(method+' timed out'));},120000);this.pending.set(id,{ok,no,timer});this.socket.send(JSON.stringify({id,method,params}));});}
 async evaluate(expression){const r=await this.call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});assert.ok(!r.exceptionDetails,r.exceptionDetails?.exception?.description);return r.result.value;}
 async invoke(name,args={}){const r=await this.evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(name)},${JSON.stringify(args)}).then(value=>({ok:true,value}),error=>({ok:false,error}))`);assert.ok(r.ok,JSON.stringify({name,error:r.error}));return r.value;}
 async close(){for(const p of this.pending.values()){clearTimeout(p.timer);p.no(Error('Closed'));}this.pending.clear();this.socket.close();}
}
let child,cdp;
const checks=[];
async function launch(){
 const debuggingPort=await port();
 const env={...process.env,TEMP:scratch,TMP:scratch,
  WEBVIEW2_USER_DATA_FOLDER:join(profile,'webview'),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${debuggingPort} --remote-debugging-address=127.0.0.1`};
 delete env.WERC_ASR_DIRECTORY;
 child=spawn(executable,['--data-directory',relative(audit,join(profile,'data'))],{cwd:audit,windowsHide:true,stdio:'ignore',env});
 const page=await until(async()=>{assert.equal(child.exitCode,null,'Desktop exited');try{return(await(await fetch(`http://127.0.0.1:${debuggingPort}/json/list`)).json()).find(p=>p.type==='page'&&!p.url.startsWith('devtools:'));}catch{return null;}},'WebView');
 const socket=new WebSocket(page.webSocketDebuggerUrl);await new Promise((ok,no)=>{socket.addEventListener('open',ok,{once:true});socket.addEventListener('error',no,{once:true});});
 cdp=new Cdp(socket);await until(()=>cdp.evaluate('Boolean(window.__TAURI_INTERNALS__)'),'IPC');
 await until(()=>cdp.evaluate("location.hostname === 'tauri.localhost' || location.protocol === 'tauri:'"),'embedded production page');
 await until(async()=>{const state=await cdp.invoke('startup_status');assert.notEqual(state.phase,'failed',state.error);return state.phase==='ready';},'startup');
 assert.ok(!page.url.includes('5173'),'Release must not depend on Vite');
}
async function quit(){
 if(child?.exitCode!==null)return;
 void cdp.invoke('close_desktop',{action:'exit'}).catch(()=>{});
 await until(()=>child.exitCode!==null,'exit');assert.equal(child.exitCode,0);await cdp.close();
}
try{
 await launch();
 const location=await cdp.invoke('runtime_location');assert.equal(location.identifier,'local.werc.mon3trbot');
 // Resolve both paths because the CLI accepts a relative directory.
 assert.equal(toNamespacedPath(resolve(location.data_directory)),toNamespacedPath(join(profile,'data')));
 const lock=await cdp.invoke('app_lock_status');assert.equal(lock.configured,false);assert.equal(lock.locked,false);checks.push('application-password-default-off');
 // This smoke starts with setup completed; the separate first-run suite exercises the wizard.
 await cdp.evaluate(`localStorage.setItem('werc.first-run.v1','complete')`);
 await cdp.call('Page.reload');
 await until(async()=>{try{return await cdp.evaluate(`Boolean(document.querySelector('[aria-label="主导航"]'))`);}catch{return false;}},'workbench-after-setup-fixture');
 const initial=await cdp.invoke('bootstrap');assert.equal(initial.accounts.length,0);assert.equal(initial.status.version,version);assert.equal(initial.status.stage,'stable');
 assert.equal((await cdp.invoke('native_status')).connected,false);checks.push('normal-release-identity','fresh-explicit-profile','no-dev-server','no-wechat-attachment');
 const report=await cdp.invoke('diagnostic_report');assert.ok(report.components.length===4);assert.ok(report.components.every(c=>c.status==='verified'),JSON.stringify(report.components));
 const prefs=await cdp.invoke('desktop_preferences');assert.equal(prefs.diagnostics_logging,true);assert.equal(prefs.asr_provider,'cpu');
 console.log('Components verified; checking bundled ASR and API');
 const asr=await cdp.invoke('asr_status');assert.equal(asr.available,true);assert.ok(asr.models.includes('tiny'));
 const catalog=await cdp.invoke('model_catalog');assert.ok(catalog.installed.includes('tiny'));assert.ok(!catalog.removable.includes('tiny'));
 assert.equal((await cdp.invoke('sherpa_catalog')).cpu_runtime_available,true);checks.push('native-components-hashes','default-cpu-model-ready','bundled-model-preserved','logging-default');
 const apiPort=await port();await cdp.invoke('set_api_enabled',{enabled:true,port:apiPort});
 const health=await(await fetch(`http://127.0.0.1:${apiPort}/health`)).json();assert.equal(health.version,version);
 assert.equal((await fetch(`http://127.0.0.1:${apiPort}/api/v1/accounts`)).status,401);
 await cdp.invoke('set_api_enabled',{enabled:false,port:apiPort});checks.push('loopback-health','unauthenticated-business-denied');
 if(mode==='portable'){
  console.log('Testing owned audio with shipped default model');
  const account=await cdp.invoke('register_account',{label:'自动化测试：隔离发行验收'});
  const bytes=[...await readFile(join(root,'target/voice-validation-20261111/automation-test-native-voice.wav'))];
  const asset=await cdp.invoke('upload_audio',{accountId:account.id,bytes});
  const transcript=await cdp.invoke('transcribe_audio',{accountId:account.id,source:{kind:'uploaded',asset_id:asset.id},model:'tiny',language:'zh',requestId:'release-owned-asr'});
  assert.ok(transcript.text.trim());checks.push('actual-default-asr-transcription');
  await cdp.invoke('diagnostic_frontend_error',{code:'FRONTEND_ERROR'});assert.ok((await cdp.invoke('diagnostic_report')).log_records.some(r=>r.code==='FRONTEND_ERROR'));
 }
 await until(()=>cdp.evaluate(`Boolean(document.querySelector('[aria-label="主导航"]'))`),'frontend');
 await cdp.evaluate(`document.querySelector('[aria-label="收起使用指南"]')?.click()`);
 for(const width of [1200,390]){
  await cdp.call('Emulation.setDeviceMetricsOverride',{width,height:900,deviceScaleFactor:1,mobile:false});
  await cdp.evaluate(`document.querySelector('details.nav-tools').open=true`);
  for(const label of ['设置','Bot 接入','发送队列','连接','能力','音频','资料库']){
   await cdp.evaluate(`document.querySelector('nav [aria-label="${label}"]').click()`);await sleep(250);
   const layout=await cdp.evaluate(`({overflow:document.documentElement.scrollWidth>window.innerWidth+2,text:document.body.innerText})`);
   assert.equal(layout.overflow,false,`${label} overflow at ${width}`);
   assert.ok(!/\bDEV\b|开发中|正常环境开发版|foundation_development/.test(layout.text),label);
  }
  await cdp.evaluate(`document.querySelector('nav [aria-label="设置"]').click()`);await sleep(250);
  const shot=await cdp.call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});await writeFile(join(audit,`${mode}-${width}.png`),Buffer.from(shot.data,'base64'));
 }
 checks.push('responsive-1200-390','final-ui-version-and-labels');
 const updates=await cdp.invoke('update_status');assert.equal(updates.configured,false);assert.equal(updates.current_version,version);
 await quit();
 // Reopen the same final binary/profile to verify preferences and encrypted store persist.
 await launch();assert.equal((await cdp.invoke('desktop_preferences')).asr_provider,'cpu');
 const reopened=await cdp.invoke('bootstrap');assert.equal(reopened.accounts.length,mode==='portable'?1:0);
 await quit();checks.push('reopen-same-release-profile');
 await writeFile(join(audit,`runtime-${mode}.json`),JSON.stringify({passed:true,mode,version,desktop_sha256:createHash('sha256').update(await readFile(executable)).digest('hex'),checks,normal_product_data_opened:false,wechat_connected:false,messages_sent:false},null,2));
 console.log(`Final ${mode} runtime passed: ${checks.join(', ')}`);
}finally{
 if(child?.exitCode===null){try{await quit();}catch{child.kill();}}
 if(cdp)await cdp.close();
}
