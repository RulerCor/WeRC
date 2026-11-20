import {HelpTip} from '../components/HelpTip';
import {waitForOnlineRead} from '../lib/busy-read';
import {ConversationMessages} from './ConversationMessages';
import {isEmoticon} from '../lib/emoticon';
import {LiveSender} from '../components/LiveSender';
import {useEffect,useRef,useState} from 'react';
import {command,desktopAvailable,type DataSourceCatalog} from '../lib/desktop';
import {safeTime} from '../lib/browse';
import {RecoveryPanel} from './RecoveryPanel';
import {CapturePanel} from './CapturePanel';
import {LiveFile} from './LiveFile';
import {LiveVoice} from './LiveVoice';
import {ReceivedEmoticon} from './ReceivedEmoticon';
import {LiveContacts} from './LiveContacts';
import {LiveSessions} from './LiveSessions';
import {LiveSearch} from './LiveSearch';
import {LiveFavorites} from './LiveFavorites';
import {LiveSpecialRecords} from './LiveSpecialRecords';
import {LiveMoments} from './LiveMoments';
import {CurrentProfile} from '../components/CurrentProfile';
export interface LiveMessage {at_user_list?:string[]|null;metadata?:string|null;sender_username?:string|null;local_id:string;server_id:string|null;timestamp:number;message_type:string;text:string|null;content_status:string}
export interface LivePage {view_id:string;rows:LiveMessage[];older:string|null;newer:string|null;continuation:string|null;older_source_rows_outside_window:boolean;expires_in_seconds:number;mode:'live_bounded_result_view'}
export interface MultiLivePage extends Omit<LivePage,'rows'|'mode'|'older_source_rows_outside_window'> {rows:{source:string;source_index:number;message:LiveMessage}[];mode:'live_multi_source_window';source_count:number;source_snapshot_atomic:false}
function missingContent(status:string){return ({content_missing:'来源未保存消息正文',content_invalid:'消息内容无法解码',content_oversized:'消息内容超过当前读取上限',compression_unsupported:'此压缩格式暂不支持',unsupported_type:'此消息类型暂未解析'} as Record<string,string>)[status]??'消息内容暂未解析';}
function isVoice(type:string){try{return (BigInt(type)&0xffffffffn)===34n;}catch{return false;}}
export function LiveChat({accountId,current}:{accountId:string;current:boolean}) {
  const [conversationOpen,setConversationOpen]=useState(false);
  const [section,setSection]=useState<'filehelper'|'sessions'|'contacts'|'moments'|'search'|'favorites'|'records'>('filehelper');
  const [sources,setSources]=useState<{path:string;label:string;root:string}[]>([]),[source,setSource]=useState('');
  const [page,setPage]=useState<LivePage|MultiLivePage|null>(null),[busy,setBusy]=useState(false),[canceling,setCanceling]=useState(false),[error,setError]=useState('');
  const generation=useRef(0),request=useRef<string|null>(null),expires=useRef(0),cancelRequested=useRef(false);
  const nextSection=useRef<typeof section>('filehelper');
  useEffect(()=>{generation.current++;setPage(null);setSources([]);setSource('');setBusy(false);setCanceling(false);setError('');setSection('filehelper');setConversationOpen(false);nextSection.current='filehelper';
    return()=>{generation.current++;cancelRequested.current=true;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});};
  },[accountId,current]);
  useEffect(()=>{if(!page)return;const timer=window.setInterval(()=>{if(Date.now()>=expires.current){
    generation.current++;cancelRequested.current=true;
    const id=request.current;request.current=null;
    if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});
    setBusy(false);setCanceling(false);setPage(null);setError('在线窗口已过期，请重新读取。');
  }},1000);return()=>window.clearInterval(timer);},[page]);
  useEffect(()=>{if(page){setSection(nextSection.current);nextSection.current='filehelper';}setConversationOpen(false);},[page?.view_id]);
  async function automatic(single=false,target:typeof section='sessions'){
    if(request.current)return;const id=++generation.current,requestId=crypto.randomUUID();request.current=requestId;cancelRequested.current=false;
    setBusy(true);setCanceling(false);setError('');setPage(null);
    try{const result=await waitForOnlineRead(()=>command<LivePage|MultiLivePage>('native_live_auto_prepare',{accountId,requestId,single}),()=>id!==generation.current||cancelRequested.current);
      if(!result||id!==generation.current||cancelRequested.current)return;nextSection.current=target;expires.current=Date.now()+result.expires_in_seconds*1000;setPage(result);
    }catch(e){if(id===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(id===generation.current){request.current=null;setBusy(false);setCanceling(false);}}
  }
  async function cancel(){if(request.current){cancelRequested.current=true;setCanceling(true);await command('cancel_browse',{requestId:request.current}).catch(e=>setError(String(e)));}}
  async function detect(){const id=++generation.current;setBusy(true);setError('');setPage(null);
    try {const catalog=await command<DataSourceCatalog>('discover_data_sources',{root:null,confirmed:true});if(id!==generation.current)return;
      const found=catalog.sources.flatMap(s=>s.databases.filter(d=>/\/message_\d+\.db$/.test(d.relative_path)&&d.wal_present).map(d=>({root:s.account_directory,path:`${s.account_directory}/${d.relative_path}`,label:`${s.directory_name} / ${d.relative_path}`}))).sort((a,b)=>b.path.localeCompare(a.path,undefined,{numeric:true}));
      setSources(found);setSource(found[0]?.path??'');if(!found.length)setError('没有找到可在线读取的消息来源。');
    }catch(e){if(id===generation.current)setError(e instanceof Error?e.message:String(e));}finally{if(id===generation.current)setBusy(false);}}
  async function read(cursor?:string,direction?:'older'|'newer',continuation=false,multi=false){
    const combined=cursor?page?.mode==='live_multi_source_window':multi;
    const id=++generation.current;cancelRequested.current=false;setBusy(true);setCanceling(false);setError('');
    const requestId=crypto.randomUUID();request.current=cursor&&!continuation?null:requestId;
    if(!cursor||continuation)setPage(null);
    try{const result:LivePage|MultiLivePage=combined?(continuation?await command<MultiLivePage>('native_live_multi_continue',{accountId,cursor,requestId}):cursor?await command<MultiLivePage>('native_live_multi_page',{accountId,cursor,direction}):await command<MultiLivePage>('native_live_multi_prepare',{accountId,databases:group.map(s=>s.path),requestId,confirmed:true})):(continuation?await command<LivePage>('native_live_continue',{accountId,cursor,requestId}):cursor?await command<LivePage>('native_live_page',{accountId,cursor,direction}):await command<LivePage>('native_live_prepare',{accountId,database:source,requestId,confirmed:true}));
      if(id!==generation.current||cancelRequested.current)return;expires.current=Date.now()+result.expires_in_seconds*1000;setPage(result);
    }catch(e){if(id===generation.current){setPage(null);setError(e instanceof Error?e.message:String(e));}}finally{if(id===generation.current){request.current=null;setBusy(false);setCanceling(false);}}}
  const root=sources.find(s=>s.path===source)?.root;
  const group=sources.filter(s=>s.root===root);
  const rows=page?(page.mode==='live_multi_source_window'?page.rows.map(r=>({row:r.message,key:`${r.source}:${r.message.local_id}`,label:` / 分片${r.source_index}`})):page.rows.map(r=>({row:r,key:r.local_id,label:''}))):[];
  if(!current)return <><p className="empty-inline">在线读取只用于当前微信，旧资料请使用已导入快照或补读记录。</p><RecoveryPanel accountId={accountId} viewId={null} disabled={false}/></>;
  return <section className="live-chat" aria-label="在线文件传输助手">
    <header className="conversation-header"><div><h2>在线会话与资料</h2></div><HelpTip label="在线消息说明">只读当前登录微信的本机数据，不改变微信已读状态。自动核验账号并准备当前来源；不需要导入所有历史，也不需要手工输入路径。若发现多个来源目录，请展开高级来源选择一次。</HelpTip></header>
    <div className="actions"><button className="button primary" disabled={busy||!desktopAvailable} onClick={()=>void automatic()}>{busy?'正在准备在线资料…':page?'刷新在线会话':'打开在线会话'}</button>{busy&&request.current&&<button className="button" disabled={canceling} onClick={()=>void cancel()}>{canceling?'正在取消…':'取消读取'}</button>}</div>
    <details><summary>高级 · 手工选择来源</summary><div className="actions"><button className="button" disabled={busy||!desktopAvailable} onClick={()=>void detect()}>检测消息来源</button>
      <label>消息来源<select aria-label="在线消息来源" value={source} disabled={busy||!sources.length} onChange={e=>{setSource(e.target.value);setPage(null);}}><option value="">请选择</option>{sources.map(s=><option key={s.path} value={s.path}>{s.label}</option>)}</select></label>
      <button className="button primary" disabled={busy||!source||!desktopAvailable} onClick={()=>void read()}>只读准备并查看</button>
      <button className="button" disabled={busy||group.length<2||group.length>16||!desktopAvailable} onClick={()=>void read(undefined,undefined,false,true)}>合并当前目录分片（{group.length}）</button>
    </div>
    <HelpTip label="如何准备在线消息">先连接当前微信原生通道，再检测并选择来源。首次使用会准备所选来源的密钥，无需导入全部历史；准备期间取消可能需要等待取钥结束。默认只读文件传输助手，准备后可打开会话列表或联系人资料。</HelpTip></details>
    {error&&<p className="error-notice" role="alert">{error}</p>}
    {page&&section!=='sessions'&&<CurrentProfile accountId={accountId} viewId={page.view_id}/>}
    {page&&<div className="live-workspace-nav" role="group" aria-label="在线资料分类">
      {([['sessions','会话'],['search','跨会话搜索'],['contacts','联系人'],['moments','朋友圈'],['favorites','微信收藏'],['records','专项记录'],['filehelper','文件传输助手']] as const).map(([id,label])=><button type="button" className={section===id?'button primary':'button'} key={id} aria-pressed={section===id} disabled={busy} onClick={()=>{if(id==='moments'&&page.mode==='live_multi_source_window')void automatic(true,'moments');else setSection(id);}}>{label}</button>)}
      <HelpTip label="在线资料分类说明">选择一个分类再查看资料，切换时会关闭当前聊天并清除临时显示。联系人支持按昵称、备注和账号查找；会话支持账号或本地摘要查找；跨会话搜索直接查找当前所选消息分片的正文和类型，并可定位原消息。打开朋友圈时自动准备单个认证来源；返回合并会话可点击上方刷新。</HelpTip>
    </div>}
    {page&&section==='filehelper'&&<div className="actions"><button className="button primary" disabled={busy} onClick={()=>setConversationOpen(v=>!v)}>{conversationOpen?'返回基础窗口':'查找消息与查看媒体'}</button><HelpTip label="完整会话入口说明">打开文件传输助手的完整会话视图，可按日期、文字、类型和发送者查找，定位消息，按需读取图片、视频、语音、表情与文件。搜索覆盖所选来源，不限于当前显示的60条。</HelpTip></div>}
    {page&&section==='filehelper'&&conversationOpen&&<ConversationMessages key={`filehelper:${accountId}:${page.view_id}`} accountId={accountId} parentViewId={page.view_id} username="filehelper" multi={page.mode==='live_multi_source_window'} onClose={()=>setConversationOpen(false)}/>}
    {page&&section==='filehelper'&&!conversationOpen&&<><div className="timeline-scroll" tabIndex={0} aria-label="在线消息窗口"><ol className="message-timeline">{rows.map(({row,key,label})=><li key={key}><div className="message-heading"><LiveSender username={row.sender_username}/>{label&&<span>{label}</span>}<time>{safeTime(row.timestamp)}</time></div><div className="message-bubble"><span className="message-body" dir="auto">{row.text??(isEmoticon(row.message_type,row.metadata)?"表情消息":isVoice(row.message_type)?"语音消息":row.content_status.startsWith("metadata")?"文件或应用消息":missingContent(row.content_status))}</span>{page.mode!=="live_multi_source_window"&&row.content_status.startsWith("metadata")&&!isEmoticon(row.message_type,row.metadata)&&<LiveFile accountId={accountId} viewId={page.view_id} localId={row.local_id}/>}{page.mode!=="live_multi_source_window"&&isEmoticon(row.message_type,row.metadata)&&<ReceivedEmoticon key={`${accountId}:${page.view_id}:${row.local_id}`} accountId={accountId} viewId={page.view_id} localId={row.local_id}/>}{page.mode!=="live_multi_source_window"&&isVoice(row.message_type)&&<LiveVoice key={`${accountId}:${page.view_id}:${row.local_id}`} accountId={accountId} viewId={page.view_id} localId={row.local_id}/>}</div></li>)}</ol>{!rows.length&&<p className="chat-empty">所选来源内暂无文件传输助手消息。</p>}</div>
      {page.mode==='live_multi_source_window'&&<p className="field">已合并{page.source_count}个明确分片；逐分片短事务，非同一时刻快照。不同分片的相同消息标识分别保留。</p>}
      <div className="actions"><button className="button" disabled={busy||!page.newer} onClick={()=>void read(page.newer!,'newer')}>较新消息</button><button className="button" disabled={busy||!page.older} onClick={()=>void read(page.older!,'older')}>较早消息</button><span className="field">每页60条 / 新消息在前 / 窗口保留约2分钟</span></div>
      {page.continuation&&!page.older&&<div className="actions"><button className="button" disabled={busy} onClick={()=>void read(page.continuation!,undefined,true)}>读取更早窗口</button><span className="field">所选来源还有窗口以外的消息；继续读取会替换当前窗口。</span></div>}</>}
    {page&&section==='search'&&<LiveSearch key={`search:${accountId}:${page.view_id}`} accountId={accountId} viewId={page.view_id} multi={page.mode==='live_multi_source_window'}/>}
    {page&&section==='contacts'&&<LiveContacts embedded key={`${accountId}:${page.view_id}`} accountId={accountId} viewId={page.view_id} multi={page.mode==='live_multi_source_window'}/>}
    {page&&section==='sessions'&&<LiveSessions embedded autoRead key={`sessions:${accountId}:${page.view_id}`} accountId={accountId} viewId={page.view_id} multi={page.mode==='live_multi_source_window'}/>}
    {page&&section==='records'&&<LiveSpecialRecords key={`records:${accountId}:${page.view_id}`} accountId={accountId} viewId={page.view_id}/>}
    {page&&section==='favorites'&&<LiveFavorites key={`favorites:${accountId}:${page.view_id}`} accountId={accountId} viewId={page.view_id}/>}
    {page?.mode==='live_bounded_result_view'&&section==='moments'&&<LiveMoments embedded key={`moments:${accountId}:${page.view_id}`} accountId={accountId} viewId={page.view_id}/>}
    <details className="live-read-scope"><summary>高级 · 读取范围</summary><p className="field">每个消息分片每次最多读取2048条。窗口内翻页使用本次结果，窗口外继续读取会核验边界并建立新的只读视图；不同窗口不代表同一时刻的快照。查看最新消息可点击“刷新在线会话”。自动合并当前认证目录最多16个在线分片；未检测或不可读取的分片不在结果内。打开完整会话后，可按需读取图片、视频、语音、表情和文件，保存的媒体加密存储。这里不会清除实时接收中的缺口，也不代表服务端回执。</p></details>
    <RecoveryPanel accountId={accountId} viewId={page?.view_id??null} multi={page?.mode==='live_multi_source_window'} disabled={busy}/>
    <CapturePanel accountId={accountId} viewId={page?.view_id}/>
  </section>;
}
