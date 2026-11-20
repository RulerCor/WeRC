import { useEffect, useRef, useState, type FormEvent } from 'react';
import {listen} from '@tauri-apps/api/event';
import { command, desktopAvailable, type ClientInstallation, type DiscoveryReport } from '../lib/desktop';
import type { Workbench } from '../hooks/useWorkbench';
import { DataSources } from './DataSources';
import {HelpTip} from '../components/HelpTip';

interface ScanProgress {request_id:string;sequence:number;phase:string;bytes_read:number}

export function KeyAcquisition({ accountId, installation, busy, act }: Pick<Workbench, 'busy' | 'act'> & { accountId: string; installation: ClientInstallation }) {
  const [pid, setPid] = useState('');
  const [database, setDatabase] = useState('');
  const [consent, setConsent] = useState(false);
  const [processes, setProcesses] = useState<number[] | null>(null);
  const [scanning,setScanning]=useState(false),[progress,setProgress]=useState<ScanProgress|null>(null),[timeout,setTimeoutSeconds]=useState(30),[elapsed,setElapsed]=useState(0),[canceling,setCanceling]=useState(false),[error,setError]=useState('');
  const request=useRef<string|null>(null),generation=useRef(0);
  useEffect(()=>{setProgress(null);setScanning(false);setCanceling(false);setError('');setProcesses(null);setPid('');setDatabase('');setConsent(false);return()=>{generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});request.current=null;};},[accountId,installation.id,installation.sha256]);
  useEffect(()=>{if(!scanning)return;const started=Date.now();setElapsed(0);const timer=setInterval(()=>setElapsed(Math.floor((Date.now()-started)/1000)),500);return()=>clearInterval(timer);},[scanning]);
  const disabled = busy || scanning || !desktopAvailable;
  const validPid = /^\d{1,10}$/.test(pid) && Number(pid) > 0 && Number(pid) <= 0xffffffff;
  const supported = installation.architecture === 'x86_64' && /^4\.(1|2)\.\d+\.\d+$/.test(installation.version);
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!consent || !validPid || !database.trim() || !supported || disabled) return;
    const processId = Number(pid), selected = database.trim();
    setConsent(false);
    const id=crypto.randomUUID(),g=++generation.current;request.current=id;setScanning(true);setProgress(null);setError('');setCanceling(false);
    let unlisten:(()=>void)|undefined;
    try{
      unlisten=await listen<ScanProgress>('key-scan-progress',event=>{if(g!==generation.current||event.payload.request_id!==id)return;setProgress(old=>!old||event.payload.sequence>old.sequence?event.payload:old);});
      if(g!==generation.current)return;
      await act(() => command('acquire_database_key', { accountId, processId, database: selected, confirmed: true, requestId:id, timeoutSeconds:timeout }));
    }catch(e){if(g===generation.current)setError(String(e));}
    finally{try{await unlisten?.();}catch{/* The window may already be closing. */}if(g===generation.current){request.current=null;setScanning(false);setCanceling(false);}}
  }
  return <details className="permission-details key-acquisition">
    <summary>自动取钥 / 实验性</summary>
    <p className="field">微信 4.1.x / 4.2.x · x64。逐库首验不代表账号归属、完整解密或收发已就绪。</p>
    <button className="button" disabled={disabled || !supported} onClick={() => act(async () => {
      const report = await command<DiscoveryReport>('discover_clients');
      const match = report.installations.find(item => item.id === installation.id && item.sha256 === installation.sha256);
      setProcesses(match?.process_ids ?? []); setPid(''); setConsent(false);
    })}>列出此安装进程</button>
    {processes !== null && (processes.length ? <label>运行进程<select aria-label="取钥进程列表" value={pid} disabled={disabled} onChange={e => { setPid(e.target.value); setConsent(false); }}><option value="">选择 PID</option>{processes.map(id => <option key={id} value={id}>{id}</option>)}</select></label> : <p className="field">此安装没有检测到运行进程。</p>)}
    <DataSources busy={disabled || !supported} act={act} onChoose={path => { setDatabase(path); setConsent(false); }}/>
    <form onSubmit={submit} className="source-form">
      <label>目标进程 PID<input aria-label="取钥进程PID" inputMode="numeric" maxLength={10} value={pid} disabled={disabled || !supported} onChange={e => { setPid(e.target.value); setConsent(false); }}/></label>
      <label>待验真数据库<input aria-label="取钥数据库路径" value={database} disabled={disabled || !supported} onChange={e => { setDatabase(e.target.value); setConsent(false); }} placeholder="所选加密数据库的完整路径"/></label>
      <label className="permission-choice"><input type="checkbox" aria-label="确认只读取钥" checked={consent} disabled={disabled || !supported} onChange={e => setConsent(e.target.checked)}/>我有权读取此进程及数据库第一页，并保护匹配的密钥</label>
      <button className="button" disabled={disabled || !supported || !validPid || !database.trim() || !consent || !Number.isInteger(timeout) || timeout<1 || timeout>30}>获取并保护密钥</button>
    </form>
    <details><summary>扫描选项</summary><label>扫描时间上限（秒）<input type="number" min={1} max={30} value={timeout} disabled={disabled} onChange={e=>setTimeoutSeconds(Number(e.target.value))}/></label><HelpTip label="取钥扫描进度说明">扫描使用只读的WCDB配置路径，不输出内存或密钥。显示实际累计读取字节和单次事件序号，多轮扫描可能读取同一区域，字节数不是完成百分比。取消后停止后续读取，未提交的候选不保存；取消到达前已经完成保存时保留成功结果。</HelpTip></details>
    {scanning&&<div role="status"><progress aria-label="取钥扫描进度"/> 已用时 {elapsed} 秒 · 已读取 {((progress?.bytes_read??0)/1048576).toFixed(1)} MiB · 事件 {progress?.sequence??0} <button type="button" className="button" disabled={canceling} onClick={()=>{const id=request.current;if(!id)return;setCanceling(true);void command('cancel_browse',{requestId:id}).catch(e=>{setError(String(e));setCanceling(false);});}}>{canceling?'正在取消…':'取消取钥'}</button></div>}
    {!scanning&&progress&&<p role="status">{progress.phase==='complete'?'密钥已验证并保护':progress.phase==='canceled'?'取钥已取消':progress.phase==='failed'?'本次取钥未完成':'扫描已结束'} · 事件 {progress.sequence}</p>}
    {error&&<p role="alert">{error}</p>}
    <p className="field">不提权、不注入、不写来源，不输出原始密钥。最多 30 秒扫描、32 个数据库密钥；首次成功会替换未验真的手动密钥。</p>
    {!supported && <p className="field">此安装版本或架构尚未适配取钥布局。</p>}
  </details>;
}
