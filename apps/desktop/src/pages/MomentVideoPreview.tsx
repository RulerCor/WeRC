import {useEffect,useState} from 'react';
import {ImagePreview} from './ImagePreview';
import {VideoPreview} from './VideoPreview';
import type {VideoSendAsset} from '../lib/outbox';
import type {MomentImageAsset} from './MomentImagePreview';
export function MomentVideoPreview({accountId,video,cover,onError,onReady}:{accountId:string;video:VideoSendAsset;cover:MomentImageAsset;onError:(message:string)=>void;onReady:(ready:boolean)=>void}) {
  const [videoReady,setVideoReady]=useState(false),[coverReady,setCoverReady]=useState(false);
  useEffect(()=>{onReady(videoReady&&coverReady);return()=>onReady(false);},[videoReady,coverReady,onReady]);
  return <div><strong>视频动态 · {(video.info.duration_ms/1000).toFixed(2)} 秒</strong>
    <VideoPreview accountId={accountId} assetId={video.file.asset_id} onError={onError} onReady={setVideoReady}/>
    <details><summary>查看封面和素材信息</summary><ImagePreview accountId={accountId} assetId={cover.asset_id} onError={onError} onReady={setCoverReady}/><p>{video.file.file_name} · {video.info.width}×{video.info.height}</p><code>{video.file.sha256}</code></details>
  </div>;
}
