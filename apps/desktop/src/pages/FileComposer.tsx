import {useEffect,useRef,useState,type FormEvent} from 'react';
import {command,desktopAvailable,type FileAsset,type FilePreview} from '../lib/desktop';
import type {SendStatus} from './Outbox';

export function FileComposer({accountId,targetId='filehelper',assets,canSend,busy,act,refresh,onDraft,onError}: {
  accountId:string;targetId?:string;assets:FileAsset[];canSend:boolean;busy:boolean;
  act:(action:()=>Promise<unknown>)=>Promise<void>;refresh:()=>Promise<void>;
  onDraft:(id:string)=>Promise<void>;onError:(message:string)=>void;
}) {
  const [selected,setSelected]=useState('');
  const [preview,setPreview]=useState<FilePreview|null>(null);
  const [file,setFile]=useState<File|null>(null);
  const [draftKey,setDraftKey]=useState(()=>crypto.randomUUID());
  const input=useRef<HTMLInputElement>(null);
  const current=assets.find(a=>a.id===selected);
  const id=current?.id;
  useEffect(()=>{
    let active=true;setPreview(null);
    if(id)void command<FilePreview>('read_file_preview',{accountId,id}).then(p=>{if(active)setPreview(p);},e=>{if(active)onError(e instanceof Error?e.message:'无法读取文件信息。');});
    return ()=>{active=false;};
  },[accountId,id,onError]);
  async function upload(e:FormEvent) {
    e.preventDefault();
    if(!file||!file.size||file.size>8*1024*1024){onError('请选择非空且不超过8MiB的文件。');return;}
    await act(async()=>{
      const bytes=new Uint8Array(await file.arrayBuffer()),values=Array.from(bytes);
      try {
        const asset=await command<FileAsset>('upload_file',{accountId,fileName:file.name,bytes:values});
        await refresh();setSelected(asset.id);setFile(null);setDraftKey(crypto.randomUUID());
        if(input.current)input.current.value='';
      } finally {bytes.fill(0);values.fill(0);}
    });
  }
  async function draft() {
    let resultId='';
    await act(async()=>{
      const result=await command<SendStatus>('create_file_draft',{accountId,targetId,assetId:selected,idempotencyKey:draftKey});
      resultId=result.metadata.id;setDraftKey(crypto.randomUUID());
    });
    if(resultId)await onDraft(resultId);
  }
  return <details className="outbox-compose"><summary>创建原生文件草稿</summary>
    <p className="field">最多8MiB。文件名与内容加密保存24小时，发送前逐条核对名称、大小和摘要。</p>
    <form onSubmit={upload}><label>文件<input ref={input} aria-label="附件文件" type="file" disabled={busy||!desktopAvailable} onChange={e=>setFile(e.target.files?.[0]??null)}/></label><button className="button" disabled={busy||!desktopAvailable||!file}>导入文件</button></form>
    <label>已导入文件<select aria-label="文件草稿附件" value={id??''} disabled={busy} onChange={e=>{setSelected(e.target.value);setDraftKey(crypto.randomUUID());}}><option value="">选择文件</option>{assets.map(a=><option value={a.id} key={a.id}>{a.size}字节 / {a.id}</option>)}</select></label>
    {current&&preview?.asset.id===current.id&&<><p className="file-asset-name">{preview.file_name} / {current.size}字节</p><details><summary>文件摘要</summary><code>{current.sha256}</code></details><button className="button" disabled={busy} onClick={()=>void act(async()=>{await command('delete_file',{accountId,id:current.id});await refresh();setSelected('');})}>删除文件</button></>}
    <button className="button" disabled={busy||!canSend||!current||preview?.asset.id!==current.id} onClick={()=>void draft()}>保存文件草稿并预览</button>
  </details>;
}
