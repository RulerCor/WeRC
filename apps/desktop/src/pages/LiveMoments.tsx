import {MomentActions} from './MomentActions';
import {MomentGallery} from './MomentGallery';
import {MomentPreservation} from './MomentPreservation';
import {MomentDetails} from './MomentDetails';
import {MomentTools,type MomentQuery} from './MomentTools';
import {MomentComposer} from './MomentComposer';
import {MomentInteractions} from './MomentInteractions';
import type {ContactPage} from './LiveContacts';
import type {MomentRow} from '../lib/moment-people';
import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {safeTime} from '../lib/browse';
import {HelpTip} from '../components/HelpTip';

interface MomentPage {
  rows: MomentRow[];
  next:string|null;expires_in_seconds:number;mode:'local_moment_cache';complete_timeline:false;cross_page_snapshot:false;
}
const emptyQuery:MomentQuery={text:'',author:null,from:null,until:null,content_type:null};
function localTime(n:number|null){if(n===null)return '';const d=new Date(n*1000);return `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}T${String(d.getHours()).padStart(2,'0')}:${String(d.getMinutes()).padStart(2,'0')}`;}
const missing:Record<string,string>={missing:'本地未保存内容',oversized:'内容超过读取上限',unsupported:'此内容格式尚未适配',invalid:'内容暂时无法解析'};
export function LiveMoments({accountId,viewId,embedded=false}:{accountId:string;viewId:string;embedded?:boolean}) {
  const [page,setPage]=useState<MomentPage|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[index,setIndex]=useState(0);
  const [query,setQuery]=useState<MomentQuery>(emptyQuery),[toolBusy,setToolBusy]=useState(false);
  const [preservationOpen,setPreservationOpen]=useState(false);
  const [authorNames,setAuthorNames]=useState<Record<string,string>>({});
  const generation=useRef(0),request=useRef<string|null>(null),cursors=useRef<(string|null)[]>([null]),expires=useRef(0);
  useEffect(()=>{
    clear();setToolBusy(false);
    return()=>{generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});request.current=null;};
  },[accountId,viewId]);
  useEffect(()=>{if(!page)return;const timer=window.setInterval(()=>{if(Date.now()>=expires.current){clear();setError('读取窗口已过期，请重新准备消息来源。');}},1000);return()=>window.clearInterval(timer);},[page]);
  function clear(){generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});setPage(null);setAuthorNames({});setBusy(false);setError('');setIndex(0);cursors.current=[null];}
  async function readAuthorNames(){
    if(!page||request.current||toolBusy)return;
    const g=++generation.current,id=crypto.randomUUID();request.current=id;setBusy(true);setError('');
    try{for(const username of new Set(page.rows.map(r=>r.anchor.username))){
      let cursor:string|null=null;
      do{const contacts:ContactPage=await command('native_live_contacts',{accountId,viewId,cursor,searchQuery:username,requestId:id});if(g!==generation.current)return;
        const contact=contacts.rows.find(r=>r.anchor.username===username);
        if(contact){const name=contact.remark||contact.nickname||username;setAuthorNames(old=>({...old,[username]:name}));break;}
        cursor=contacts.next;
      }while(cursor);
    }}catch(e){if(g===generation.current)setError(`部分作者资料未读取：${String(e)}。未匹配的作者继续显示准确账号，可重新读取。`);}
    finally{if(g===generation.current){request.current=null;setBusy(false);}}
  }
  async function read(at:number,cursor:string|null,selection:MomentQuery=query){
    if(request.current||toolBusy)return;
    const expected=++generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');
    try{const result=await command<MomentPage>('native_live_moments',{accountId,viewId,cursor,query:cursor?null:selection,requestId});if(expected!==generation.current)return;
      cursors.current=cursors.current.slice(0,at+1);cursors.current[at]=cursor;expires.current=Date.now()+result.expires_in_seconds*1000;setPage(result);setIndex(at);
    }catch(e){if(expected===generation.current){setPage(null);setError(e instanceof Error?e.message:String(e));}}
    finally{if(expected===generation.current){request.current=null;setBusy(false);}}
  }
  return <details open={embedded||undefined} className="live-moments" onToggle={e=>{if(!e.currentTarget.open)clear();}}>
    <summary hidden={embedded}>朋友圈</summary>
    <div className="actions"><button className="button" disabled={busy||toolBusy} onClick={()=>void read(0,null)}>{busy?'正在读取…':page?'重新读取朋友圈':'查看本地朋友圈'}</button>
      {busy&&<button className="button" onClick={clear}>取消读取</button>}
      <HelpTip label="本地朋友圈说明">这里按需读取当前账号已缓存的朋友圈，每页最多30条，按动态标识顺序显示。它可能缺少手机中的动态；翻页为独立的短时间读取，不能代表完整朋友圈或服务端实时状态。可按关键词、作者、类型和日期筛选，展开日期与作者可统计，导出会遍历完整筛选范围。可按需读取本页作者备注与昵称，准确账号保留在问号中，正文可展开；媒体显示数量，赞和评论为本地缓存数量。媒体可按需载入、播放和加密保存；单条动态的“互动”中可保存点赞或评论请求，再到发送队列批准。公开或仅自己可见的文字、图片、视频动态可在“写一条朋友圈动态”中保存草稿，经发送队列预览批准后发布。关闭面板或窗口过期会清除显示。首次使用请先连接微信、检测并准备一个消息来源。</HelpTip>
    </div>
    <div className="history-query"><label>查找动态<input value={query.text} disabled={busy||toolBusy} placeholder="关键词" onChange={e=>{clear();setQuery({...query,text:e.target.value});}}/></label><button className="button" disabled={busy||toolBusy} onClick={()=>void read(0,null)}>查找朋友圈</button></div>
    <details><summary>作者、类型与时间范围</summary><div className="history-query"><label>作者账号<input value={query.author??''} disabled={busy||toolBusy} onChange={e=>{clear();setQuery({...query,author:e.target.value||null});}}/></label><label>内容类型<select value={query.content_type??''} disabled={busy||toolBusy} onChange={e=>{clear();setQuery({...query,content_type:e.target.value?Number(e.target.value):null});}}><option value="">全部类型</option><option value="1">图片</option><option value="2">文字</option><option value="3">链接</option><option value="15">视频</option></select></label><label>开始时间<input type="datetime-local" value={localTime(query.from)} disabled={busy||toolBusy} onChange={e=>{clear();setQuery({...query,from:e.target.value?Math.floor(new Date(e.target.value).getTime()/1000):null});}}/></label><label>结束时间<input type="datetime-local" value={localTime(query.until)} disabled={busy||toolBusy} onChange={e=>{clear();setQuery({...query,until:e.target.value?Math.floor(new Date(e.target.value).getTime()/1000):null});}}/></label><button className="button" disabled={busy||toolBusy} onClick={()=>{clear();setQuery(emptyQuery);}}>清除筛选</button></div></details>
    <MomentTools accountId={accountId} viewId={viewId} query={query} disabled={busy} onBusy={setToolBusy} onSelect={q=>{clear();setQuery(q);void read(0,null,q);}}/>
    <MomentInteractions accountId={accountId} viewId={viewId} query={query} disabled={busy} onBusy={setToolBusy}/>
    <details onToggle={e=>setPreservationOpen(e.currentTarget.open)}><summary>保留朋友圈副本</summary>{preservationOpen&&<MomentPreservation key={accountId} accountId={accountId}/>}</details>
    <MomentComposer key={accountId} accountId={accountId}/>
    {error&&<p className="error-notice" role="alert">{error}</p>}
    {page&&<><div className="actions"><button className="button" disabled={busy||toolBusy||!page.rows.length} onClick={()=>void readAuthorNames()}>读取本页作者名称</button><HelpTip label="朋友圈作者名称说明">从当前来源联系人缓存按准确账号匹配备注和昵称，名称不会替代作者真实账号。按需读取以避免每次翻页额外扫描联系人；未找到时保留账号，不按相似昵称匹配。</HelpTip></div><ul className="history-messages" aria-label="本地朋友圈列表">{page.rows.map(row=><li key={`${row.anchor.id}:${row.anchor.row_id}`}>
      <div className="moment-heading"><strong dir="auto">{authorNames[row.anchor.username]||row.anchor.username}</strong><HelpTip label="查看朋友圈作者账号"><span dir="auto">{row.anchor.username}</span></HelpTip>{row.content&&<time className="field">{safeTime(row.content.timestamp)}</time>}</div>
      {row.content?<>
        {row.content.text?<details className="moment-body" key={`${row.anchor.id}:${index}`}><summary><span className="session-summary-preview" dir="auto">{row.content.text}</span><span className="field">展开 / 收起正文</span></summary><p className="message-body" dir="auto">{row.content.text}</p></details>:<p className="field">这条动态没有文字正文。</p>}
        <span className="field">{row.content.media_count?`${row.content.media_count}项媒体 · `:''}缓存中{row.content.cached_like_count}个赞 · {row.content.cached_comment_count}条评论</span>
        {row.content.details&&<><MomentGallery key={`${row.anchor.id}:${row.anchor.row_id}`} accountId={accountId} viewId={viewId} anchor={row.anchor} media={row.content.details.media}/><MomentDetails value={row.content.details} accountId={accountId} snsId={row.anchor.id}/></>}
        <MomentActions key={row.anchor.id} accountId={accountId} snsId={row.anchor.id}/>
      </>:<p className="field">{missing[row.content_status]??'内容暂不可用'}</p>}
    </li>)}</ul>
      {!page.rows.length&&<p className="empty-inline">{page.next?'本次扫描尚无匹配动态，可继续扫描下一页。':'当前缓存没有匹配的动态。'}</p>}
      <div className="actions"><button className="button" disabled={busy||toolBusy||index===0} onClick={()=>void read(index-1,cursors.current[index-1])}>上一页动态</button><span className="field" role="status">第{index+1}页 · 本页{page.rows.length}条</span><button className="button" disabled={busy||toolBusy||!page.next} onClick={()=>void read(index+1,page.next)}>下一页动态</button></div>
    </>}
  </details>;
}
