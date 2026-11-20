import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import type {MomentDetailsData} from './MomentDetails';
export interface MomentAnchor {id:string;row_id:string;username:string}
type Variant='original'|'thumbnail'|'live_video';
interface Manifest {id:string;file_name:string;mime:string;size:number;sha256:string;source_md5_matches:boolean|null}
export function MomentGallery({accountId,viewId,anchor,media}:{accountId:string;viewId?:string;anchor:MomentAnchor;media:MomentDetailsData['media']}) {
  const [index,setIndex]=useState(0),[variant,setVariant]=useState<Variant>('original'),[busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const [preview,setPreview]=useState<{url:string;manifest:Manifest}|null>(null);
  const generation=useRef(0),request=useRef<string|null>(null),url=useRef<string|null>(null),running=useRef(false);
  function clear(){generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});request.current=null;running.current=false;if(url.current)URL.revokeObjectURL(url.current);url.current=null;setPreview(null);setBusy(false);setError('');setNotice('');}
  useEffect(()=>{clear();setIndex(0);setVariant('original');return()=>{generation.current++;running.current=false;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});request.current=null;if(url.current)URL.revokeObjectURL(url.current);url.current=null;};},[accountId,viewId,anchor.id,anchor.username,anchor.row_id]);
  if(!media.length)return null;
  async function load(){
    if(running.current)return;running.current=true;const g=++generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');
    try{
      const manifest=await command<Manifest>('native_moment_media',{accountId,viewId:viewId??null,selection:{anchor,slot:media[index].slot,variant},requestId});if(g!==generation.current)return;
      const p=await command<{mime:string;bytes:number[]}>('moment_media_preview',{accountId,id:manifest.id});if(g!==generation.current){p.bytes.fill(0);return;}
      const bytes=new Uint8Array(p.bytes);p.bytes.fill(0);const next=URL.createObjectURL(new Blob([bytes],{type:p.mime}));bytes.fill(0);if(url.current)URL.revokeObjectURL(url.current);url.current=next;setPreview({url:next,manifest});setNotice('已载入加密副本，下次可离线查看。');
    }catch(e){if(g===generation.current)setError(String(e));}finally{if(g===generation.current){running.current=false;request.current=null;setBusy(false);}}
  }
  async function save(){if(!preview||running.current)return;running.current=true;setBusy(true);setError('');const g=generation.current;try{const path=await command<string|null>('choose_media_save_path',{suggestedName:preview.manifest.file_name});if(!path||g!==generation.current)return;await command('save_moment_media',{accountId,id:preview.manifest.id,path});if(g===generation.current)setNotice('媒体已保存。');}catch(e){if(g===generation.current)setError(String(e));}finally{if(g===generation.current){running.current=false;setBusy(false);}}}
  return <details className="moment-gallery" onToggle={e=>{if(!e.currentTarget.open)clear();}}><summary>查看图片与视频（{media.length}）</summary>
    <div className="actions"><button className="button" disabled={busy||index===0} onClick={()=>{clear();setIndex(i=>i-1);setVariant('original');}}>上一项</button><span role="status">{index+1} / {media.length}{media[index].live_photo?' · 实况图':''}</span><button className="button" disabled={busy||index+1===media.length} onClick={()=>{clear();setIndex(i=>i+1);setVariant('original');}}>下一项</button><HelpTip label="朋友圈媒体说明">点击载入后读取所选媒体，并加密保存在本机。已保存的内容可离线查看和另存；视频不会自动播放。资源过期或格式暂不支持时会显示原因。实况图可分别选择静态图片和动态片段。保存的是此来源实际返回并完整解析的内容，可能与手机原图尺寸不同。</HelpTip></div>
    <div className="actions"><label>内容<select value={variant} disabled={busy} onChange={e=>{clear();setVariant(e.target.value as Variant);}}><option value="original">图片或视频</option><option value="thumbnail">缩略图</option>{media[index].live_photo&&<option value="live_video">实况动态片段</option>}</select></label><button className="button" disabled={busy} onClick={()=>void load()}>{busy?'正在载入…':viewId?'载入所选媒体':'查看已保存媒体'}</button>{busy&&request.current&&<button className="button" onClick={clear}>取消载入</button>}{preview&&<button className="button" disabled={busy} onClick={()=>void save()}>另存媒体…</button>}</div>
    {preview&&(preview.manifest.mime==='video/mp4'?<video src={preview.url} controls preload="metadata" onError={()=>setError("当前系统播放器无法解码这段视频。加密原文件已保留，可点“另存媒体”后用支持该编码的播放器打开。")} style={{maxWidth:'100%',maxHeight:520}}/>:<img src={preview.url} alt={`朋友圈第${index+1}项媒体`} style={{maxWidth:'100%',maxHeight:520,objectFit:'contain'}}/>)}
    {preview&&<details><summary>副本信息</summary><p className="message-body">{preview.manifest.file_name} · {preview.manifest.size.toLocaleString()} 字节</p><code className="message-body">{preview.manifest.sha256}</code>{preview.manifest.source_md5_matches===false&&<p className="field">此来源可能经过尺寸或格式转换，与缓存所记原资源摘要不同。副本已独立计算摘要并在回读时核验。</p>}</details>}
    {notice&&<p role="status">{notice}</p>}{error&&<p role="alert" className="error-notice">{error}</p>}
  </details>;
}
