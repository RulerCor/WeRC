import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
export type MediaKind='image'|'voice'|'video'|'file'|'emoticon';
export interface MediaHint {local_id:string;source:string|null;kind:MediaKind}
interface MediaRef {kind:MediaKind;id:string;proof:string}
interface Asset {reference:MediaRef;size:number;sha256:string}
const labels:Record<MediaKind,string>={image:'图片',voice:'语音',video:'视频',file:'附件',emoticon:'表情'};
export function ConversationMedia({accountId,viewId,hint,forwardIndex,forwardPath}:{accountId:string;viewId:string;hint:MediaHint;forwardIndex?:number;forwardPath?:number[]}){
 const [asset,setAsset]=useState<Asset|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[url,setUrl]=useState(''),[path,setPath]=useState(''),[notice,setNotice]=useState('');
 const forwardKey=forwardPath?.join('.');
 const generation=useRef(0),request=useRef<string|null>(null),blob=useRef(''),pending=useRef(false);
 function stop(){if(blob.current)URL.revokeObjectURL(blob.current);blob.current='';setUrl('');}
 useEffect(()=>()=>{generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});if(blob.current)URL.revokeObjectURL(blob.current);},[accountId,viewId,hint.local_id,hint.source,forwardIndex,forwardKey]);
 async function perform(work:()=>Promise<void>){if(pending.current)return;pending.current=true;setBusy(true);setError('');setNotice('');const g=generation.current;try{await work();}catch(e){if(g===generation.current)setError(e instanceof Error?e.message:String(e));}finally{pending.current=false;if(g===generation.current)setBusy(false);}}
 async function save(){const g=generation.current,requestId=crypto.randomUUID();request.current=requestId;try{
  const result=await command<Asset>('native_conversation_media_save',{accountId,selection:{view_id:viewId,local_id:hint.local_id,source:hint.source},forwardIndex:forwardIndex??null,forwardPath:forwardPath??null,requestId});
  if(g===generation.current)setAsset(result);
 }finally{if(request.current===requestId)request.current=null;}}
 async function preview(){if(!asset)return;const g=generation.current;
  const result=await command<ArrayBuffer|number[]>('native_conversation_media_content',{accountId,reference:asset.reference});if(g!==generation.current)return;
  const bytes=result instanceof ArrayBuffer?new Uint8Array(result):new Uint8Array(result);
  const mime=hint.kind==='voice'?'audio/wav':hint.kind==='video'?'video/mp4':bytes[0]===71&&bytes[1]===73&&bytes[2]===70?'image/gif':'image/png';
  stop();blob.current=URL.createObjectURL(new Blob([bytes],{type:mime}));setUrl(blob.current);
 }
 return <div className="conversation-media">
  <div className="actions">{!asset?<button className="button" disabled={busy} onClick={()=>void perform(save)}>{busy?'正在读取…':`读取${labels[hint.kind]}`}</button>:<>
   <span className="state-tag">已加密保存 · {Math.ceil(asset.size/1024)} KiB</span>
   {hint.kind!=='file'&&(url?<button className="button" onClick={stop}>停止预览</button>:<button className="button" disabled={busy} onClick={()=>void perform(preview)}>{busy?'正在准备…':`预览${labels[hint.kind]}`}</button>)}
  </>}
  <HelpTip label={`${labels[hint.kind]}读取说明`}>点击后读取所选消息对应的媒体并加密保存24小时。{forwardPath&&hint.kind==='file'?'此合并记录附件仅读取当前账号的微信本地缓存，并核对完整大小和摘要；未缓存时请先在微信打开该附件。':forwardPath||forwardIndex!==undefined?'按当前展开层级选择该图片原图，完整大小与摘要验证后保存；可能需要微信联网下载。':'语音和表情读取微信本地缓存；图片、视频和普通附件可能需要微信联网下载。'}读取不发送消息，也不改变微信已读状态。预览需单独打开，附件仅供导出。关闭会话后可在媒体管理中清理文件和语音。</HelpTip></div>
  {url&&(hint.kind==='voice'?<audio controls preload="metadata" src={url} aria-label="会话语音预览"/>:hint.kind==='video'?<video controls preload="metadata" src={url} aria-label="会话视频预览"/>:<img src={url} alt={`会话${labels[hint.kind]}预览`}/>)}
  {asset&&<details className="media-management"><summary>导出与清理</summary><label>导出文件名<input value={path} onChange={e=>setPath(e.target.value)} placeholder="留空使用原文件名，保存到系统下载目录"/></label><div className="actions">
   <button className="button" disabled={busy} onClick={()=>void perform(async()=>{const g=generation.current;const bytes=await command<number>('export_conversation_media',{accountId,reference:asset.reference,path});if(g===generation.current)setNotice(`已导出 ${bytes} 字节`);})}>导出</button>
   <button className="button" disabled={busy} onClick={()=>void perform(async()=>{const g=generation.current;await command('native_conversation_media_delete',{accountId,reference:asset.reference});if(g===generation.current){stop();setAsset(null);setNotice('已删除本地加密副本');}})}>删除本地副本</button>
  </div></details>}
  {notice&&<p role="status">{notice}</p>}{error&&<p className="error-notice" role="alert">{error}</p>}
 </div>;
}
