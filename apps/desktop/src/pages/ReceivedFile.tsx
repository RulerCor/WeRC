import {useEffect,useRef,useState} from 'react';
import {command,type FileAsset} from '../lib/desktop';
interface Info {file_name:string;size:number}
export function ReceivedFile({accountId,position,connected}:{accountId:string;position:number;connected:boolean}) {
  const [info,setInfo]=useState<Info|null>(null),[saved,setSaved]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState(''),[path,setPath]=useState(''),[exported,setExported]=useState(false);
  const generation=useRef(0);
  useEffect(()=>{const current=++generation.current;setInfo(null);setBusy(false);setSaved(false);setPath('');setExported(false);setError('');
    void Promise.all([command<Info|null>('native_received_file_info',{accountId,position}),command<FileAsset|null>('native_received_file_asset',{accountId,position})]).then(([i,a])=>{if(current===generation.current){setInfo(i);setSaved(Boolean(a));}},e=>{if(current===generation.current)setError(e instanceof Error?e.message:String(e));});
    return()=>{generation.current++;};
  },[accountId,position]);
  async function perform(save:boolean) { const current=generation.current;setBusy(true);setError('');
    try{if(save){await command('native_received_file_save',{accountId,position,path});if(current===generation.current)setExported(true);}else{await command('native_received_file_download',{accountId,position});if(current===generation.current)setSaved(true);}}
    catch(e){if(current===generation.current)setError(e instanceof Error?e.message:String(e));}finally{if(current===generation.current)setBusy(false);}
  }
  return <div className="received-file">{info&&<><strong>{info.file_name}</strong><span className="field">{info.size.toLocaleString()} 字节</span>
    {!saved?<button className="button" disabled={busy||!connected} onClick={()=>void perform(false)}>{busy?'正在下载…':'下载附件'}</button>:<details><summary>保存到电脑</summary><form onSubmit={e=>{e.preventDefault();void perform(true);}}><label>保存位置（相对系统下载目录）<input aria-label="接收附件保存路径" value={path} placeholder={`下载目录内文件名，例如 ${info.file_name}`} onChange={e=>{setPath(e.target.value);setExported(false);}} disabled={busy}/></label><button className="button" disabled={busy||!path.trim()}>保存文件</button></form><span className="field">本机加密缓存24小时。保存时创建新文件，不覆盖已有文件。</span>{exported&&<p role="status">文件已保存。</p>}</details>}
    </>}{error&&<p className="error-notice" role="alert">{error}</p>}</div>;
}
