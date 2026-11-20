import {useEffect,useRef,useState} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {MessageCard,type ConversationCard} from '../components/MessageCard';
import {conversationName} from '../lib/native-receive';

interface Status {conversations:string[];scanned_position:number;saved:number;unrecognized_notices:number}
interface Item {id:string;conversation:string;original_server_id:string;notice_at:string;notice_position:number;state:'original_saved'|'original_not_observed';message_type:number|null;text:string|null;original_observed_at:string|null;card:ConversationCard['content']|null}
interface Page {data:Item[];next_cursor:number|null;scanned:number}

/** Mounted only while open; key by account to discard all old account state. */
export function MessagePreservation({accountId,conversation}:{accountId:string;conversation:string}) {
  const [status,setStatus]=useState<Status|null>(null),[page,setPage]=useState<Page|null>(null);
  const [state,setState]=useState(''),[onlyCurrent,setOnlyCurrent]=useState(Boolean(conversation));
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const generation=useRef(0),running=useRef(false);
  useEffect(()=>{const g=++generation.current;setStatus(null);setPage(null);setOnlyCurrent(Boolean(conversation));setError('');setNotice('');setBusy(false);running.current=false;
    void command<Status>('message_preservation_status',{accountId}).then(value=>{if(g===generation.current)setStatus(value);}).catch(e=>{if(g===generation.current)setError(String(e));});
    return()=>{generation.current++;};
  },[accountId,conversation]);
  async function run(action:(g:number)=>Promise<void>){if(running.current)return;running.current=true;const g=generation.current;setBusy(true);setError('');setNotice('');try{await action(g);}catch(e){if(g===generation.current)setError(String(e));}finally{if(g===generation.current){running.current=false;setBusy(false);}}}
  function read(before:number|null=null){void run(async g=>{const result=await command<Page>('preserved_messages',{accountId,before,conversation:onlyCurrent?conversation:null,status:state||null});if(g===generation.current)setPage(old=>before!==null&&old?{...result,data:[...old.data,...result.data]}:result);});}
  function toggle(target:string){void run(async g=>{const enabled=!status?.conversations.includes(target);const result=await command<Status>('configure_message_preservation',{accountId,conversation:target,enabled});if(g===generation.current){setStatus(result);setNotice(enabled?'已开启，后台会处理已保存的接收记录。':'已暂停；已有副本继续保留。');}});}
  return <section className="local-tools" aria-label="消息撤回留存">
    <div className="section-title"><h3>消息撤回留存</h3><HelpTip label="撤回留存说明">为指定会话开启后，WeRC 会在已加密保存的原生接收记录中，按微信原消息编号精确关联撤回通知并独立保留原文或消息卡片。无法匹配时显示“未观察到原文”。它不修改微信、不阻止服务端撤回，也不筛选 Bot 消息流。已下载媒体仍由原接收记录管理；这里不承诺恢复未保存或已失效的媒体。暂停保留已有副本。软件关闭时不运行。</HelpTip></div>
    <div className="actions"><button className="button" disabled={!desktopAvailable||busy||!status||!conversation} onClick={()=>toggle(conversation)}>{conversation&&status?.conversations.includes(conversation)?'暂停当前会话留存':'为当前会话开启留存'}</button><button className="button" disabled={!desktopAvailable||busy} onClick={()=>void run(async g=>{const result=await command<Status>('step_message_preservation',{accountId});if(g===generation.current){setStatus(result);setNotice('已处理一批接收记录，可刷新查看副本。');}})}>处理与刷新状态</button></div>
    {!conversation&&<p className="field">先在上方选择一个会话，再开启留存。</p>}
    {status&&<p className="field" role="status">已保留 {status.saved} 条 · 已处理到接收位置 {status.scanned_position}{status.unrecognized_notices>0&&<HelpTip label="无法关联的通知">有 {status.unrecognized_notices} 条通知缺少可验证的原消息编号或会话不匹配，未作为已保留原文处理。</HelpTip>}</p>}
    {Boolean(status?.conversations.length)&&<details><summary>管理已开启会话（{status?.conversations.length}）</summary><ul className="history-messages">{status?.conversations.map(target=><li key={target}><span className="message-body">{conversationName(target)}</span><button className="button" disabled={busy} onClick={()=>toggle(target)}>暂停</button></li>)}</ul></details>}
    <div className="history-query"><label>保留状态<select value={state} disabled={busy} onChange={e=>{setState(e.target.value);setPage(null);}}><option value="">全部状态</option><option value="original_saved">已保留原文</option><option value="original_not_observed">未观察到原文</option></select></label>{conversation&&<label><input type="checkbox" checked={onlyCurrent} disabled={busy} onChange={e=>{setOnlyCurrent(e.target.checked);setPage(null);}}/>只看当前会话</label>}<button className="button" disabled={busy||!desktopAvailable} onClick={()=>read()}>查看保留记录</button></div>
    {error&&<p className="error-notice" role="alert">{error}</p>}{notice&&<p role="status">{notice}</p>}
    {page&&<><ul className="history-messages" aria-label="已保留撤回消息">{page.data.map(item=><li key={item.id}><div className="message-heading"><strong className="message-body">{conversationName(item.conversation)}</strong><time>{new Date(item.notice_at).toLocaleString('zh-CN')}</time></div>{item.state==='original_not_observed'?<p className="field">未观察到原文</p>:item.text!==null?<p className="message-body" dir="auto">{item.text}</p>:item.card?<MessageCard card={item.card}/>:<p className="field">已保存消息元数据（类型 {item.message_type}），无可显示正文。</p>}<details><summary>关联信息</summary><p className="field">原消息编号：{item.original_server_id} · 撤回通知接收位置：{item.notice_position}</p>{item.original_observed_at&&<p className="field">原消息本机保存时间：{new Date(item.original_observed_at).toLocaleString('zh-CN')}</p>}</details></li>)}</ul>{!page.data.length&&<p className="empty-inline">{page.next_cursor!==null?'本页没有匹配记录，可继续读取。':'暂无匹配记录。'}</p>}{page.next_cursor!==null&&<button className="button" disabled={busy} onClick={()=>read(page.next_cursor)}>继续读取</button>}</>}
  </section>;
}
