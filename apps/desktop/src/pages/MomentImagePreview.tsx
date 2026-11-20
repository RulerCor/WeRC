import {useCallback,useEffect,useRef} from 'react';
import {ImagePreview} from './ImagePreview';
export interface MomentImageAsset {asset_id:string;source_sha256:string;png_sha256:string;width:number;height:number}
export function MomentImagePreview({accountId,images,onError,onReady}:{accountId:string;images:MomentImageAsset[];onError:(message:string)=>void;onReady:(ready:boolean)=>void}){
  const ready=useRef(new Set<number>());
  const update=useCallback((index:number,value:boolean)=>{if(value)ready.current.add(index);else ready.current.delete(index);onReady(images.length>0&&ready.current.size===images.length);},[images.length,onReady]);
  useEffect(()=>{ready.current.clear();onReady(false);return()=>onReady(false);},[accountId,images,onReady]);
  return <ol className="moment-image-grid moment-image-preview" aria-label="待发布图片顺序">{images.map((image,index)=><MomentImageTile key={`${index}/${image.asset_id}`} accountId={accountId} image={image} index={index} update={update} onError={onError}/>)}</ol>;
}
function MomentImageTile({accountId,image,index,update,onError}:{accountId:string;image:MomentImageAsset;index:number;update:(index:number,ready:boolean)=>void;onError:(message:string)=>void}){
  const onReady=useCallback((ready:boolean)=>update(index,ready),[index,update]);
  return <li><strong>第 {index+1} 张 · {image.width}×{image.height}</strong><ImagePreview accountId={accountId} assetId={image.asset_id} onError={onError} onReady={onReady}/><details><summary>图片摘要</summary><code>{image.png_sha256}</code></details></li>;
}
