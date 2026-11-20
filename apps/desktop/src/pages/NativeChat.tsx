import {MessageCard} from '../components/MessageCard';
import type {NotificationNavigation} from '../components/DesktopNotification';
import {BackfillRecovery} from './BackfillRecovery';
import {conversationName,observationDirection,type NativeRow as Row} from '../lib/native-receive';
import {isEmoticon} from '../lib/emoticon';
import {ReceivedEmoticon} from './ReceivedEmoticon';
import {useEffect,useState} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
import type {Workbench} from '../hooks/useWorkbench';
import type {NativeStatus} from './NativeConnection';
import {Outbox} from './Outbox';
import {ReceivedImage} from './ReceivedImage';
import {ReceivedVoice} from './ReceivedVoice';
import {ReceivedFile} from './ReceivedFile';
import {ReceivedVideo} from './ReceivedVideo';
import {HelpTip} from '../components/HelpTip';
import {MessagePreservation} from './MessagePreservation';
interface Page {data:Row[];cursor:{gaps:number}|null;historical_complete:false;data_mode:string}
const kinds:Record<number,string>={3:"图片",34:"语音",37:"好友验证",42:"联系人名片",48:"位置",50:"通话记录",43:"视频",47:"表情",49:"文件或应用消息",62:"短视频",10000:"系统消息",10002:"撤回通知"};
export function NativeChat({accountId,focus,...workbench}:Workbench&{accountId:string;focus?:NotificationNavigation|null}) {
  const [filter,setFilter]=useState('');
  const [preservationOpen,setPreservationOpen]=useState(false);
  useEffect(()=>{setFilter(focus?.account===accountId&&focus.page==='history'?focus.conversation:'');},[accountId,focus]);
  const [rows,setRows]=useState<Row[]>([]),[gaps,setGaps]=useState(0),[error,setError]=useState(''),[status,setStatus]=useState<NativeStatus|null>(null);
  useEffect(()=>{if(!desktopAvailable)return;setRows([]);let alive=true,pending=false,after=focus?.account===accountId&&focus.position>0?focus.position-1:-1;
    const poll=async()=>{if(pending)return;pending=true;try{
      const [page,s]=await Promise.all([command<Page>('native_messages',{accountId,after,limit:100}),command<NativeStatus>('native_status')]);
      if(!alive)return;setStatus(s);setGaps(page.cursor?.gaps??0);setError('');
      if(page.data.length){after=page.data[page.data.length-1].position;setRows(old=>[...old,...page.data.filter(r=>!old.some(o=>o.position===r.position))].slice(-100));}
    }catch(e){if(alive)setError(e instanceof Error?e.message:String(e));}finally{pending=false;}};
    void poll();const timer=window.setInterval(()=>void poll(),1000);return()=>{alive=false;window.clearInterval(timer);};
  },[accountId,focus?.position]);
  const conversations=[...new Set(rows.map(row=>row.payload.target_id))];
  const visible=filter?rows.filter(row=>row.payload.target_id===filter):rows;
  return <section aria-label="原生实时接收" className="native-chat"><header className="conversation-header"><div><h2>实时接收</h2><p className="field">{status?.connected&&status.account_id===accountId?'原生接收已连接':'接收未连接 / 在“当前微信”页连接'}</p></div><span className="state-tag">已保存的接收记录 <HelpTip label="接收记录说明">显示当前配置最近100条原生观测；会话筛选只作用于这100条。群会话标识不是群成员作者，旧记录没有方向信息。时间是本机保存时间。这里的记录不代表已补齐完整会话，也不是服务器发送回执。要查看连接前的消息，请切换到在线最近消息。</HelpTip></span></header>
    <label className="field native-conversation-filter">查看会话 <select aria-label="接收会话筛选" value={filter} onChange={e=>setFilter(e.target.value)}><option value="">全部最近会话</option>{conversations.map(target=><option key={target} value={target}>{conversationName(target)}</option>)}{filter&&!conversations.includes(filter)&&<option value={filter}>{conversationName(filter)}（不在最近窗口内）</option>}</select></label>
    {gaps>0&&<p className="field" role="status">有 {gaps} 个连接前、断连或溢出区间尚未补齐。</p>}
    <BackfillRecovery key={accountId} accountId={accountId}/>
    <details onToggle={e=>setPreservationOpen(e.currentTarget.open)}><summary>本机撤回留存</summary>{preservationOpen&&<MessagePreservation key={accountId} accountId={accountId} conversation={filter}/>}</details>
    {error&&<p className="error-notice" role="alert">{error}</p>}
    {status?.error_code&&<p className="error-notice">{status.error_code==='NATIVE_ADAPTER_PENDING'?'当前微信版本的原生通道待适配':status.error_code==='NATIVE_BUILD_UNSUPPORTED'?'当前微信构建暂不支持原生通道':'连接已停止'} <HelpTip label="连接状态详情">错误码：{status.error_code}。请在连接页查看支持范围与诊断。</HelpTip></p>}
    <div className="timeline-scroll" tabIndex={0} aria-label="原生接收记录"><ol className="message-timeline">{visible.map(row=><li key={row.payload.id}><div className="message-heading"><strong className="native-observation-label">{conversationName(row.payload.target_id)} <HelpTip label="消息会话说明">{observationDirection(row)}。会话标识：{row.payload.target_id}。{row.payload.route?.conversation.endsWith('@chatroom')?'群成员作者尚未单独核验。':''}</HelpTip></strong><time>{new Date(row.payload.observed_at).toLocaleString('zh-CN')}</time></div><div className="message-bubble">{row.card?<MessageCard card={row.card}/>:<span className="message-body" dir="auto">{(row.payload.message.message_type??1)===1?row.payload.message.text:`${kinds[row.payload.message.message_type!]??"消息"} · ${(row.payload.message.message_type===3||row.payload.message.message_type===43)?"可按需下载":(row.payload.message.message_type===34||isEmoticon(row.payload.message.message_type,row.payload.message.metadata))?"可查看本机缓存":row.payload.message.message_type===49?"普通附件可按需下载":"已保存元数据，内容未展开"}`}</span>}{isEmoticon(row.payload.message.message_type,row.payload.message.metadata)&&<ReceivedEmoticon key={`${accountId}/${row.position}`} accountId={accountId} position={row.position} connected={Boolean(status?.connected&&status.account_id===accountId)}/>}{row.payload.message.message_type===43&&<ReceivedVideo key={`${accountId}/${row.position}`} accountId={accountId} position={row.position} connected={Boolean(status?.connected&&status.account_id===accountId)}/>}{row.payload.message.message_type===3&&<ReceivedImage key={`${accountId}/${row.position}`} accountId={accountId} position={row.position} connected={Boolean(status?.connected&&status.account_id===accountId)}/>}{row.payload.message.message_type===49&&!row.card&&!isEmoticon(row.payload.message.message_type,row.payload.message.metadata)&&<ReceivedFile key={`${accountId}/${row.position}`} accountId={accountId} position={row.position} connected={Boolean(status?.connected&&status.account_id===accountId)}/>}{row.payload.message.message_type===34&&<ReceivedVoice key={`${accountId}/${row.position}`} accountId={accountId} position={row.position}/>}</div></li>)}</ol>{!visible.length&&<p className="chat-empty">{filter?'最近100条记录中没有此会话的消息。':'连接当前微信后，新消息会自动保存在这里。'}</p>}</div>
    {accountId===workbench.data.current_connection.account_id&&<details className="outbox-compose"><summary>创建消息与查看发送队列</summary><Outbox {...workbench}/></details>}
  </section>;
}
