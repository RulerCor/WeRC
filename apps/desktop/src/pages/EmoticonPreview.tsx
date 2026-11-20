import {emoticonMime} from '../lib/emoticon';
import {useEffect,useState} from 'react';
import {command} from '../lib/desktop';

export function EmoticonPreview({accountId,assetId,onError,onReady}:{accountId:string;assetId:string;onError:(message:string)=>void;onReady?:(ready:boolean)=>void}) {
  const [playing,setPlaying]=useState(false),[url,setUrl]=useState('');
  useEffect(()=>{
    let active=true,current='';
    setUrl('');onReady?.(false);
    if(playing)void (async()=>{
      let bytes:Uint8Array<ArrayBuffer>|undefined;
      try {
        const buffer=await command<ArrayBuffer>('preview_emoticon_content',{accountId,id:assetId});
        if(!(buffer instanceof ArrayBuffer))throw new Error('表情响应格式无效。');
        bytes=new Uint8Array(buffer);
        if(!active)return;
        current=URL.createObjectURL(new Blob([buffer],{type:emoticonMime(bytes)}));setUrl(current);
      }catch(e){if(active){setPlaying(false);onReady?.(false);onError(e instanceof Error?e.message:String(e));}}
      finally{bytes?.fill(0);}
    })();
    return()=>{active=false;if(current)URL.revokeObjectURL(current);};
  },[accountId,assetId,playing,onError,onReady]);
  return <div><button className="button" onClick={()=>setPlaying(v=>!v)}>{playing?'停止表情预览':'播放表情预览'}</button>
    {playing&&!url&&<p className="field">正在读取表情…</p>}
    {url&&<img src={url} alt="表情预览" style={{display:'block',maxWidth:'100%',maxHeight:360,objectFit:'contain'}} onLoad={()=>onReady?.(true)} onError={()=>{setPlaying(false);onReady?.(false);onError('当前设备无法显示此表情。');}}/>}
  </div>;
}
