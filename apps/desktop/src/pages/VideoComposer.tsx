import {useEffect,useRef,useState,type FormEvent} from 'react';
import {command,desktopAvailable,type FileAsset} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import type {SendStatus,VideoSendAsset} from './Outbox';

export function VideoComposer({accountId,targetId='filehelper',assets,canSend,busy,act,refresh,onDraft,onError}:{
  accountId:string;targetId?:string;assets:FileAsset[];canSend:boolean;busy:boolean;
  act:(action:()=>Promise<unknown>)=>Promise<void>;refresh:()=>Promise<void>;
  onDraft:(id:string)=>Promise<void>;onError:(message:string)=>void;
}) {
  const [selected,setSelected]=useState(''),[file,setFile]=useState<File|null>(null);
  const [preview,setPreview]=useState<VideoSendAsset|null>(null),[draftKey,setDraftKey]=useState(()=>crypto.randomUUID());
  const input=useRef<HTMLInputElement>(null),generation=useRef(0);
  const id=assets.find(a=>a.id===selected)?.id;
  useEffect(()=>{generation.current++;return()=>{generation.current++;};},[accountId]);
  useEffect(()=>{
    let active=true;setPreview(null);
    if(id)void command<VideoSendAsset>('read_video_preview',{accountId,id}).then(p=>{if(active)setPreview(p);},e=>{if(active)onError(e instanceof Error?e.message:String(e));});
    return()=>{active=false;};
  },[accountId,id,onError]);
  async function upload(e:FormEvent) {
    e.preventDefault();if(!file||!file.size||file.size>8*1024*1024){onError('请选择不超过8MiB的MP4视频。');return;}
    const token=generation.current;
    await act(async()=>{
      const bytes=new Uint8Array(await file.arrayBuffer()),values=Array.from(bytes);
      try {
        if(token!==generation.current)return;
        const asset=await command<FileAsset>('upload_video',{accountId,fileName:file.name,bytes:values});
        await refresh();if(token!==generation.current)return;
        setSelected(asset.id);setFile(null);setDraftKey(crypto.randomUUID());if(input.current)input.current.value='';
      }finally{bytes.fill(0);values.fill(0);}
    });
  }
  async function draft() {
    const token=generation.current;let resultId='';
    await act(async()=>{
      const result=await command<SendStatus>('create_video_draft',{accountId,targetId,assetId:selected,idempotencyKey:draftKey});
      if(token===generation.current){resultId=result.metadata.id;setDraftKey(crypto.randomUUID());}
    });
    if(resultId&&token===generation.current)await onDraft(resultId);
  }
  return <details className="outbox-compose"><summary>创建原生视频草稿</summary>
    <div className="button-row"><span className="field">MP4 · 最多8MiB</span><HelpTip label="视频发送说明">支持 H.264 视频与可选 AAC 音轨，1–60秒。导入后加密保存24小时，与附件共用64MiB / 128项额度。保存草稿后可播放预览，再批准一次发送。原生提交成功不代表接收端已确认；结果未知时不会重发。</HelpTip></div>
    <form onSubmit={upload}><label>选择视频<input ref={input} aria-label="发送视频文件" type="file" accept=".mp4,video/mp4" disabled={busy||!desktopAvailable} onChange={e=>setFile(e.target.files?.[0]??null)}/></label><button className="button" disabled={busy||!desktopAvailable||!file}>导入视频</button></form>
    <details><summary>使用已保存的附件</summary><label>选择包含MP4的附件<select aria-label="视频草稿资产" value={id??''} disabled={busy} onChange={e=>{setSelected(e.target.value);setDraftKey(crypto.randomUUID());}}><option value="">选择已保存资产</option>{assets.map(a=><option key={a.id} value={a.id}>{(a.size/1024).toFixed(1)} KiB · {a.id}</option>)}</select></label></details>
    {id&&preview?.file.asset_id===id&&<><p className="file-asset-name">{preview.file.file_name} · {(preview.info.duration_ms/1000).toFixed(2)} 秒 · {preview.info.width} × {preview.info.height}</p><details><summary>更多视频操作</summary><code>{preview.file.sha256}</code><button className="button" disabled={busy} onClick={()=>void act(async()=>{await command('delete_file',{accountId,id});await refresh();setSelected('');})}>删除已导入视频</button></details></>}
    <button className="button" disabled={busy||!canSend||!id||preview?.file.asset_id!==id} onClick={()=>void draft()}>保存视频草稿并预览</button>
  </details>;
}
