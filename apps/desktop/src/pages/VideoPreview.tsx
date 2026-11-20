import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';

export function VideoPreview({accountId,assetId,onError,onReady}:{accountId:string;assetId:string;onError:(message:string)=>void;onReady?:(ready:boolean)=>void}) {
  const [url,setUrl]=useState('');
  const player=useRef<HTMLVideoElement>(null);
  useEffect(()=>{
    let active=true,current='';
    const element=player.current;
    setUrl('');onReady?.(false);
    void (async()=>{
      let bytes:Uint8Array<ArrayBuffer>|undefined;
      try {
        const buffer=await command<ArrayBuffer>('preview_video_content',{accountId,id:assetId});
        if(!(buffer instanceof ArrayBuffer))throw new Error('视频响应格式无效。');
        bytes=new Uint8Array(buffer);
        if(!active)return;
        if(!bytes.length||bytes.length>8*1024*1024)throw new Error('视频大小无效。');
        current=URL.createObjectURL(new Blob([buffer],{type:'video/mp4'}));setUrl(current);
      }catch(e){if(active){onReady?.(false);onError(e instanceof Error?e.message:String(e));}}
      finally{bytes?.fill(0);}
    })();
    return()=>{active=false;element?.pause();if(current)URL.revokeObjectURL(current);};
  },[accountId,assetId,onError,onReady]);
  return <div>{!url&&<p className="field">正在读取视频预览…</p>}<video ref={player} src={url||undefined} hidden={!url} controls playsInline preload="auto" aria-label="待发送视频" style={{display:url?'block':'none',width:'100%',maxHeight:360,objectFit:'contain'}} onLoadedData={()=>onReady?.(true)} onError={()=>{onReady?.(false);onError('当前设备无法播放此视频，暂不能批准发送。');}}/></div>;
}
