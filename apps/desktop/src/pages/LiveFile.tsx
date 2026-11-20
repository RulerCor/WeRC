import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
export function LiveFile({accountId,viewId,localId}:{accountId:string;viewId:string;localId:string}) {
  const [info,setInfo]=useState<{file_name:string;size:number}|null>(null),[path,setPath]=useState(''),[busy,setBusy]=useState(false),[error,setError]=useState(''),[saved,setSaved]=useState(false);
  const generation=useRef(0);
  useEffect(()=>{const id=++generation.current;setInfo(null);setBusy(false);setPath('');setSaved(false);setError('');void command<{file_name:string;size:number}>('native_live_file_info',{accountId,viewId,localId}).then(v=>{if(id===generation.current)setInfo(v);},()=>{});return()=>{generation.current++;};},[accountId,viewId,localId]);
  async function save(){const id=generation.current;setBusy(true);setError('');try{await command('native_live_file_save',{accountId,viewId,localId,path});if(id===generation.current)setSaved(true);}catch(e){if(id===generation.current)setError(e instanceof Error?e.message:String(e));}finally{if(id===generation.current)setBusy(false);}}
  if(!info)return null;
  return <details className="live-file"><summary>{info.file_name} · {info.size.toLocaleString()} 字节</summary><form onSubmit={e=>{e.preventDefault();void save();}}><label>保存位置（相对系统下载目录）<input aria-label="在线附件保存路径" value={path} placeholder={`下载目录内文件名，例如 ${info.file_name}`} disabled={busy} onChange={e=>{setPath(e.target.value);setSaved(false);}}/></label><button className="button" disabled={busy||!path.trim()}>{busy?'正在下载…':'下载并保存附件'}</button></form><span className="field">窗口有效期间可下载。创建新文件，不覆盖已有文件。</span>{saved&&<p role="status">文件已保存。</p>}{error&&<p className="error-notice" role="alert">{error}</p>}</details>;
}
