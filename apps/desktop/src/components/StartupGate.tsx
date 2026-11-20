import {useEffect,useState,type ReactNode} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
interface Progress {phase:'starting'|'migrating'|'canceling'|'verifying'|'ready'|'failed';copied:number;total:number;error:string|null}
export function StartupGate({children}:{children:ReactNode}){
 const [status,setStatus]=useState<Progress>({phase:desktopAvailable?'starting':'ready',copied:0,total:0,error:null}),[error,setError]=useState('');
 useEffect(()=>{if(!desktopAvailable||status.phase==='ready'||status.phase==='failed')return;let alive=true,pending=false;const read=async()=>{if(pending)return;pending=true;try{const value=await command<Progress>('startup_status');if(alive){setStatus(value);setError('');}}catch(e){if(alive)setError(String(e));}finally{pending=false;}};void read();const timer=setInterval(()=>void read(),500);return()=>{alive=false;clearInterval(timer);};},[status.phase]);
 if(status.phase==='ready')return <>{children}</>;
 const migrating=['migrating','canceling','verifying'].includes(status.phase);
 return <main className="ef app-lock-screen"><section className="panel" aria-label="准备本机资料"><h1>{status.phase==='failed'?'暂时无法打开资料':migrating?'正在迁移本机资料':'正在打开 WeRC'}</h1>{migrating&&<><progress aria-label="迁移进度" value={status.copied} max={Math.max(status.total,1)}/><p role="status">{status.phase==='canceling'?'正在取消…':status.phase==='verifying'?'正在校验副本…':`${(status.copied/1024**2).toFixed(1)} / ${(status.total/1024**2).toFixed(1)} MiB`}</p><p>完成校验后将打开资料。原位置的备份会保留。</p><button className="button" disabled={status.phase!=='migrating'} onClick={()=>void command('cancel_startup_migration').catch(e=>setError(String(e)))}>取消迁移，使用原位置</button></>}{(error||status.error)&&<p role="alert" className="error-notice">{error||status.error}</p>}{status.phase==='failed'&&<button className="button" onClick={()=>void command('exit_failed_startup').catch(e=>setError(String(e)))}>退出</button>}</section></main>;
}
