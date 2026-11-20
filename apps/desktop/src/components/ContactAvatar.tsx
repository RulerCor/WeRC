import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
export function ContactAvatar({accountId,viewId,username,name,available=true}:{accountId:string;viewId:string;username:string;name:string;available?:boolean}){
  const [url,setUrl]=useState(''),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const request=useRef<string|null>(null),generation=useRef(0),objectUrl=useRef('');
  useEffect(()=>{setUrl('');setBusy(false);setError('');objectUrl.current='';request.current=null;return()=>{generation.current++;if(objectUrl.current)URL.revokeObjectURL(objectUrl.current);if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});};},[accountId,viewId,username]);
  async function read(){
    if(request.current)return;const id=crypto.randomUUID(),expected=generation.current;request.current=id;setBusy(true);setError('');
    try{const result=await command<{png_hex:string}>('native_contact_avatar',{accountId,viewId,username,requestId:id});if(expected!==generation.current)return;
      if(!result.png_hex.length||result.png_hex.length>2*1024*1024||result.png_hex.length%2||!/^[a-f0-9]+$/i.test(result.png_hex))throw new Error('头像数据无效。');
      const bytes=Uint8Array.from(result.png_hex.match(/../g)!,h=>parseInt(h,16));
      if(objectUrl.current)URL.revokeObjectURL(objectUrl.current);objectUrl.current=URL.createObjectURL(new Blob([bytes],{type:'image/png'}));setUrl(objectUrl.current);
    }catch(e){if(expected===generation.current)setError(String(e));}
    finally{if(expected===generation.current){request.current=null;setBusy(false);}}
  }
  return <div className="contact-avatar-control">{url?<img src={url} alt={`${name}的头像`} width={48} height={48} style={{borderRadius:12,objectFit:'cover'}} onError={()=>{setUrl('');setError('头像无法显示，可重新读取。');}}/>:<span className="chat-avatar" aria-hidden="true">{Array.from(name)[0]||'?'}</span>}
    {available&&!url&&<button className="button" disabled={busy} onClick={()=>void read()} aria-label={`读取${name}的头像`}>{busy?'读取中…':'读取头像'}</button>}{error&&<span className="field" role="status">{error}</span>}
  </div>;
}
