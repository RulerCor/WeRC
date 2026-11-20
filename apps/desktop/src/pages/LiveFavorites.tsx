import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {safeTime} from '../lib/browse';
type Block={slot:number[];kind:number;title:string;text:string;format:string;size:number|null;duration:number|null;source_name:string;source_time:string;media_available:boolean};
type Content={title:string;text:string;tags:string[];url:string|null;author:string;blocks:Block[]};
type Item={anchor:{updated:number;local_id:string};server_id:string;kind:number;source_user:string|null;conversation:string|null;content_status:string;content:Content|null;item_token:string};
type Query={kind:number|null;text:string;tag:string|null;from:number|null;until:number|null};
type Page={rows:Item[];counts:Record<string,number>;next:string|null;query:Query;scanned:number;expires_in_seconds:number};
const kinds:Record<string,string>={'1':'文字','2':'图片','3':'语音','4':'视频','5':'链接','6':'位置','7':'音乐','8':'文件','14':'聊天记录','18':'笔记','20':'视频号','21':'视频号直播'};
export function LiveFavorites({accountId,viewId}:{accountId:string;viewId:string}){
  const [page,setPage]=useState<Page|null>(null),[index,setIndex]=useState(0),[busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const [text,setText]=useState(''),[kind,setKind]=useState(''),[tag,setTag]=useState(''),[from,setFrom]=useState(''),[until,setUntil]=useState('');
  const [preview,setPreview]=useState<{url:string;mime:string}|null>(null),[savedPath,setSavedPath]=useState('');
  const generation=useRef(0),request=useRef<string|null>(null),cursors=useRef<(string|null)[]>([null]),expires=useRef(0),frozen=useRef<Query|null>(null);
  useEffect(()=>{
    clear();setError('');setSavedPath('');cursors.current=[null];frozen.current=null;
    return()=>{generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});request.current=null;};
  },[accountId,viewId]);
  useEffect(()=>()=>{if(preview)URL.revokeObjectURL(preview.url);},[preview]);
  useEffect(()=>{if(!page)return;const timer=window.setInterval(()=>{if(Date.now()>=expires.current){clear();setError('来源窗口已过期，请重新准备消息来源。');}},1000);return()=>clearInterval(timer);},[page]);
  function clear(){generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});request.current=null;setBusy(false);setPage(null);setPreview(null);setIndex(0);setNotice('');}
  function change(action:()=>void){clear();action();}
  async function read(at=0,cursor:string|null=null){
    if(request.current)return;const g=++generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');setPreview(null);
    const query:Query={kind:kind?Number(kind):null,text:text.trim(),tag:tag.trim()||null,from:from?Math.floor(new Date(from).getTime()/1000):null,until:until?Math.floor(new Date(until).getTime()/1000):null};
    if(at===0)frozen.current=query;
    try{const result=await command<Page>('native_live_favorites',{accountId,viewId,query:cursor?null:frozen.current,cursor,requestId});if(g!==generation.current)return;
      cursors.current=cursors.current.slice(0,at+1);cursors.current[at]=cursor;expires.current=Date.now()+result.expires_in_seconds*1000;setPage(result);setIndex(at);
    }catch(e){if(g===generation.current){setPage(null);setError(String(e));}}finally{if(g===generation.current){request.current=null;setBusy(false);}}
  }
  async function act(item:Item,action:'copy'|'preview'|'save',block?:Block){
    if(request.current)return;const g=generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');setNotice('');
    const args={accountId,viewId,itemToken:item.item_token,requestId,slot:block?.slot};
    try{
      if(action==='copy'){await command('native_favorite_copy',args);if(g===generation.current)setNotice('正文和链接已复制到资料库的本地收藏。');}
      else if(action==='preview'){
        const result=await command<{mime:string;bytes:number[]}>('native_favorite_preview',args);
        if(g===generation.current)setPreview({url:URL.createObjectURL(new Blob([new Uint8Array(result.bytes)],{type:result.mime})),mime:result.mime});
      }else{
        const extension=block?.kind===2?'png':block?.kind===3?'wav':block?.kind===4?'mp4':block?.format||'bin';
        const file=block?.kind===8&&block.title?block.title:`收藏-${item.anchor.local_id}-${block?.slot.join('-')}.${extension}`;
        const suggestedName=file.replace(/[<>:"/\\|?*\x00-\x1f]/g,'_');
        const path=await command<string|null>('choose_media_save_path',{suggestedName});if(!path||g!==generation.current)return;
        const size=await command<number>('native_favorite_save',{...args,path});if(g===generation.current){setSavedPath(path);setNotice(`已保存 ${size.toLocaleString()} 字节。`);}
      }
    }catch(e){if(g===generation.current)setError(String(e));}finally{if(g===generation.current){request.current=null;setBusy(false);}}
  }
  return <section aria-label="微信收藏">
    <div className="actions"><h3>微信收藏</h3><HelpTip label="微信收藏使用说明">读取当前微信已同步到这台电脑的收藏。先点击查看，再展开条目；正文可复制到本地资料库，图片、视频和语音按需预览，文件可另存为。类型、文字、标签和更新时间可组合筛选。文字只搜索可显示正文，不搜索下载凭据。每页最多30条、每次扫描200项，结果为空时仍可能需要继续扫描。这里不会修改微信收藏，未同步内容需先由微信同步。</HelpTip></div>
    <form className="history-query" onSubmit={e=>{e.preventDefault();void read();}}>
      <label>搜索收藏<input disabled={busy} value={text} maxLength={128} placeholder="标题或正文" onChange={e=>change(()=>setText(e.target.value))}/></label>
      <label>类型<select disabled={busy} value={kind} onChange={e=>change(()=>setKind(e.target.value))}><option value="">全部类型</option>{Object.entries(kinds).map(([id,label])=><option key={id} value={id}>{label}{page?.counts[id]!=null?` (${page.counts[id]})`:''}</option>)}{page&&Object.keys(page.counts).filter(id=>!kinds[id]).map(id=><option key={id} value={id}>类型{id} ({page.counts[id]})</option>)}</select></label>
      <button className="button primary" disabled={busy}>查看收藏</button>
    </form>
    <details><summary>更多筛选</summary><div className="history-query"><label>标签<input disabled={busy} value={tag} maxLength={64} onChange={e=>change(()=>setTag(e.target.value))}/></label><label>更新起始<input type="datetime-local" disabled={busy} value={from} onChange={e=>change(()=>setFrom(e.target.value))}/></label><label>更新截止<input type="datetime-local" disabled={busy} value={until} onChange={e=>change(()=>setUntil(e.target.value))}/></label></div></details>
    {preview&&<section aria-label="收藏媒体预览"><button className="button" onClick={()=>setPreview(null)}>关闭预览</button>{preview.mime==='image/png'?<img src={preview.url} alt="微信收藏图片" style={{maxWidth:'100%',maxHeight:480}}/>:preview.mime.startsWith('audio/')?<audio controls src={preview.url} style={{maxWidth:'100%'}}/>:<video controls src={preview.url} style={{maxWidth:'100%',maxHeight:480}}/>}</section>}
    {page&&<><ul className="recovery-messages">{page.rows.map(item=><li key={item.anchor.local_id}><details>
      <summary><strong dir="auto">{item.content?.title||item.content?.text.slice(0,80)||kinds[String(item.kind)]||`类型${item.kind}`}</strong><span className="field"> · {safeTime(item.anchor.updated)}</span></summary>
      {item.content?<><p className="message-body" dir="auto">{item.content.text}</p>{item.content.tags.length>0&&<p className="field">标签：{item.content.tags.join('、')}</p>}{item.content.author&&<p>{item.content.author}</p>}
        {item.content.url&&<div className="actions"><button className="button" onClick={()=>void navigator.clipboard.writeText(item.content!.url!).then(()=>setNotice('链接已复制。')).catch(e=>setError(String(e)))}>复制链接</button><span className="message-body">{item.content.url}</span></div>}
        {item.content.blocks.map(b=><div key={b.slot.join('-')} className="message-bubble"><strong>{b.title||kinds[String(b.kind)]||`类型${b.kind}`}</strong><p className="message-body" dir="auto">{b.text}</p>{b.size!=null&&<span className="field">{b.size.toLocaleString()} 字节</span>}{b.media_available&&[2,3,4,8].includes(b.kind)&&<div className="actions">{b.kind!==8&&<button className="button" disabled={busy} onClick={()=>void act(item,'preview',b)}>预览媒体</button>}<button className="button" disabled={busy} onClick={()=>void act(item,'save',b)}>媒体另存为…</button></div>}</div>)}
        <button className="button" disabled={busy} onClick={()=>void act(item,'copy')}>复制正文到本地收藏</button>
      </>:<p className="field">{({missing:'本地未保存内容',oversized:'内容超过当前读取上限',unsupported:'此格式尚未支持',invalid:'内容暂时无法解析'} as Record<string,string>)[item.content_status]??'内容不可用'}</p>}
      <details><summary>来源详情</summary><p className="message-body">收藏编号 {item.anchor.local_id} · 类型 {item.kind}<br/>来源 {item.source_user||'未记录'} · 会话 {item.conversation||'未记录'}</p></details>
    </details></li>)}</ul>{!page.rows.length&&<p role="status">{page.next?'本批没有匹配收藏，可继续扫描。':'没有匹配的本地收藏。'}</p>}<div className="actions"><button className="button" disabled={busy||index===0} onClick={()=>void read(index-1,cursors.current[index-1])}>上一页收藏</button><span role="status">第{index+1}页 · 扫描{page.scanned}项</span><button className="button" disabled={busy||!page.next} onClick={()=>void read(index+1,page.next)}>下一页收藏</button></div></>}
    {busy&&<button className="button" onClick={clear}>取消当前操作</button>}{notice&&<p role="status">{notice}</p>}{savedPath&&<button className="button" onClick={()=>void command('show_directory_export',{path:savedPath}).catch(e=>setError(String(e)))}>查看保存位置</button>}{error&&<p role="alert" className="error-notice">{error}</p>}
  </section>;
}
