import {useRef,useState,type FormEvent} from 'react';
import {command,desktopAvailable,type ImageAsset} from '../lib/desktop';
import {ImagePreview} from './ImagePreview';
import {HelpTip} from '../components/HelpTip';
import type {SendStatus} from './Outbox';

export function ImageComposer({accountId,targetId='filehelper',assets,canSend,busy,act,refresh,onDraft,onError}: {
  accountId:string;targetId?:string;assets:ImageAsset[];canSend:boolean;busy:boolean;
  act:(action:()=>Promise<unknown>)=>Promise<void>;refresh:()=>Promise<void>;
  onDraft:(id:string)=>Promise<void>;onError:(message:string)=>void;
}) {
  const [selected,setSelected]=useState('');
  const [file,setFile]=useState<File|null>(null);
  const [draftKey,setDraftKey]=useState(()=>crypto.randomUUID());
  const input=useRef<HTMLInputElement>(null);
  const current=assets.find(a=>a.id===selected);
  async function upload(e:FormEvent) {
    e.preventDefault();
    if(!file || !file.size || file.size>8*1024*1024){onError('请选择非空且不超过8MiB的PNG或JPEG图片。');return;}
    await act(async()=>{
      const bytes=new Uint8Array(await file.arrayBuffer());const values=Array.from(bytes);
      try {
        const asset=await command<ImageAsset>('upload_image',{accountId,bytes:values});
        await refresh();setSelected(asset.id);setFile(null);setDraftKey(crypto.randomUUID());
        if(input.current)input.current.value='';
      } finally {bytes.fill(0);values.fill(0);}
    });
  }
  async function draft() {
    let id='';
    await act(async()=>{
      const result=await command<SendStatus>('create_image_draft',{accountId,targetId,assetId:selected,idempotencyKey:draftKey});
      id=result.metadata.id;setDraftKey(crypto.randomUUID());
    });
    if(id)await onDraft(id);
  }
  return <details className="outbox-compose"><summary>创建原生图片草稿</summary>
    <div className="button-row"><span>选择图片，预览后保存草稿</span><HelpTip label="图片导入说明">支持静态PNG和JPEG照片，每张最多8MiB、边长4096像素、总面积4194304像素。JPEG按照片方向转正后转换为PNG，移除原始元数据；请以预览为准。导入后在本机加密保存24小时，保存草稿不会发送，仍需在队列预览并批准。</HelpTip></div>
    <form onSubmit={upload}><label>选择图片<input ref={input} aria-label="图片文件" type="file" accept=".png,.jpg,.jpeg,image/png,image/jpeg" disabled={busy||!desktopAvailable} onChange={e=>setFile(e.target.files?.[0]??null)}/></label><button className="button" disabled={busy||!desktopAvailable||!file}>导入图片</button></form>
    <label>已导入图片<select aria-label="图片草稿图片" value={current?.id??''} disabled={busy} onChange={e=>{setSelected(e.target.value);setDraftKey(crypto.randomUUID());}}><option value="">选择图片</option>{assets.map(a=><option value={a.id} key={a.id}>{a.image.width}×{a.image.height} / {a.id}</option>)}</select></label>
    {current&&<><ImagePreview key={current.id} accountId={accountId} assetId={current.id} onError={onError}/><details><summary>图片摘要</summary><code>{current.sha256}</code></details>
      <button className="button" disabled={busy} onClick={()=>void act(async()=>{await command('delete_image',{accountId,id:current.id});await refresh();setSelected('');})}>删除图片</button></>}
    <button className="button" disabled={busy||!canSend||!current} onClick={()=>void draft()}>保存图片草稿并预览</button>
  </details>;
}
