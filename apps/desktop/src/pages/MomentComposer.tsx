import {useEffect,useRef,useState,type FormEvent} from 'react';
import {command,desktopAvailable,type ImageAsset,type FileAsset} from '../lib/desktop';
import {validSendText,textByteLength,MAX_TEXT_BYTES} from '../lib/send-text';
import {HelpTip} from '../components/HelpTip';
import type {SendStatus} from './Outbox';
import {videoCover} from '../lib/video-cover';
import {VideoPreview} from './VideoPreview';
import {ImagePreview} from './ImagePreview';

export function MomentComposer({accountId,enabled=true,onDraft}:{accountId:string;enabled?:boolean;onDraft?:(id:string)=>Promise<void>}) {
  const [text,setText]=useState(''),[busy,setBusy]=useState(false),[saved,setSaved]=useState(false),[error,setError]=useState('');
  const [visibility,setVisibility]=useState<'private'|'public'>('private');
  const [images,setImages]=useState<ImageAsset[]>([]);
  const [mode,setMode]=useState<'images'|'video'>('images'),[video,setVideo]=useState<FileAsset|null>(null);
  const fileInput=useRef<HTMLInputElement>(null);
  const key=useRef(crypto.randomUUID()),running=useRef(false),alive=useRef(true);
  useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
  const valid=(validSendText(text)||(text===''&&images.length>0))&&(mode!=='video'||(video!==null&&images.length===1));
  function changed(){key.current=crypto.randomUUID();setSaved(false);}
  async function addImages(files:File[]){
    if(running.current||!enabled||!files.length)return;
    if(images.length+files.length>(mode==='video'?1:9)){setError(mode==='video'?'视频仅需一张封面，可先移除已有封面。':'最多选择9张图片，可先移除已选图片。');return;}
    if(files.some(f=>!f.size||f.size>8*1024*1024)){setError('每张图片需为非空且不超过8MiB的PNG或JPEG图片。');return;}
    running.current=true;setBusy(true);setError('');changed();
    try{
      for(const file of files){
        if(!alive.current)break;
        const bytes=new Uint8Array(await file.arrayBuffer()),values=Array.from(bytes);
        try{const asset=await command<ImageAsset>('upload_image',{accountId,bytes:values});if(alive.current)setImages(current=>[...current,asset]);}
        finally{bytes.fill(0);values.fill(0);}
      }
    }catch(e){if(alive.current)setError(e instanceof Error?e.message:String(e));}
    finally{running.current=false;if(alive.current){setBusy(false);if(fileInput.current)fileInput.current.value='';}}
  }
  async function addVideo(file?:File){
    if(!file||running.current||!enabled)return;
    if(!file.size||file.size>8*1024*1024){setError('请选择不超过8MiB的MP4视频。');return;}
    running.current=true;setBusy(true);setError('');changed();setVideo(null);setImages([]);
    let bytes:Uint8Array<ArrayBuffer>|undefined,cover:Uint8Array<ArrayBuffer>|undefined;
    let values:number[]=[],coverValues:number[]=[];
    try{
      cover=await videoCover(file);bytes=new Uint8Array(await file.arrayBuffer());
      if(!alive.current)return;
      values=Array.from(bytes);coverValues=Array.from(cover);
      const asset=await command<FileAsset>('upload_video',{accountId,fileName:file.name,bytes:values});
      if(!alive.current)return;
      const poster=await command<ImageAsset>('upload_image',{accountId,bytes:coverValues});
      if(alive.current){setVideo(asset);setImages([poster]);}
    }catch(e){if(alive.current)setError(e instanceof Error?e.message:String(e));}
    finally{bytes?.fill(0);cover?.fill(0);values.fill(0);coverValues.fill(0);running.current=false;if(alive.current)setBusy(false);}
  }
  function move(index:number,offset:number){
    setImages(current=>{const next=[...current],other=index+offset;if(other>=0&&other<next.length)[next[index],next[other]]=[next[other],next[index]];return next;});changed();
  }
  async function save(event:FormEvent){
    event.preventDefault();if(running.current||!enabled||!valid)return;
    running.current=true;setBusy(true);setError('');setSaved(false);
    try{const draft=await command<SendStatus>('create_moment_draft',{accountId,text,visibility,videoAssetId:mode==='video'?video?.id:null,imageAssetIds:images.map(image=>image.id),idempotencyKey:key.current});
      if(!alive.current)return;setSaved(true);if(onDraft)await onDraft(draft.metadata.id);
    }catch(e){if(alive.current)setError(e instanceof Error?e.message:String(e));}
    finally{running.current=false;if(alive.current)setBusy(false);}
  }
  return <details className="outbox-compose moment-compose"><summary>写一条朋友圈动态</summary>
    <form onSubmit={e=>void save(e)}>
      <div className="button-row"><strong>朋友圈</strong><HelpTip label="朋友圈发布说明">先填写正文或选择图片并保存草稿，再到发送队列预览发布账号、完整正文、图片顺序和可见范围，勾选后批准一次。保存不会发布。支持公开或仅自己可见的文字、最多9张PNG或JPEG图片，或一个H.264 MP4视频。服务器已接受表示收到匹配的原生完成事件，手机显示仍需单独核验。结果未知时只回查，不重发。</HelpTip></div>
      <label>谁可以看<select aria-label="动态可见范围" value={visibility} disabled={busy||!enabled||!desktopAvailable} onChange={e=>{setVisibility(e.target.value as 'private'|'public');changed();}}><option value="private">仅自己可见</option><option value="public">公开 · 朋友可见</option></select></label>
      <label>动态内容<select aria-label="动态内容类型" value={mode} disabled={busy||!enabled} onChange={e=>{setMode(e.target.value as 'images'|'video');setImages([]);setVideo(null);changed();}}><option value="images">文字 / 图片</option><option value="video">视频</option></select></label>
      {mode==='video'&&<div><label>选择视频<input type="file" accept=".mp4,video/mp4" aria-label="动态视频文件" disabled={busy||!enabled||!desktopAvailable} onChange={e=>void addVideo(e.target.files?.[0])}/></label><HelpTip label="动态视频说明">H.264 MP4，1–60秒、最多8MiB。选择视频后会自动提取封面；展开封面可移除并改选图片。视频与封面一起加密保存，预览后批准才会发布。</HelpTip>{video&&<VideoPreview accountId={accountId} assetId={video.id} onError={setError}/>}</div>}
      <label>动态正文<textarea aria-label="动态正文" value={text} disabled={busy||!enabled||!desktopAvailable} onChange={e=>{setText(e.target.value);changed();}}/></label>
      <details className="moment-image-picker"><summary>{mode==='video'?'视频封面':'添加图片（可选）'}{images.length>0&&` · ${images.length}/${mode==='video'?1:9}`}</summary>
        <div className="button-row"><label>选择图片<input ref={fileInput} type="file" multiple accept=".png,.jpg,.jpeg,image/png,image/jpeg" aria-label="动态图片文件" disabled={busy||!enabled||!desktopAvailable||images.length>=(mode==='video'?1:9)} onChange={e=>void addImages(Array.from(e.target.files??[]))}/></label><HelpTip label="动态图片说明">最多9张PNG或JPEG图片，每张不超过8MiB、边长4096像素、总面积4194304像素。JPEG会按照片方向转正并移除元数据，以预览为准。选择后会在本机加密保存24小时；有图片时正文可以留空。下方顺序就是发布顺序，移除仅取消本次选择，已导入缓存仍按到期时间清理。</HelpTip></div>
        <ol className="moment-image-grid">{images.map((image,index)=><li key={`${index}/${image.id}`}>
          <strong>第 {index+1} 张 · {image.image.width}×{image.image.height}</strong><ImagePreview accountId={accountId} assetId={image.id} onError={setError}/>
          <div className="button-row"><button type="button" className="button" aria-label={`上移第${index+1}张图片`} disabled={busy||index===0} onClick={()=>move(index,-1)}>上移</button><button type="button" className="button" aria-label={`下移第${index+1}张图片`} disabled={busy||index===images.length-1} onClick={()=>move(index,1)}>下移</button><button type="button" className="button" aria-label={`移除第${index+1}张图片`} disabled={busy} onClick={()=>{setImages(current=>current.filter((_,i)=>i!==index));changed();}}>移除</button></div>
        </li>)}</ol>
      </details>
      <div className="button-row"><span className="field" role="status">{textByteLength(text)} / {MAX_TEXT_BYTES} 字节</span><button className="button" disabled={!enabled||!desktopAvailable||busy||!valid||saved}>{busy?'正在处理…':onDraft?'保存动态并预览':'保存到发送队列'}</button></div>
      {!enabled&&<p className="field">连接支持的微信版本后可创建动态。</p>}
      {saved&&<p role="status">草稿已保存{onDraft?'，请在下方核对内容。':'，请在左侧“发送队列”中预览和批准。'}</p>}
      {error&&<p className="error-notice" role="alert">{error}</p>}
    </form>
  </details>;
}
