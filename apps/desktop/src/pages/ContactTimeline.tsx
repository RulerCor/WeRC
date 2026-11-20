import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';

type Cursor={timestamp:number;page:number;row:number;snapshot:string};
type Item={page:number;row:number;timestamp:number;username:string;sender:string|null;message_type:string;text:string|null;local_status:string;direction:string;mentions_me:boolean;mentions_all:boolean;pending:string|null};
type Timeline={data:Item[];next:Cursor|null;summary:{sent_contacts:number;received_contacts:number;waiting_me:number;waiting_them:number;mentions:number;mention_groups:number;group_rows_without_metadata:number};complete_selected_sources:boolean;scanned:number;duplicates_skipped:number};
const modes=[['all','全部联系'],['private','私聊'],['waiting_me','等我回应'],['waiting_them','还没回音'],['mentions','群聊 @我']] as const;
export function ContactTimeline({accountId,id,disabled=false,onLocate}:{accountId:string;id:string;disabled?:boolean;onLocate:(p:{page:number;row:number})=>void}) {
  const [mode,setMode]=useState('all'),[text,setText]=useState(''),[username,setUsername]=useState(''),[from,setFrom]=useState(''),[until,setUntil]=useState('');
  const [data,setData]=useState<Timeline|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const request=useRef<string|null>(null),generation=useRef(0);
  useEffect(()=>{setData(null);setError('');setBusy(false);setText('');setUsername('');setFrom('');setUntil('');setMode('all');return()=>{generation.current++;const pending=request.current;request.current=null;if(pending)void command('cancel_browse',{requestId:pending}).catch(()=>{});};},[accountId,id]);
  function change(action:()=>void){action();setData(null);setError('');}
  async function load(after:Cursor|null=null) {
    if(request.current||disabled)return;
    const g=generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');
    try {
      const start=from?Math.floor(new Date(from).getTime()/1000):null,end=until?Math.floor(new Date(until).getTime()/1000):null;
      if((start!==null&&!Number.isFinite(start))||(end!==null&&!Number.isFinite(end))||(start!==null&&end!==null&&start>=end))throw Error('请选择有效的起止时间，结束时间应晚于开始时间。');
      const result=await command<Timeline>('capture_timeline',{accountId,id,query:{mode,text,username:username.trim()||null,from:start,until:end},after,requestId});
      if(g===generation.current)setData(old=>after&&old?{...result,data:[...old.data,...result.data]}:result);
    } catch(e){if(g===generation.current)setError(String(e));}
    finally{if(g===generation.current){request.current=null;setBusy(false);}}
  }
  return <details className="local-tools"><summary>联络时间线与群聊 @我</summary>
    <div className="history-query"><label>查看范围<select disabled={busy||disabled} value={mode} onChange={e=>change(()=>setMode(e.target.value))}>{modes.map(([key,label])=><option key={key} value={key}>{label}</option>)}</select></label>
      <label>搜索消息<input disabled={busy||disabled} value={text} maxLength={512} placeholder="在全部已保存文字中查找" onChange={e=>change(()=>setText(e.target.value))}/></label>
      <button className="button primary" disabled={busy||disabled} onClick={()=>void load()}>查看时间线</button>
      <HelpTip label="联络时间线说明">只读取当前归档，按消息时间倒序排列，可跳回原消息。“等我回应”和“还没回音”依据所选时间内每个私聊最新一条非系统消息的已知方向，不判断是否应当回复，也不影响 Bot 转发。未知身份不会计入待回复。群聊 @我要求来源中明确保存了当前登录账号或所有人标记，不按昵称猜测。</HelpTip>
    </div>
    <details><summary>指定联系人与时间</summary><div className="history-query">
      <label>会话账号<input value={username} disabled={busy||disabled} placeholder="留空查看全部会话" onChange={e=>change(()=>setUsername(e.target.value))}/></label>
      <label>开始时间<input type="datetime-local" value={from} disabled={busy||disabled} onChange={e=>change(()=>setFrom(e.target.value))}/></label>
      <label>结束时间<input type="datetime-local" value={until} disabled={busy||disabled} onChange={e=>change(()=>setUntil(e.target.value))}/></label>
    </div></details>
    {data&&<>
      <p role="status">发过消息 {data.summary.sent_contacts} 人 · 收到消息 {data.summary.received_contacts} 人 · 等我回应 {data.summary.waiting_me} 人 · 还没回音 {data.summary.waiting_them} 人 · 群聊提及 {data.summary.mentions} 条 / {data.summary.mention_groups} 个群</p>
      {!data.complete_selected_sources&&<p className="field">当前归档尚未补读完成，时间线只含已保存部分。</p>}
      {data.summary.group_rows_without_metadata>0&&<p className="field">{data.summary.group_rows_without_metadata} 条群消息未保存可读提及字段，无法据此判断是否 @我。<HelpTip label="提及字段覆盖说明">旧版本归档、来源缺少 source 列或字段无法完整解析时保留未知。重新补读可读取当前本地来源可用的提及字段，无法恢复来源本身已缺失的数据。</HelpTip></p>}
      <ol className="recovery-messages" aria-label="联络消息时间线">{data.data.map(item=><li key={`${item.page}:${item.row}`}>
        <div className="message-body">{item.username} · {new Date(item.timestamp*1000).toLocaleString()}</div>
        <div className="field">{item.direction==='outgoing'?'我发出':item.direction==='incoming'?'收到':'发送者未知'}{item.sender&&` · ${item.sender}`}</div>
        {item.mentions_me&&<span className="state-tag">@我</span>}{item.mentions_all&&<span className="state-tag">@所有人</span>}{item.pending&&<span className="state-tag">{item.pending==='waiting_me'?'等我回应':'还没回音'}</span>}
        {['edited_local','added_local'].includes(item.local_status)&&<span className="state-tag">{item.local_status==='edited_local'?'本地已修改':'本地补充'}</span>}
        <p className="message-body">{item.text??`[类型 ${item.message_type}，打开来源查看内容]`}</p>
        <button className="button" disabled={busy||disabled} onClick={()=>onLocate({page:item.page,row:item.row})}>定位原消息</button>
      </li>)}</ol>
      {!data.data.length&&<p>没有符合条件的消息。</p>}
      {data.next&&<button className="button" disabled={busy||disabled} onClick={()=>void load(data.next)}>更早的联系</button>}
    </>}
    {busy&&<button className="button" onClick={()=>{if(request.current)void command('cancel_browse',{requestId:request.current}).catch(e=>setError(String(e)));}}>取消时间线读取</button>}
    {error&&<p className="error-notice" role="alert">{error}</p>}
  </details>;
}
