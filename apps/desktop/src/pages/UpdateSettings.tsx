import {useEffect,useState} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';

interface UpdateStatus {
  configured:boolean;current_version:string;phase:string;version:string|null;notes:string|null;
  downloaded_bytes:number;total_bytes:number|null;release_page_available:boolean;
  error:{code:string;message:string}|null;
}
const labels:Record<string,string>={idle:'尚未检查',checking:'正在检查…',available:'有新版本',up_to_date:'已是当前发布源的最新版本',downloading:'正在下载…',ready:'下载完成',installing:'正在启动安装…',canceled:'下载已取消',failed:'更新未完成'};
export function UpdateSettings(){
  const [status,setStatus]=useState<UpdateStatus|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[confirm,setConfirm]=useState(false);
  useEffect(()=>{
    if(!desktopAvailable||busy)return;let alive=true,pending=false;
    const read=async()=>{if(pending)return;pending=true;try{const next=await command<UpdateStatus>('update_status');if(alive)setStatus(next);}catch(e){if(alive)setError(e instanceof Error?e.message:String(e));}finally{pending=false;}};
    void read();const timer=setInterval(()=>void read(),1000);return()=>{alive=false;clearInterval(timer);};
  },[busy]);
  async function act(name:string){setBusy(true);setError('');setConfirm(false);try{setStatus(await command<UpdateStatus>(name));}catch(e){setError(e instanceof Error?e.message:String(e));}finally{setBusy(false);}}
  const active=busy||['checking','downloading','installing'].includes(status?.phase??'');
  return <details className="panel"><summary>应用更新</summary>
    <div className="section-title"><h3>版本 {status?.current_version??'—'} <HelpTip label="应用更新说明">WeRC 从 GitHub 自动检查、下载和安装更新。发送任务结束后会自动重启。</HelpTip></h3><span className="state-tag" role="status">{status&&!status.configured?'仅手动更新':labels[status?.phase??'idle']??status?.phase}</span></div>
    {status&&!status.configured&&<p className="field">请从 GitHub 发布页下载新版。</p>}
    {status?.version&&<p>可用版本：{status.version}</p>}
    {status?.notes&&<details><summary>版本说明</summary><p style={{whiteSpace:'pre-wrap',overflowWrap:'anywhere'}}>{status.notes}</p></details>}
    {status?.phase==='downloading'&&<div role="status"><progress aria-label="更新下载进度" value={status.total_bytes?status.downloaded_bytes:undefined} max={status.total_bytes??undefined}/><span> {(status.downloaded_bytes/1048576).toFixed(1)} MiB{status.total_bytes?` / ${(status.total_bytes/1048576).toFixed(1)} MiB`:''}</span></div>}
    {(error||status?.error)&&<p role="alert">{error||status?.error?.message}</p>}
    <div className="actions">
      <button className="button" disabled={!desktopAvailable||!status?.configured||active} onClick={()=>void act('check_for_update')}>检查更新</button>
      {status?.version&&status.phase!=='ready'&&<button className="button" disabled={!desktopAvailable||active} onClick={()=>void act('download_update')}>{['failed','canceled'].includes(status.phase)?'重新下载':'下载更新'}</button>}
      {status?.phase==='downloading'&&<button className="button" disabled={busy} onClick={()=>void act('cancel_update_download')}>取消下载</button>}
      {status?.phase==='ready'&&<button className="button primary" disabled={busy} onClick={()=>setConfirm(true)}>安装更新…</button>}
      {status?.release_page_available&&<button className="button" disabled={!desktopAvailable||busy} onClick={()=>void command('open_release_page').catch(e=>setError(e instanceof Error?e.message:String(e)))}>查看发布页</button>}
    </div>
    {confirm&&<div role="alertdialog" aria-label="确认安装更新"><p>安装将退出WeRC，原生连接和API随之停止。</p><div className="actions"><button className="button" disabled={busy} onClick={()=>setConfirm(false)}>稍后安装</button><button className="button primary" disabled={busy} onClick={()=>void act('install_update')}>退出并安装</button></div></div>}
  </details>;
}
