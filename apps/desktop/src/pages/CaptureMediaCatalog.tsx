import {useEffect, useRef, useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';

type Position = {page:number; row:number};
type Media = {name:string; kind:string; size:number; mime:string};
type Item = {position:Position; username:string; timestamp:number; media:{row:number; kind:string|null; saved:Media|null; matches_message:boolean; can_fetch:boolean}};
type Catalog = {data:Item[]; next:Position|null};
const kinds:Record<string,string> = {image:'图片',voice:'语音',video:'视频',file:'附件',emoticon:'表情'};

export function CaptureMediaCatalog({accountId,id,onLocate}:{accountId:string; id:string;onLocate:(position:Position)=>void}) {
  const [items,setItems] = useState<Item[]>([]), [next,setNext] = useState<Position|null>(null);
  const [kind,setKind] = useState(''), [saved,setSaved] = useState(''), [username,setUsername] = useState('');
  const [from,setFrom]=useState(''),[until,setUntil]=useState(''),[zoom,setZoom]=useState(1),[savedPath,setSavedPath]=useState('');
  const [busy,setBusy] = useState(false), [error,setError] = useState(''), [notice,setNotice] = useState('');
  const [loaded,setLoaded] = useState(false), [preview,setPreview] = useState<{url:string; mime:string; label:string;position:Position}|null>(null);
  const generation = useRef(0), request = useRef<string|null>(null);
  useEffect(() => {
    setItems([]); setNext(null); setPreview(null); setLoaded(false); setError(''); setNotice(''); setBusy(false);
    return () => {generation.current++; if(request.current) void command('cancel_browse',{requestId:request.current}).catch(()=>{});};
  },[accountId,id]);
  useEffect(() => () => {if(preview) URL.revokeObjectURL(preview.url);},[preview]);
  function filter(change:()=>void) {change(); setItems([]); setNext(null); setLoaded(false); setPreview(null);}
  async function load(after:Position|null=null) {
    const g=generation.current, requestId=crypto.randomUUID(); request.current=requestId;
    setBusy(true); setError('');
    try {
      const result=await command<Catalog>('capture_media_catalog',{accountId,id,after,requestId,query:{kind:kind||null,saved:saved===''?null:saved==='saved',username:username.trim()||null,from:from?Math.floor(new Date(from).getTime()/1000):null,until:until?Math.floor(new Date(until).getTime()/1000):null}});
      if(g===generation.current) {setItems(old=>after?[...old,...result.data]:result.data); setNext(result.next); setLoaded(true);}
    } catch(e) {if(g===generation.current)setError(String(e));}
    finally {if(g===generation.current){setBusy(false);request.current=null;}}
  }
  async function act(item:Item, action:'save'|'preview'|'export'|'saveas') {
    const g=generation.current, requestId=crypto.randomUUID(); request.current=requestId;
    setBusy(true); setError(''); setNotice('');
    const args={accountId,id,index:item.position.page,row:item.position.row,requestId};
    try {
      if(action==='save') {
        const result=await command<Media>('capture_media_save',args);
        if(g===generation.current) setItems(old=>old.map(i=>i===item?{...i,media:{...i.media,saved:result,matches_message:true}}:i));
      } else if(action==='preview') {
        const result=await command<{mime:string;bytes:number[]}>('capture_media_preview',args);
        if(g===generation.current) {
          const url=URL.createObjectURL(new Blob([new Uint8Array(result.bytes)],{type:result.mime}));
          setZoom(1);setPreview({url,mime:result.mime,position:item.position,label:`${item.username} · ${new Date(item.timestamp*1000).toLocaleString()}`});
        }
      } else {
        const name=(item.media.saved?.name??'media').replace(/[<>:"/\\|?*\x00-\x1f]/g,'_');
        let path=`WeRC-${id.slice(0,8)}-${item.position.page}-${item.position.row}-${name}`;
        if(action==='saveas') {const selected=await command<string|null>('choose_media_save_path',{suggestedName:path});if(!selected||g!==generation.current)return;path=selected;}
        await command('capture_media_export',{...args,path});
        if(g===generation.current){setSavedPath(path);setNotice(`已保存：${path}`);}
      }
    } catch(e) {if(g===generation.current)setError(String(e));}
    finally {if(g===generation.current){setBusy(false);request.current=null;}}
  }
  const previewItems=items.filter(i=>i.media.saved&&i.media.saved.kind!=='file');
  const previewIndex=preview?previewItems.findIndex(i=>i.position.page===preview.position.page&&i.position.row===preview.position.row):-1;
  return <details className="local-tools"><summary>媒体总览</summary>
    <div className="actions"><HelpTip label="媒体总览说明">跨归档页浏览图片、语音、视频、附件和表情。已归档的媒体可离线预览或保存；尚未归档的媒体需连接原账号补齐。每次最多显示40项，筛选较少时可继续扫描。附件保存后由对应程序打开。</HelpTip></div>
    <div className="history-query">
      <label>媒体类型<select disabled={busy} value={kind} onChange={e=>filter(()=>setKind(e.target.value))}><option value="">全部类型</option>{Object.entries(kinds).map(([key,name])=><option key={key} value={key}>{name}</option>)}</select></label>
      <label>保存状态<select disabled={busy} value={saved} onChange={e=>filter(()=>setSaved(e.target.value))}><option value="">全部状态</option><option value="saved">已归档</option><option value="missing">待补齐</option></select></label>
      <label>会话账号<input disabled={busy} value={username} placeholder="留空查看所有会话" onChange={e=>filter(()=>setUsername(e.target.value))}/></label>
      <button className="button" disabled={busy} onClick={()=>void load()}>查看媒体</button>
    </div>
    <details><summary>日期范围</summary><div className="history-query"><label>开始时间<input type="datetime-local" disabled={busy} value={from} onChange={e=>filter(()=>setFrom(e.target.value))}/></label><label>结束时间<input type="datetime-local" disabled={busy} value={until} onChange={e=>filter(()=>setUntil(e.target.value))}/></label></div></details>
    {preview&&<section aria-label="媒体预览"><div className="actions"><span>{preview.label}</span><button className="button" disabled={busy||previewIndex<=0} onClick={()=>void act(previewItems[previewIndex-1],'preview')}>上一项媒体</button><button className="button" disabled={busy||previewIndex<0||previewIndex>=previewItems.length-1} onClick={()=>void act(previewItems[previewIndex+1],'preview')}>下一项媒体</button><button className="button" onClick={()=>setPreview(null)}>关闭预览</button></div>
      {preview.mime.startsWith('image/')?<><label>图片缩放 {Math.round(zoom*100)}%<input type="range" aria-label="图片缩放" min="1" max="4" step="0.25" value={zoom} onChange={e=>setZoom(Number(e.target.value))}/></label><div style={{maxHeight:480,overflow:'auto'}}><img src={preview.url} alt="归档媒体" style={{width:`${zoom*100}%`,maxWidth:'none',height:'auto'}}/></div></>:preview.mime.startsWith('audio/')?<audio controls src={preview.url} style={{maxWidth:'100%'}}/>:<video controls src={preview.url} style={{maxWidth:'100%',maxHeight:480}}/>}
    </section>}
    <ul className="recovery-messages">{items.map(item=>{const media=item.media.saved, actual=media?.kind??item.media.kind??'';return <li key={`${item.position.page}:${item.position.row}`}>
      <div>{kinds[actual]} · {item.username} · {new Date(item.timestamp*1000).toLocaleString()}</div>
      <div className="message-body">{media?.name??'尚未归档'}{media?` · ${(media.size/1024).toFixed(1)} KiB`:''}{!item.media.matches_message?' · 对应编辑前原文':''}</div>
      <div className="actions">{media?<>{actual!=='file'&&<button className="button" disabled={busy} onClick={()=>void act(item,'preview')}>预览</button>}<button className="button" disabled={busy} onClick={()=>void act(item,'export')}>保存到下载目录</button><button className="button" disabled={busy} onClick={()=>void act(item,'saveas')}>另存为…</button></>:<button className="button" disabled={busy||!item.media.can_fetch} onClick={()=>void act(item,'save')}>{item.media.can_fetch?'补齐此媒体':'无本机读取来源'}</button>}<button className="button" disabled={busy} onClick={()=>onLocate(item.position)}>查看来源消息页</button></div>
    </li>;})}</ul>
    {loaded&&!items.length&&<p role="status">{next?'当前扫描范围没有匹配项，可继续扫描。':'没有匹配的媒体。'}</p>}
    {next&&<button className="button" disabled={busy} onClick={()=>void load(next)}>继续扫描媒体</button>}
    {busy&&<button className="button" onClick={()=>{if(request.current)void command('cancel_browse',{requestId:request.current}).catch(e=>setError(String(e)));}}>取消当前操作</button>}
    {notice&&<p role="status">{notice}</p>}{savedPath&&<button className="button" onClick={()=>void command('show_directory_export',{path:savedPath}).catch(e=>setError(String(e)))}>查看保存位置</button>}{error&&<p role="alert" className="error-notice">{error}</p>}
  </details>;
}
