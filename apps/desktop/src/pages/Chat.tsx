import type {NotificationNavigation} from '../components/DesktopNotification';
import { useEffect,useLayoutEffect, useRef, useState, type FormEvent } from 'react';
import { ArrowDown, ArrowLeft, ArrowUp, Download, MessageSquare, RefreshCw, Search } from 'lucide-react';
import { command, desktopAvailable } from '../lib/desktop';
import type { Workbench } from '../hooks/useWorkbench';
import { useConversationList, useMessages } from '../hooks/useBrowse';
import { baseQuery, messageLabel, safeTime, type ConversationView, type MessageView, type TimelinePage } from '../lib/browse';
import { History as SnapshotManager } from './History';
import './chat.css';
import { NativeChat } from './NativeChat';
import { LiveChat } from './LiveChat';
import {HelpTip} from '../components/HelpTip';

export function Chat(workbench:Workbench&{onConnect?:()=>void;notificationTarget?:NotificationNavigation|null}) {
  const {data}=workbench;
  const [archive,setArchive]=useState('');
  const [mode,setMode]=useState(()=>{try{const saved=localStorage.getItem('werc.chat.mode');return ['native','live_window','imported_snapshot'].includes(saved??'')?saved!:'native';}catch{return 'native';}});
  const [clearedScope,setClearedScope]=useState<string|null>(null),[clearing,setClearing]=useState(false),[cacheError,setCacheError]=useState('');
  useEffect(()=>{const n=workbench.notificationTarget;if(n?.page==='history'){setMode('native');setArchive(n.account);}},[workbench.notificationTarget]);
  const accountId=archive||data.current_connection.account_id||'';
  const account=data.accounts.find(a=>a.id===accountId);
  const sources=data.history_sources.filter(s=>s.account_id===accountId);
  function chooseMode(value:string){setMode(value);try{localStorage.setItem('werc.chat.mode',value);}catch{/* Optional preference. */}}
  const scope=`${accountId}/${data.current_connection.revision}/${sources.map(s=>s.id).sort().join(',')}`;
  async function clearCache() {
    setClearing(true);setCacheError('');
    try {await command<number>('clear_browse_cache',{accountId});setClearedScope(scope);}
    catch(e){setCacheError(e instanceof Error?e.message:String(e));}
    finally{setClearing(false);}
  }
  return <>
    <div className="chat-mode-tabs" role="group" aria-label="会话内容">{([['live_window','微信会话'],['native','实时新消息'],['imported_snapshot','离线归档']] as const).map(([id,label])=><button className="button" key={id} aria-pressed={mode===id} onClick={()=>chooseMode(id)}>{label}</button>)}<HelpTip label="会话内容说明">微信会话用于浏览当前电脑保存的聊天和资料；实时新消息用于连接期间接收的消息；离线归档用于已导入的历史快照。</HelpTip></div>
    <details className="chat-scope"><summary>来源与范围</summary><div className="chat-toolbar">
      <label>查看<select aria-label="聊天读取模式" value={mode} onChange={e=>{setMode(e.target.value);try{localStorage.setItem('werc.chat.mode',e.target.value);}catch{/* Preference storage is optional. */}}}><option value="imported_snapshot">已导入快照</option><option value="native">实时接收</option><option value="live_window">微信历史与资料</option></select></label><HelpTip label="如何选择消息来源">实时接收显示连接期间保存的消息；微信历史与资料按需读取当前电脑保存的数据；历史快照用于离线浏览已归档资料。不同来源分别展示，避免把旧快照误认成实时消息。</HelpTip>
      <label>资料范围<select aria-label="聊天归档范围" value={archive} disabled={!desktopAvailable} onChange={e=>setArchive(e.target.value)}><option value="">{data.accounts.find(a=>a.id===data.current_connection.account_id)?.label??'尚未配置当前微信'}</option>{data.accounts.filter(a=>a.id!==data.current_connection.account_id).map(a=><option key={a.id} value={a.id}>归档 / {a.label}</option>)}</select></label>
      <span className="state-tag">{mode==='native'?'连接期间保存的消息':mode==='live_window'?'当前微信资料':'离线快照'}</span>
    </div></details>
    {!account&&<button className="button primary" onClick={workbench.onConnect}>前往连接微信</button>}
    {mode==='live_window'?(account?<LiveChat key={scope} accountId={account.id} current={account.id===data.current_connection.account_id}/>:<p className="empty-inline">尚未配置微信。</p>):mode==='native'?(account?<NativeChat key={account.id} accountId={account.id} focus={workbench.notificationTarget} {...workbench}/>:<p className="empty-inline">尚未配置微信。</p>):clearedScope===scope?<div className="empty-state" role="status"><h3>查询缓存已清理</h3><button className="button" onClick={()=>setClearedScope(null)}>重新打开会话</button></div>:account&&sources.length>0?<ChatWorkspace key={scope} accountId={account.id} sources={sources} scope={scope} onOpenLive={()=>setMode('live_window')}/>:<div className="empty-state"><MessageSquare size={36} strokeWidth={1.3}/><h3>{account?'此范围还没有历史快照':'尚未配置微信'}</h3><p>连接后可查看实时接收；查看历史请切换到“微信历史与资料”。</p></div>}
    <details className="chat-cache"><summary>查询缓存</summary><p className="field">只清理当前资料范围的可重建查询缓存，不删除快照或密钥。</p><button className="button" disabled={!desktopAvailable||!account||clearing} onClick={()=>void clearCache()}>{clearing?'正在清理…':'清理查询缓存'}</button>{cacheError&&<p role="alert" className="error-notice">{cacheError}</p>}</details>
    <details className="chat-management"><summary>快照来源、导入与独立导出</summary><SnapshotManager {...workbench}/></details>
  </>;
}
function ChatWorkspace({accountId,sources,scope,onOpenLive}:{accountId:string;sources:Workbench['data']['history_sources'];scope:string;onOpenLive:()=>void}) {
  const [sourceId,setSourceId]=useState<string|null>(null),[conversation,setConversation]=useState<ConversationView|null>(null);
  const [draft,setDraft]=useState(''),[search,setSearch]=useState('');
  const selectedScope=`${scope}/${sourceId??'aggregate'}`;
  const list=useConversationList(accountId,sourceId,selectedScope,search);
  const [pages,setPages]=useState<ConversationView[]>([]);
  const items=pages.length?pages:list.data?.data??[];
  async function more(){const page=await list.load(list.data?.next_cursor);if(page)setPages(old=>[...(old.length?old:items),...page.data.filter(row=>!(old.length?old:items).some(p=>p.id===row.id))]);}
  function chooseSource(value:string){list.cancel();setSourceId(value||null);setConversation(null);setPages([]);}
  return <>
    <div className="chat-source-row"><label>快照<select aria-label="聊天快照" value={sourceId??''} disabled={list.loading} onChange={e=>chooseSource(e.target.value)}><option value="">聚合已导入快照 / 最新已知版本</option>{sources.map(s=><option key={s.id} value={s.id}>{new Date(s.imported_at).toLocaleString('zh-CN')} / {s.message_count} 条 / {s.id.slice(0,8)}</option>)}</select></label><p className="field">仅本机副本 / 不改变微信已读</p></div>
    <div className={`chat-workspace ${conversation?'has-conversation':''}`}>
      <aside className="conversation-panel" aria-label="会话列表">
        <form className="chat-search" onSubmit={e=>{e.preventDefault();setSearch(draft.trim());setPages([]);setConversation(null);}}><input aria-label="搜索会话" value={draft} maxLength={200} onChange={e=>setDraft(e.target.value)} placeholder="名称或本地标识"/><button className="icon-button" aria-label="搜索会话" disabled={list.loading}><Search size={17}/></button></form>
        <div className="conversation-heading"><strong>会话</strong><button className="icon-button" aria-label="刷新会话" disabled={list.loading} onClick={()=>{setPages([]);void list.load();}}><RefreshCw size={15}/></button></div>
        {list.loading&&<QueryLoading cancel={list.cancel} canceling={list.canceling}/>}
        {list.error&&<p className="error-notice" role="alert">{list.error}</p>}
        <ul className="conversation-list">{items.map(row=><li key={row.id}><button className="conversation-row" aria-pressed={conversation?.id===row.id} onClick={()=>setConversation(row)}><span className="chat-avatar" aria-hidden="true">{row.display_name.slice(0,1)}</span><span className="conversation-text"><strong dir="auto">{row.display_name}</strong><span dir="auto">{row.summary||'暂无文字摘要'}</span><time>{safeTime(row.timestamp)}</time></span>{row.historical_unread_count&&row.historical_unread_count!=='0'&&<span className="historical-unread" title="快照记录的历史未读，不代表当前未读">{row.historical_unread_count}</span>}</button></li>)}</ul>
        {!list.loading&&!list.error&&list.data&&!items.length&&<p className="chat-empty">当前范围没有会话。</p>}
        {list.data?.next_cursor&&<button className="button" disabled={list.loading} onClick={()=>void more()}>更多会话</button>}
      </aside>
      {conversation?<Conversation key={`${selectedScope}/${conversation.id}`} accountId={accountId} conversation={conversation} sourceId={sourceId} scope={selectedScope} back={()=>setConversation(null)} onOpenLive={onOpenLive}/>:<section className="chat-placeholder"><MessageSquare size={32} strokeWidth={1.2}/><h3>选择一个会话</h3><p>预览最新消息，或按日期定位。</p></section>}
    </div>
  </>;
}
function QueryLoading({cancel,canceling}:{cancel:()=>void;canceling:boolean}) {return <div className="query-loading" role="status"><span>{canceling?'正在取消扫描…':'正在准备或查询快照…'}</span><button className="button" onClick={cancel} disabled={canceling}>取消查询</button></div>;}
function Conversation({accountId,conversation,sourceId,scope,back,onOpenLive}:{accountId:string;conversation:ConversationView;sourceId:string|null;scope:string;back:()=>void;onOpenLive:()=>void}) {
  const [draft,setDraft]=useState(''),[search,setSearch]=useState(''),[date,setDate]=useState(''),[anchor,setAnchor]=useState('');
  const messages=useMessages(accountId,conversation.id,sourceId,scope,search);
  const [pages,setPages]=useState<TimelinePage[]|null>(null);
  const rows=pages?.flatMap(page=>page.data)??null;
  const [selected,setSelected]=useState<MessageView|null>(null),[exportPath,setExportPath]=useState(''),[exported,setExported]=useState<number|null>(null),[exporting,setExporting]=useState(false),[exportError,setExportError]=useState('');
  const scroll=useRef<HTMLDivElement>(null),position=useRef<{height:number;top:number;mode:'prepend'|'bottom';anchor?:{id:string;offset:number}}|null>(null);
  const display=rows??messages.data?.data??[];
  const before=pages?pages[0].older_cursor:messages.data?.older_cursor,after=pages?pages[pages.length-1].newer_cursor:messages.data?.newer_cursor;
  useLayoutEffect(()=>{
    if(!scroll.current)return;
    const hold=position.current;
    if(hold){
      const pane=scroll.current;
      const anchor=hold.anchor&&[...pane.querySelectorAll<HTMLElement>('[data-message-id]')].find(node=>node.dataset.messageId===hold.anchor?.id);
      if(anchor&&hold.anchor)pane.scrollTop+=anchor.getBoundingClientRect().top-pane.getBoundingClientRect().top-hold.anchor.offset;
      else pane.scrollTop=hold.mode==='prepend'?hold.top+pane.scrollHeight-hold.height:pane.scrollHeight;
      position.current=null;
    }
    else if(rows===null&&messages.data)scroll.current.scrollTop=scroll.current.scrollHeight;
  },[pages,messages.data]);
  async function page(direction:'older'|'newer'|'latest'|'around',cursor:string|null=null) {
    const previous=pages??(messages.data?[messages.data]:[]);
    const timestamp=direction==='around'&&date?Math.floor(new Date(date).getTime()/1000):null;
    const result=await messages.load(direction,cursor,direction==='around'&&!date?anchor||null:null,timestamp);
    if(!result)return;
    const pane=scroll.current;
    // 在应用结果时捕获，尊重等待期间用户自己进行的滚动。
    const bounds=pane?.getBoundingClientRect();
    const retained=new Set(previous.slice(0,2).flatMap(page=>page.data.map(message=>message.id)));
    const visible=pane&&bounds?[...pane.querySelectorAll<HTMLElement>('[data-message-id]')].find(node=>retained.has(node.dataset.messageId??'')&&node.getBoundingClientRect().bottom>bounds.top&&node.getBoundingClientRect().top<bounds.bottom):undefined;
    const hold=pane?{height:pane.scrollHeight,top:pane.scrollTop,mode:'prepend' as const,anchor:visible&&bounds?{id:visible.dataset.messageId!,offset:visible.getBoundingClientRect().top-bounds.top}:undefined}:null;
    if(direction==='older'){
      position.current=hold;
      // 保留完整页及其末端游标，裁剪后不能复用被裁掉消息的边界。
      setPages([result,...previous].slice(0,3));
    }else {
      position.current=direction==='latest'&&pane?{height:0,top:0,mode:'bottom'}:null;
      setPages([result]);
    }
    setSelected(null);
  }
  async function exportConversation(e:FormEvent) {
    e.preventDefault();setExporting(true);setExportError('');setExported(null);
    try{setExported(await command<number>('save_history_export',{accountId,query:{...baseQuery(),conversation_id:conversation.id,search:search||null},format:'json',path:exportPath}));}
    catch(e){setExportError(e instanceof Error?e.message:String(e));}finally{setExporting(false);}
  }
  return <section className="timeline-panel" aria-label="聊天时间线">
    <header className="conversation-header"><button className="icon-button chat-back" aria-label="返回会话列表" onClick={back}><ArrowLeft size={18}/></button><div><h2 dir="auto">{conversation.display_name}</h2><p className="field">快照 / 方向未解析</p></div><button className="button" disabled={messages.loading} onClick={()=>void page('latest')}>最新页</button></header>
    <form className="chat-search" onSubmit={e=>{e.preventDefault();setPages(null);setSelected(null);setSearch(draft.trim());}}><input aria-label="搜索当前会话" placeholder="会话内关键词" value={draft} maxLength={200} onChange={e=>setDraft(e.target.value)}/><button className="icon-button" aria-label="查询当前会话" disabled={messages.loading}><Search size={17}/></button></form>
    <details className="chat-locate"><summary>定位日期或消息</summary><p className="field">会话标识 / <code>{conversation.id}</code></p><form onSubmit={e=>{e.preventDefault();void page('around');}}><label>日期（本地时间）<input aria-label="聊天定位日期" type="datetime-local" value={date} onChange={e=>{setDate(e.target.value);setAnchor('');}}/></label><label>本地消息 ID<input aria-label="聊天定位消息ID" value={anchor} maxLength={512} onChange={e=>{setAnchor(e.target.value);setDate('');}}/></label><button className="button" disabled={messages.loading||(!date&&!anchor)}>定位</button></form></details>
    {messages.loading&&<QueryLoading cancel={messages.cancel} canceling={messages.canceling}/>}
    {messages.error&&<p className="error-notice" role="alert">{messages.error}<button className="button" disabled={messages.loading} onClick={()=>void page('latest')}>重试</button></p>}
    <div className="timeline-scroll" ref={scroll} tabIndex={0} aria-label="消息记录">
      {before&&<button className="button timeline-page" disabled={messages.loading} onClick={()=>void page('older',before)}><ArrowUp size={15}/>更早消息</button>}
      <ol className="message-timeline">{display.map((message,index)=><li key={message.id} data-message-id={message.id} data-direction={message.direction}>
        {(index===0||new Date(display[index-1].timestamp*1000).toLocaleDateString()!==new Date(message.timestamp*1000).toLocaleDateString())&&<p className="timeline-date">{safeTime(message.timestamp).split(' ')[0]}</p>}
        <div className="message-heading"><strong dir="auto">{message.sender_name??message.sender_id??'发送者未解析'}</strong><time>{safeTime(message.timestamp)}</time></div>
        <button className="message-bubble" aria-label={`查看消息 ${message.server_id}`} onClick={()=>setSelected(selected?.id===message.id?null:message)}><span className="message-kind">{messageLabel(message.message_type)}</span><span className="message-body" dir="auto">{message.text?message.text.length>12000?`${message.text.slice(0,12000)}\n[长消息已折叠，点击查看全文]`:message.text:`[${messageLabel(message.message_type)}内容尚未解码]`}</span>{message.content_status!=='decoded'&&message.content_status!=='text'&&<span className="field">内容状态 / {message.content_status}</span>}</button>
      </li>)}</ol>
      {!messages.loading&&messages.data&&!display.length&&<p className="chat-empty">当前条件下没有消息。</p>}
      {after&&<button className="button timeline-page" disabled={messages.loading} onClick={()=>void page('newer',after)}><ArrowDown size={15}/>更新消息</button>}
    </div>
    {selected&&<section className="message-details" aria-label="消息详情"><div><h3>消息详情</h3><button className="button" onClick={()=>setSelected(null)}>关闭</button></div><dl><dt>服务端 ID</dt><dd><code>{selected.server_id}</code></dd><dt>本地 ID / 序列</dt><dd><code>{selected.local_id} / {selected.sequence}</code></dd><dt>消息锚点</dt><dd><code>{selected.id}</code></dd><dt>发送者</dt><dd>{selected.sender_id??'未解析'}</dd></dl><p className="field">时间线最多保留 180 条；更早记录仍可分页查看。</p>{selected.text&&<pre dir="auto">{selected.text}</pre>}<HelpTip label="快照媒体说明">此导入快照只包含已解码文字和消息标识。查看图片、视频或附件，可切换“微信历史与资料”读取当前微信；离线归档的媒体可从已保存资料中查看。</HelpTip></section>}
    <details className="chat-export"><summary>导出当前会话</summary><form onSubmit={e=>void exportConversation(e)}><label>JSON 目标文件（相对系统下载目录）<input aria-label="聊天导出路径" value={exportPath} onChange={e=>setExportPath(e.target.value)} disabled={exporting}/></label><button className="button" disabled={exporting||!exportPath.trim()||sourceId!==null}><Download size={15}/>导出</button></form><p className="field">聚合快照 / 当前会话与关键词 / 32MiB 上限 / 不覆盖文件。选择单份快照时不导出不同范围。</p>{exported!==null&&<p role="status">已导出 {exported.toLocaleString()} 字节</p>}{exportError&&<p role="alert">{exportError}</p>}</details>
    <div className="chat-composer"><p className="field">正在浏览历史快照</p><button className="button" onClick={onOpenLive}>前往微信历史与资料</button><HelpTip label="如何从历史快照发送消息">先切换到当前登录微信，打开会话并准备收件人，即可手动发送；历史快照本身只用于回看。</HelpTip></div>
  </section>;
}
