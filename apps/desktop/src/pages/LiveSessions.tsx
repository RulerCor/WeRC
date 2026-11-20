import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {ConversationMessages} from './ConversationMessages';
import {HelpTip} from '../components/HelpTip';
import {SearchHighlight} from '../components/SearchHighlight';
import {ContactAvatar} from '../components/ContactAvatar';
import {CurrentProfile} from '../components/CurrentProfile';
import type {ContactPage} from './LiveContacts';

export interface SessionPage {
  search_query?:string|null;
  rows:{anchor:{row_id:string;username:string;sort_timestamp:number};summary:string|null;summary_status:string;last_timestamp:number|null;unread_count:number|null;is_hidden:number|null}[];
  next:string|null;expires_in_seconds:number;mode:'local_session_cache';complete_session_list:false;cross_page_snapshot:false;
}
export function LiveSessions({accountId,viewId,multi=false,embedded=false,autoRead=false}:{accountId:string;viewId:string;multi?:boolean;embedded?:boolean;autoRead?:boolean}) {
  const [page,setPage]=useState<SessionPage|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[filter,setFilter]=useState(''),[index,setIndex]=useState(0),[selected,setSelected]=useState<string|null>(null);
  const generation=useRef(0),request=useRef<string|null>(null),cursors=useRef<(string|null)[]>([null]);
  const [contacts,setContacts]=useState<Record<string,ContactPage['rows'][number]>>({}),[namesNotice,setNamesNotice]=useState('');
  const namesReady=useRef(false);
  useEffect(()=>{clear();if(autoRead)void read(0,null,null);return()=>{generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});};},[accountId,viewId,autoRead]);
  async function read(at:number,cursor:string|null,searchQuery:string|null){
    if(request.current)return;
    const query=searchQuery?.trim()||null;
    if(query&&(new TextEncoder().encode(query).length>256||query.includes('\0'))){setError('搜索文字最多256个UTF-8字节，不能包含空字符。');return;}
    const expected=++generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');setSelected(null);
    try{
      const result=await command<SessionPage>('native_live_sessions',{accountId,viewId,cursor,searchQuery:query,requestId});
      if(expected!==generation.current)return;
      cursors.current=cursors.current.slice(0,at+1);cursors.current[at]=cursor;setPage(result);setIndex(at);setFilter(result.search_query??'');
      if(!namesReady.current){
        try{let next:string|null=null;const values:Record<string,ContactPage['rows'][number]>={};let count=0;
          do{if(expected!==generation.current)return;const nameRequest=crypto.randomUUID();request.current=nameRequest;
            const names:ContactPage=await command('native_live_contacts',{accountId,viewId,cursor:next,searchQuery:null,requestId:nameRequest});
            if(expected!==generation.current)return;for(const row of names.rows){values[row.anchor.username]=row;count++;}if(count>50000)throw new Error('名称目录超过当前读取范围。');next=names.next;
          }while(next);setContacts(values);namesReady.current=true;setNamesNotice('');
        }catch{if(expected===generation.current)setNamesNotice('联系人名称暂时不可用，当前仍按真实账号显示。重新读取可重试。');}
      }
    }catch(e){if(expected===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(expected===generation.current){request.current=null;setBusy(false);}}
  }
  function clear(){setSelected(null);generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});setPage(null);setBusy(false);setError('');setFilter('');setIndex(0);setContacts({});namesReady.current=false;setNamesNotice('');cursors.current=[null];}
  const query=page?.search_query??'';
  const visible=page?.rows??[];
  return <details open={embedded||undefined} className="live-sessions" onToggle={e=>{if(!e.currentTarget.open)clear();}}>
    <summary hidden={embedded}>会话资料</summary>
    {page&&!busy&&<CurrentProfile accountId={accountId} viewId={viewId}/>}
    <form className="contact-search" onSubmit={e=>{e.preventDefault();void read(0,null,filter);}}>
      <label className="field">查找会话<input type="search" aria-label="查找会话" value={filter} onChange={e=>setFilter(e.target.value)} maxLength={256} placeholder="会话账号或本地摘要" disabled={busy}/></label>
      <div className="actions"><button type="submit" className="button" disabled={busy}>{busy?'正在读取…':filter.trim()?'查找会话':page?'重新读取会话':'查看本地会话'}</button>
        {query&&<button type="button" className="button" disabled={busy} onClick={()=>void read(0,null,null)}>清除搜索</button>}
        {busy&&<button type="button" className="button" onClick={clear}>取消读取</button>}
        <HelpTip label="本地会话说明">搜索当前账号本地会话缓存中的账号标识与有效摘要，不限于本页200条，也不改变微信未读状态。翻页保持查询条件；查找昵称或备注请使用联系人资料。点击查看消息会打开当前已准备的单个或多个分片，未选择的分片不包含在内。新消息可能改变排序，边界变化时需重新读取。窗口过期后请重新准备消息来源；关闭面板清除显示。未读数与隐藏标记来自本地缓存。</HelpTip>
      </div>
    </form>
    {error&&<p className="error-notice" role="alert">{error}</p>}
    {namesNotice&&<p className="field" role="status">{namesNotice}</p>}
    {selected&&<ConversationMessages key={selected} accountId={accountId} parentViewId={viewId} username={selected} displayName={contacts[selected]?.remark||contacts[selected]?.nickname||undefined} multi={multi} onClose={()=>setSelected(null)}/>}
    {page&&!selected&&<>{query&&<p className="field" role="status">搜索结果：<span dir="auto">{query}</span></p>}
      <ul className="history-messages" aria-label="本地会话列表">{visible.map(row=><li key={row.anchor.row_id}><ContactAvatar accountId={accountId} viewId={viewId} username={row.anchor.username} name={contacts[row.anchor.username]?.remark||contacts[row.anchor.username]?.nickname||row.anchor.username} available={Boolean(contacts[row.anchor.username]?.avatar_url)}/><div className="session-row-heading"><strong dir="auto" title={row.anchor.username}><SearchHighlight text={contacts[row.anchor.username]?.remark||contacts[row.anchor.username]?.nickname||(row.anchor.username==='filehelper'?'文件传输助手':row.anchor.username)} needle={query}/></strong><button className="button" disabled={busy} onClick={()=>setSelected(row.anchor.username)}>查看消息</button></div><span className="session-summary-preview" dir="auto">{row.summary?<SearchHighlight text={row.summary} needle={query}/>:({missing:'暂无摘要',oversized:'摘要超过显示上限',invalid:'摘要无法解析'} as Record<string,string>)[row.summary_status]??'摘要暂不可用'}</span>{row.summary&&row.summary.length>120&&<HelpTip label="查看完整会话摘要"><span dir="auto">{row.summary}</span></HelpTip>}<span className="field">{row.unread_count===null?'未读数未知':`${row.unread_count}条未读`}{row.is_hidden?' · 本地已隐藏':''}</span></li>)}</ul>
      {!visible.length&&<p className="empty-inline">{query?'当前来源未找到匹配的会话，可以更换关键词或清除搜索。':'此来源暂无会话资料。'}</p>}
      <div className="actions"><button className="button" disabled={busy||index===0} onClick={()=>void read(index-1,cursors.current[index-1],query)}>上一页会话</button><span className="field" role="status">第{index+1}页 · 本页{page.rows.length}条</span><button className="button" disabled={busy||!page.next} onClick={()=>void read(index+1,page.next,query)}>下一页会话</button></div>
    </>}
  </details>;
}
