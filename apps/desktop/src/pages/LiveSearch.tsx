import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {safeTime} from '../lib/browse';
import {type ConversationQuery,queryActive} from '../lib/conversation-query';
import {HelpTip} from '../components/HelpTip';
import {SearchHighlight} from '../components/SearchHighlight';
import {LiveSender} from '../components/LiveSender';
import {ConversationSearch} from './ConversationSearch';
import {ConversationMessages} from './ConversationMessages';
import type {LiveMessage} from './LiveChat';

interface SearchRow {username:string;message:LiveMessage;source_index:number;result_token:string}
interface SearchPage {rows:SearchRow[];query:ConversationQuery;next:string|null;scanned_conversations:number;source_index:number;source_count:number;expires_in_seconds:number;source_scope:'selected_message_sources';cross_page_snapshot:false}
export function LiveSearch({accountId,viewId,multi}:{accountId:string;viewId:string;multi:boolean}) {
  const [page,setPage]=useState<SearchPage|null>(null),[previous,setPrevious]=useState<SearchPage[]>([]);
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[selected,setSelected]=useState<SearchRow|null>(null);
  const request=useRef<string|null>(null),generation=useRef(0),canceled=useRef(false);
  useEffect(()=>()=>{generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});},[accountId,viewId]);
  async function search(query:ConversationQuery|null,cursor:string|null=null) {
    if(request.current)return;
    const id=crypto.randomUUID(),version=++generation.current;
    request.current=id;canceled.current=false;setBusy(true);setError('');
    try {
      const next=await command<SearchPage>('native_live_search',{accountId,viewId,query,cursor,requestId:id});
      if(version!==generation.current||canceled.current)return;
      setPrevious(old=>cursor&&page?[...old,page].slice(-10):[]);setPage(next);
    } catch(e){if(version===generation.current&&!canceled.current)setError(e instanceof Error?e.message:String(e));}
    finally {if(version===generation.current){request.current=null;setBusy(false);}}
  }
  async function cancel(){const id=request.current;if(!id)return;canceled.current=true;setError('已请求取消，保留上一次结果。');await command('cancel_browse',{requestId:id}).catch(e=>setError(String(e)));}
  if(selected)return <ConversationMessages key={selected.result_token} accountId={accountId} parentViewId={viewId} username={selected.username} initialSearchToken={selected.result_token} multi={multi} onClose={()=>setSelected(null)}/>;
  return <section aria-label="跨会话搜索" className="live-search">
    <header className="conversation-header"><h3>跨会话搜索</h3><HelpTip label="跨会话搜索范围">搜索本次已准备的全部消息分片中的会话，包括未显示在最近会话列表里的记录。按分片、会话账号分组，每个会话内新消息在前；每批最多扫描16个会话、返回60条，空批次也可继续。这里只覆盖所选本地来源，不代表微信服务端全部历史。继续搜索保留相同条件，跨页不是同一时刻快照。来源窗口到期后需重新准备。</HelpTip></header>
    <ConversationSearch defaultOpen busy={busy} available query={page?.query} onFind={query=>void search(query)} onReset={()=>{setPage(null);setPrevious([]);setError('');}} onCancel={()=>void cancel()}/>
    {error&&<p className="error-notice" role="alert">{error}</p>}
    {!page&&!busy&&<p className="empty-inline">输入文字或选择日期、消息类型后点击查找；留空可按会话浏览所选来源。</p>}
    {page&&<><p role="status" className="field">本批检查{page.scanned_conversations}个会话，找到{page.rows.length}条 · 分片{page.source_index}/{page.source_count}</p>
      <ol className="search-results">{page.rows.map((hit,index)=><li key={`${hit.source_index}:${hit.username}:${hit.message.local_id}`}>
        {(index===0||page.rows[index-1].username!==hit.username)&&<h4 dir="auto">{hit.username==='filehelper'?'文件传输助手':hit.username}</h4>}
        <div className="message-heading"><LiveSender username={hit.message.sender_username}/><time>{safeTime(hit.message.timestamp)}</time></div>
        <p className="search-result-text" dir="auto">{hit.message.text?<SearchHighlight text={hit.message.text} needle={page.query.text}/>:<>非文字消息 · 类型 {hit.message.message_type}</>}</p>
        <button className="button" disabled={busy} onClick={()=>setSelected(hit)}>定位并查看前后消息</button>
      </li>)}</ol>
      {!page.rows.length&&<p className="empty-inline">{page.next?'本批没有匹配消息，可继续搜索其余会话。':previous.length?'本批没有更多匹配消息。':'所选来源中没有符合条件的消息。'}</p>}
      <div className="actions"><button className="button" disabled={busy||!previous.length} onClick={()=>{setPage(previous[previous.length-1]);setPrevious(old=>old.slice(0,-1));}}>上一批结果</button><button className="button primary" disabled={busy||!page.next} onClick={()=>void search(null,page.next)}>{busy?'正在搜索…':'继续搜索'}</button>{!page.next&&<span className="field">已到所选来源末尾</span>}<HelpTip label="搜索翻页说明">继续搜索会读取下一批，筛选条件和来源位置都保存在认证游标中。本页临时保留最近10批，可返回查看；如需修改条件，重新点击查找。{queryActive(page.query)?'当前筛选条件见上方。':''}</HelpTip></div>
    </>}
  </section>;
}
