import {useEffect,useRef,useState} from 'react';
import {command,type FileAsset} from '../lib/desktop';

export function ReceivedImage({accountId,position,connected}:{accountId:string;position:number;connected:boolean}) {
  const [saved,setSaved]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState(''),[url,setUrl]=useState('');
  const generation=useRef(0),objectUrl=useRef('');
  useEffect(()=>{const token=++generation.current;setSaved(false);setBusy(false);setError('');setUrl('');void command<FileAsset|null>('native_received_image_asset',{accountId,position}).then(a=>{if(token===generation.current)setSaved(Boolean(a));},e=>{if(token===generation.current)setError(e instanceof Error?e.message:String(e));});return()=>{generation.current++;if(objectUrl.current)URL.revokeObjectURL(objectUrl.current);objectUrl.current='';};},[accountId,position]);
  async function view() {
    const token=generation.current;
    setBusy(true);setError('');
    try {
      if(!saved){await command('native_received_image_download',{accountId,position});if(token!==generation.current)return;setSaved(true);}
      const values=await command<number[]>('native_received_image_preview',{accountId,position});
      const bytes=new Uint8Array(values);values.fill(0);
      try {
        if(token!==generation.current)return;
        if(!bytes.length||bytes.length>8*1024*1024)throw new Error('图片内容大小无效');
        const next=URL.createObjectURL(new Blob([bytes],{type:'image/png'}));
        if(objectUrl.current)URL.revokeObjectURL(objectUrl.current);objectUrl.current=next;setUrl(next);
      }finally{bytes.fill(0);}
    }catch(e){if(token===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(token===generation.current)setBusy(false);}
  }
  function close(){if(objectUrl.current)URL.revokeObjectURL(objectUrl.current);objectUrl.current='';setUrl('');}
  return <div className="received-image">
    {url?<><img src={url} alt="收到的图片" style={{display:'block',maxWidth:'100%',maxHeight:480,objectFit:'contain'}}/><button className="button" onClick={close}>收起图片</button></>:<button className="button" disabled={busy||(!saved&&!connected)} onClick={()=>void view()}>{busy?'正在读取图片…':saved?'查看已保存图片':'下载并查看图片'}</button>}
    {!saved&&!connected&&<span className="field">连接原生通道后可下载</span>}
    {saved&&<span className="field">本机加密缓存 / 24小时</span>}
    {error&&<p className="error-notice" role="alert">{error}</p>}
  </div>;
}
