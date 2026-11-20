import { useEffect, useState } from 'react';
import { command } from '../lib/desktop';

export function ImagePreview({accountId, assetId, onError, onReady}: {accountId:string; assetId:string; onError:(message:string)=>void; onReady?:(ready:boolean)=>void}) {
  const [url,setUrl]=useState('');
  useEffect(()=>{
    let active=true, current='';
    setUrl(''); onReady?.(false);
    void (async()=>{
      let values:number[]=[]; let bytes:Uint8Array<ArrayBuffer>|undefined;
      try {
        values=await command<number[]>('read_image_preview',{accountId,id:assetId});
        if(!active)return;
        bytes=new Uint8Array(values);
        current=URL.createObjectURL(new Blob([bytes],{type:'image/png'}));
        setUrl(current);
      } catch(e) {if(active)onError(e instanceof Error?e.message:'无法读取图片。');}
      finally {values.fill(0);bytes?.fill(0);}
    })();
    return ()=>{active=false;if(current)URL.revokeObjectURL(current);};
  },[accountId,assetId,onError,onReady]);
  return url ? <img className="image-preview" src={url} alt="待发送的规范化图片" onLoad={()=>onReady?.(true)} onError={()=>{onReady?.(false);onError('图片预览失败。');}}/> : <p className="field">正在读取图片预览…</p>;
}
