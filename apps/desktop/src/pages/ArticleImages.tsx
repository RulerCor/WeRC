import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
interface Cached{id:string;size:number;mime:string;extension:string}
export function ArticleImages({accountId,id,count}:{accountId:string;id:string;count:number}){
 const [cached,setCached]=useState<(Cached|null)[]>([]),[busy,setBusy]=useState(false),[errors,setErrors]=useState<string[]>([]),[progress,setProgress]=useState(0),[preview,setPreview]=useState<string|null>(null);
 const busyRef=useRef(false),previewGeneration=useRef(0);
 const generation=useRef(0),stopped=useRef(false),request=useRef<string|null>(null),blob=useRef<string|null>(null);
 const action=<T,>(operation:object)=>command<T>('article_action',{accountId,operation,requestId:request.current??crypto.randomUUID()});
 useEffect(()=>{const gen=++generation.current;busyRef.current=false;setBusy(false);setCached([]);setPreview(null);setProgress(0);setErrors([]);void action<(Cached|null)[]>({action:'images',id}).then(v=>{if(gen===generation.current)setCached(old=>v.map((m,i)=>old[i]??m));}).catch(e=>{if(gen===generation.current)setErrors([String(e)]);});return()=>{generation.current++;stopped.current=true;if(blob.current)URL.revokeObjectURL(blob.current);if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});};},[accountId,id]);
 async function save(){if(busyRef.current)return;busyRef.current=true;const gen=generation.current;setBusy(true);setProgress(0);setErrors([]);stopped.current=false;let completed=0;try{
  for(let index=0;index<count;index++){
   if(stopped.current||gen!==generation.current)break;
   if(cached[index]){setProgress(++completed);continue;}
   request.current=crypto.randomUUID();
   try{const m=await action<Cached>({action:'cache_image',id,index});if(gen===generation.current){setCached(old=>{const n=[...old];n[index]=m;return n;});setProgress(++completed);}}
   catch(e){if(gen===generation.current&&!stopped.current)setErrors(old=>[...old,`图片 ${index+1}：${String(e)}`]);}
   finally{request.current=null;}
  }
 }finally{if(gen===generation.current){busyRef.current=false;setBusy(false);}}}
 async function show(image:Cached){const gen=generation.current,previewGen=++previewGeneration.current;try{const r=await action<{image:Cached;hex:string}>({action:'image',id:image.id});if(gen!==generation.current||previewGen!==previewGeneration.current)return;if(r.image.id!==image.id||r.hex.length!==r.image.size*2||!Number.isSafeInteger(r.image.size)||r.image.size<=0||r.image.size>8*1024*1024||!/^[a-f0-9]+$/i.test(r.hex))throw Error('图片内容不完整');const data=new Uint8Array(r.image.size);for(let i=0;i<data.length;i++)data[i]=parseInt(r.hex.slice(i*2,i*2+2),16);if(blob.current)URL.revokeObjectURL(blob.current);blob.current=URL.createObjectURL(new Blob([data],{type:r.image.mime}));setPreview(blob.current);}catch(e){if(gen===generation.current)setErrors(old=>[...old,String(e)]);}}
 return <details><summary>离线图片 · 已保存 {cached.filter(Boolean).length}/{count}</summary><div className="actions"><button className="button" disabled={busy} onClick={()=>void save()}>保存缺少的图片</button>{busy&&<><span role="status">已处理 {progress}/{count}</span><button className="button" onClick={()=>{stopped.current=true;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});}}>停止保存</button></>}<HelpTip label="离线图片说明">已完成的图片会加密保存，停止或关闭后仍保留。再次保存只处理缺少的图片。ZIP 导出会包含已保存图片并列出缺失项。支持已解码校验的 PNG、JPEG、GIF 和 WebP；其他格式失败会显示原因。</HelpTip></div><div className="actions">{cached.map((m,i)=>m&&<button key={m.id} className="button" disabled={busy} onClick={()=>void show(m)}>查看图片 {i+1}</button>)}</div>{preview&&<figure><img src={preview} alt="已保存的文章图片" style={{maxWidth:'100%',height:'auto'}}/><button className="button" onClick={()=>{previewGeneration.current++;if(blob.current)URL.revokeObjectURL(blob.current);blob.current=null;setPreview(null);}}>关闭图片</button></figure>}{errors.length>0&&<details open><summary>未完成项（{errors.length}）</summary><ul>{errors.map((e,i)=><li key={i} role="alert">{e}</li>)}</ul></details>}</details>;
}
