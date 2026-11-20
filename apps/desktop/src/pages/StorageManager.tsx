import {StorageLocation} from './StorageLocation';
import {useEffect,useRef,useState} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {ModelManager} from './ModelManager';
interface Category {id:string;label:string;bytes:number;files:number;clearable:boolean;note:string}
interface Usage {database:{database_bytes:number;reusable_bytes:number;query_bytes:number;query_rows:number;protected_records:number};categories:Category[]}
const size=(n:number)=>n>=1024**3?`${(n/1024**3).toFixed(2)} GiB`:n>=1024**2?`${(n/1024**2).toFixed(1)} MiB`:`${(n/1024).toFixed(1)} KiB`;
export function StorageManager(){
 const [usage,setUsage]=useState<Usage|null>(null),[busy,setBusy]=useState(false),[selected,setSelected]=useState<Category|null>(null),[error,setError]=useState(''),[notice,setNotice]=useState('');const request=useRef('');
 useEffect(()=>()=>{if(request.current)void command('cancel_storage_scan',{requestId:request.current}).catch(()=>{});},[]);
 async function scan(){const id=crypto.randomUUID();request.current=id;setBusy(true);setError('');try{setUsage(await command<Usage>('scan_storage',{requestId:id}));}catch(e){setError(String(e));}finally{if(request.current===id)request.current='';setBusy(false);}}
 async function clean(){if(!selected)return;setBusy(true);setError('');try{await command('clear_storage_category',{category:selected.id});setSelected(null);setUsage(null);setNotice('清理完成。数据库释放的页会在后续写入时复用，文件大小可能不会立即缩小。');}catch(e){setError(String(e));}finally{setBusy(false);}}
 return <details className="panel"><summary>存储占用与清理</summary><div className="section-title"><h2>本机存储</h2><HelpTip label="存储管理说明">按实际存储位置扫描，可取消。查询缓存可重新建立；消息、账号、密钥、转写稿、收藏和归档媒体属于受保护资料。各项是独立位置或逻辑内容，可能包含关系，不应直接相加。不会修改微信数据目录。</HelpTip></div><div className="actions"><button className="button" disabled={!desktopAvailable||busy} onClick={()=>void scan()}>扫描占用</button>{busy&&request.current&&<button className="button" onClick={()=>void command('cancel_storage_scan',{requestId:request.current}).catch(e=>setError(String(e)))}>取消扫描</button>}</div>
 {usage&&<><dl>{usage.categories.map(c=><div key={c.id}><dt>{c.label} <HelpTip label={`${c.label}说明`}>{c.note}</HelpTip></dt><dd>{size(c.bytes)} · {c.files.toLocaleString()} 项 {c.clearable?<button className="button" disabled={busy||c.files===0} onClick={()=>setSelected(c)}>清理…</button>:<span className="field">保留</span>}</dd></div>)}</dl><p className="field">数据库已分配 {size(usage.database.database_bytes)}，其中 {size(usage.database.reusable_bytes)} 可复用。已保存消息与资料条目 {usage.database.protected_records.toLocaleString()}。</p></>}
 <details><summary>过期上传素材</summary><p>清理已过期且按原有规则可移除的临时素材。已归档的媒体副本保留。</p><button className="button" disabled={!desktopAvailable||busy} onClick={()=>setSelected({id:'expired_uploads',label:'过期上传素材',bytes:0,files:0,clearable:true,note:'仅处理已经过期的素材。'})}>清理过期素材…</button></details>
 {selected&&<div className="confirm-panel" role="group" aria-label="确认清理存储"><p>清理“{selected.label}”？{selected.note}</p><div className="actions"><button className="button danger" disabled={busy} onClick={()=>void clean()}>确认清理</button><button className="button" disabled={busy} onClick={()=>setSelected(null)}>取消</button></div></div>}
 <StorageLocation/><ModelManager/>{notice&&<p role="status">{notice}</p>}{error&&<p role="alert" className="error-notice">{error}</p>}</details>;
}
