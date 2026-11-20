import {DirectoryExport} from '../components/DirectoryExport';
import {PosterExport} from '../components/PosterExport';
import {ConversationComposer} from './ConversationComposer';
import type {ConversationCard} from '../components/MessageCard';
import type {MediaHint} from './ConversationMedia';
import {ConversationMessageRow,messageDay} from './ConversationMessageRow';
import {ConversationCalendar} from './ConversationCalendar';
import {ConversationSearch} from './ConversationSearch';
import {queryActive,type ConversationQuery} from '../lib/conversation-query';
import {LiveGroups} from './LiveGroups';
import {MessagePreservation} from './MessagePreservation';
import {useEffect,useMemo,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import type {PreparedRecipient} from '../lib/recipients';
import {HelpTip} from '../components/HelpTip';
import type {LivePage,MultiLivePage} from './LiveChat';
interface ConversationPage {focus?:{local_id:string;source:string|null}|null;query?:ConversationQuery|null;cards?:ConversationCard[];media?:MediaHint[];username:string;messages:LivePage|MultiLivePage;source_scope:'selected_message_source'|'selected_message_sources';sending_enabled:false}
type Action={action:'open_search';parent_view_id:string;result_token:string}|{action:'open';parent_view_id:string;username:string}|{action:'page';view_id:string;cursor:string;direction:'older'|'newer'}|{action:'continue';view_id:string;cursor:string}|{action:'find';view_id:string;query:ConversationQuery}|{action:'context';view_id:string;local_id:string;source:string|null};
export function ConversationMessages({accountId,parentViewId,username,onClose,multi=false,displayName,initialSearchToken}:{accountId:string;parentViewId:string;username:string;onClose:()=>void;multi?:boolean;displayName?:string;initialSearchToken?:string}){
  const [page,setPage]=useState<ConversationPage|null>(null),[readBusy,setBusy]=useState(false),[error,setError]=useState('');
  const [recipientBusy,setRecipientBusy]=useState(false),[recipientNotice,setRecipientNotice]=useState('');
  const [calendarBusy,setCalendarBusy]=useState(false);
  const [exportBusy,setExportBusy]=useState(false);
  const [preservationOpen,setPreservationOpen]=useState(false);
  const busy=readBusy||calendarBusy||exportBusy;
  const [returnQuery,setReturnQuery]=useState<ConversationQuery|null>(null);
  const [oldestFirst,setOldestFirst]=useState(false);
  const generation=useRef(0),request=useRef<string|null>(null),latest=useRef<ConversationPage|null>(null);
  const cancelRequested=useRef(false),timeline=useRef<HTMLDivElement>(null);
  const commandName=multi?'native_live_multi_conversation':'native_live_conversation';
  const clear=(viewId:string)=>command(commandName,{accountId,requestId:crypto.randomUUID(),command:{action:'clear',view_id:viewId}}).catch(()=>{});
  async function read(action:Action){
    if(request.current)return;
    setRecipientBusy(false);setRecipientNotice('');
    const expected=++generation.current,requestId=crypto.randomUUID();request.current=requestId;cancelRequested.current=false;setBusy(true);setError('');
    try{const result=await command<ConversationPage>(commandName,{accountId,requestId,command:action});
      if(expected!==generation.current){if(result)void clear(result.messages.view_id);return;}
      if(cancelRequested.current){if(result)void clear(result.messages.view_id);latest.current=null;setPage(null);setError('已取消读取，请重新打开会话。');return;}
      if(action.action==='context'&&queryActive(latest.current?.query))setReturnQuery(latest.current!.query!);
      if(action.action==='open'||action.action==='find')setReturnQuery(null);
      latest.current=result;setPage(result);
    }catch(e){if(expected===generation.current)setError(cancelRequested.current?'已取消读取。':e instanceof Error?e.message:String(e));}
    finally{if(expected===generation.current){request.current=null;setBusy(false);}}
  }
  async function cancel(){const id=request.current;if(!id)return;cancelRequested.current=true;setError('正在取消读取…');try{await command('cancel_browse',{requestId:id});}catch(e){if(request.current===id)setError(e instanceof Error?e.message:String(e));}}
  useEffect(()=>{void read(initialSearchToken?{action:'open_search',parent_view_id:parentViewId,result_token:initialSearchToken}:{action:'open',parent_view_id:parentViewId,username});return()=>{
    generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});
    if(latest.current)void clear(latest.current.messages.view_id);
  };},[accountId,parentViewId,username,multi,initialSearchToken]);
  useEffect(()=>{if(!page)return;const timer=window.setTimeout(()=>{
    if(latest.current!==page)return;
    // Expiration also retires a replacement request. Otherwise its late reply
    // could restore messages after the expired window has been cleared.
    generation.current++;
    const id=request.current;request.current=null;
    if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});
    setBusy(false);setRecipientBusy(false);setRecipientNotice('');
    void clear(page.messages.view_id);latest.current=null;setPage(null);setError('会话窗口已过期，请重新读取。');
  },page.messages.expires_in_seconds*1000);return()=>window.clearTimeout(timer);},[page]);
  useEffect(()=>{timeline.current?.querySelector<HTMLElement>('[data-located="true"]')?.scrollIntoView({block:'nearest'});},[page?.messages.view_id,oldestFirst]);
  async function prepareRecipient(){
    if(!page||recipientBusy)return;
    const current=generation.current;setRecipientBusy(true);setRecipientNotice('');
    try{const result=await command<PreparedRecipient>('native_prepare_recipient',{accountId,selection:{view_id:page.messages.view_id,multi}});
      if(current===generation.current)setRecipientNotice(`已准备收件人。请在「发送队列」选择此会话，创建并预览草稿。${result.expires_at?'五分钟内有效。':''}`);
    }catch(e){if(current===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(current===generation.current)setRecipientBusy(false);}
  }
  const messages=page?.messages;
  const posterBlocks=useMemo(()=>{
    if(!page)return [];
    const items=page.messages.mode==='live_multi_source_window'?page.messages.rows.map(r=>r.message):page.messages.rows;
    return [...items].sort((a,b)=>a.timestamp-b.timestamp).map(row=>({heading:`${row.sender_username||'身份未知'} · ${new Date(row.timestamp*1000).toLocaleString()}`,text:row.text??`[类型 ${row.message_type} · 无可读文字]`}));
  },[page]);
  const pageRows=messages?(messages.mode==='live_multi_source_window'?messages.rows.map(r=>({row:r.message,key:`${r.source}:${r.message.local_id}`,source:`分片${r.source_index}`,tag:r.source})):messages.rows.map(row=>({row,key:row.local_id,source:'',tag:null}))):[];
  const rows=oldestFirst?[...pageRows].reverse():pageRows;
  return <section aria-label="所选会话消息" className="live-conversation">
    <header className="conversation-header"><div className="conversation-title"><h3 dir="auto">{displayName||(username==='filehelper'?'文件传输助手':username)}</h3><HelpTip label="查看会话标识"><span dir="auto">{username}</span></HelpTip></div><button className="button" onClick={onClose}>返回列表</button></header>
    <div className="actions"><button className="button" disabled={busy||recipientBusy||!page?.messages.rows.length} onClick={()=>void prepareRecipient()}>{recipientBusy?'正在准备…':'设为收件人'}</button><HelpTip label="选择收件人说明">从当前已核验的会话准备收件人，不会发送消息。离开此页面后可在发送队列选择它，文字和各类媒体共用该选择；每条消息仍需预览并批准。五分钟后需重新选择。</HelpTip><button className="button" disabled={busy} onClick={()=>void read({action:'open',parent_view_id:parentViewId,username})}>{busy?'正在读取会话…':'重新读取最新消息'}</button>{messages?.mode==='live_multi_source_window'&&<span className="state-tag">已合并{messages.source_count}个分片</span>}<HelpTip label="会话消息范围">{multi?'按当前明确选择的分片合并此会话，每个分片独立只读，保留各自来源标识。':'仅查看当前已准备的一个消息分片。'}每次合并窗口最多2048条，窗口内每页60条。未选择或没有在线日志的分片不包含在内。发送者仅显示来源内可核对的账号标识，未匹配时保留未知；群会话可按需查看下方本地成员及昵称，成员缓存不代表实时完整名单；支持的媒体可逐条按需读取、预览和导出，普通文字可按需保存引用回复草稿，仍需前往发送队列预览并批准；不改变微信已读状态。窗口约两分钟后过期，继续更早窗口会重新读取并检查每个分片边界；跨分片或跨窗口不是同一时刻的快照。关闭会清除临时视图。</HelpTip></div>
    <ConversationCalendar key={`calendar:${accountId}:${parentViewId}:${username}`} accountId={accountId} parentViewId={parentViewId} username={username} busy={readBusy||exportBusy} available={Boolean(messages)} onBusyChange={setCalendarBusy} onFind={query=>{if(messages)void read({action:'find',view_id:messages.view_id,query});}}/>
    <ConversationSearch key={`${accountId}:${parentViewId}:${username}:${JSON.stringify(page?.query)}`} busy={busy} available={Boolean(messages)} query={page?.query}
      senders={[...new Set(rows.map(({row})=>row.sender_username).filter((v):v is string=>Boolean(v)))]}
      onFind={query=>{if(messages)void read({action:'find',view_id:messages.view_id,query});}}
      onReset={()=>{if(messages)void read({action:'find',view_id:messages.view_id,query:{}});}} onCancel={()=>void cancel()}/>
    {returnQuery&&messages&&<div className="actions"><button className="button" disabled={busy} onClick={()=>void read({action:'find',view_id:messages.view_id,query:returnQuery})}>返回查找结果</button><HelpTip label="定位消息范围说明">已离开筛选结果，从选中的原始消息读取前后上下文；包含最多30条相邻更新消息，向更早方向可继续翻页。定位同时核对消息ID和分片，不会跳到另一个分片中同ID的消息。可返回先前条件重新查找；原消息删除或变化时会要求重新读取。</HelpTip></div>}
    <DirectoryExport key={`export:${accountId}:${parentViewId}:${username}`} accountId={accountId} viewId={parentViewId} conversation={{username,query:page?.query}} disabled={readBusy||calendarBusy||!messages} onBusyChange={setExportBusy}/>
    {messages&&<PosterExport accountId={accountId} title={`${displayName||username} · 聊天片段`} blocks={posterBlocks} label="选择消息制作聊天海报" disabled={busy}/>}
    {username.endsWith('@chatroom')&&<LiveGroups key={`${accountId}:${parentViewId}:${username}`} accountId={accountId} viewId={parentViewId} username={username} recipientViewId={messages?.rows.length?messages.view_id:undefined} multi={multi} pinBusy={busy||!messages} onFindPin={serverId=>{if(messages)void read({action:'find',view_id:messages.view_id,query:{server_id:BigInt.asIntN(64,BigInt(serverId)).toString()}});}}/>}
    <details onToggle={e=>setPreservationOpen(e.currentTarget.open)}><summary>本机撤回留存</summary>{preservationOpen&&<MessagePreservation key={`${accountId}:${username}`} accountId={accountId} conversation={username}/>}</details>
    {recipientNotice&&<p role="status">{recipientNotice}</p>}
    {error&&<p className="error-notice" role="alert">{error}</p>}
    {messages&&<><div className="actions"><label className="field">本页阅读顺序<select aria-label="本页阅读顺序" value={oldestFirst?'oldest':'latest'} onChange={e=>setOldestFirst(e.target.value==='oldest')}><option value="latest">新消息在前</option><option value="oldest">旧消息在前</option></select></label><HelpTip label="阅读顺序说明">仅调整当前页消息的显示顺序；更早、更新按钮始终按真实时间翻页，跨分片同秒消息按来源和消息标识确定次序。查找条件与来源边界保持不变。</HelpTip></div><div key={`${messages.view_id}:${oldestFirst}`} className="timeline-scroll" ref={timeline} tabIndex={0} aria-label="所选会话时间线"><ol className="message-timeline">{rows.map(({row,key,source,tag},index)=><ConversationMessageRow
      key={`${messages.view_id}:${key}`} accountId={accountId} viewId={messages.view_id} username={username} row={row} source={source} tag={tag}
      searchText={page?.query?.text}
      focused={page?.focus?.local_id===row.local_id&&page?.focus?.source===tag} busy={busy}
      onLocate={()=>void read({action:'context',view_id:messages.view_id,local_id:row.local_id,source:tag})}
      card={page?.cards?.find(c=>c.local_id===row.local_id&&c.source===tag)} media={page?.media?.filter(h=>h.local_id===row.local_id&&h.source===tag)??[]}
      dateLabel={index===0||messageDay(rows[index-1].row.timestamp)!==messageDay(row.timestamp)?messageDay(row.timestamp):undefined}
    />)}</ol>{!messages.rows.length&&<p className="empty-inline">{queryActive(page?.query)?'没有找到符合这些条件的消息，可以调整日期、类型或文字。':'当前来源没有此会话的消息。'}</p>}</div>
    <div className="actions">{page?.focus&&pageRows[0]&&<button className="button" disabled={busy} onClick={()=>void read({action:'context',view_id:messages.view_id,local_id:pageRows[0].row.local_id,source:pageRows[0].tag})}>继续更新上下文</button>}<button className="button" disabled={busy||!messages.older} onClick={()=>void read({action:'page',view_id:messages.view_id,cursor:messages.older!,direction:'older'})}>更早会话消息</button><button className="button" disabled={busy||!messages.newer} onClick={()=>void read({action:'page',view_id:messages.view_id,cursor:messages.newer!,direction:'newer'})}>更新会话消息</button>{messages.continuation&&<button className="button" disabled={busy} onClick={()=>void read({action:'continue',view_id:messages.view_id,cursor:messages.continuation!})}>继续更早会话窗口</button>}</div></>}
    {messages&&<ConversationComposer key={`${accountId}:${username}`} accountId={accountId} viewId={messages.view_id} username={username} multi={multi} disabled={busy}/>}
  </section>;
}
