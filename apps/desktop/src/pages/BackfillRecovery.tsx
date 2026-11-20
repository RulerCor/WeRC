import {useEffect,useRef,useState} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
interface Status {source_count:number;pending_sources:number;pages:number;rows:number;observed_through:number|null;errors:string[]}
interface Row {id:string;username:string;message:{timestamp:number;text:string|null;message_type:string;server_id:string|null}}
interface Page {data:{position:number;rows:Row[]}[];next:number;has_more:boolean;status:Status}
export function BackfillRecovery({accountId}:{accountId:string}) {
  const [status,setStatus]=useState<Status|null>(null),[open,setOpen]=useState(false),[page,setPage]=useState<Page|null>(null);
  const [busy,setBusy]=useState(false),[error,setError]=useState('');
  const [previous,setPrevious]=useState<number[]>([]),[after,setAfter]=useState(0);
  const generation=useRef(0);
  useEffect(()=>{let alive=true,pending=false;const refresh=async()=>{
    if(pending||!desktopAvailable)return;pending=true;
    try {const value=await command<Status>('native_backfill_status',{accountId});if(alive)setStatus(value);}
    catch(e){if(alive)setError(e instanceof Error?e.message:String(e));}finally{pending=false;}
  };void refresh();const timer=window.setInterval(()=>void refresh(),5000);return()=>{alive=false;generation.current++;window.clearInterval(timer);};},[accountId]);
  async function load(cursor:number,history:number[]) {
    const stamp=++generation.current;setBusy(true);setError('');
    try {const value=await command<Page>('native_backfill_page',{accountId,after:cursor});if(stamp===generation.current){setPage(value);setStatus(value.status);setAfter(cursor);setPrevious(history);}}
    catch(e){if(stamp===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(stamp===generation.current)setBusy(false);}
  }
  return <details className="outbox-compose" open={open} onToggle={e=>{const value=e.currentTarget.open;setOpen(value);if(value&&!page&&!busy)void load(0,[]);}}>
    <summary>自动补读 {status?status.source_count?`· 已保存 ${status.rows.toLocaleString()} 条记录`:'· 等待消息来源':''} <HelpTip label="自动补读说明">连接微信后，后台会自动检测当前登录账号的消息来源，逐个准备并保存历史与后续消息，断连或退出后接着读取。补读页可在 Bot 端通过 messages/recovered 及其 stream 接口接收。重叠扫描可能再次观察同一条消息，客户端可按 id 与 content_digest 更新。每天重新扫描旧消息；已被微信删除或不在已发现来源中的消息无法保证恢复。这里不会修改原生收信确认或发送状态。</HelpTip></summary>
    {open&&<>
      {status?.source_count===0&&<p className="field">连接微信后自动检测来源。若检测到多个同账号目录，请在“在线消息”中选择正确来源并读取一次。</p>}
      {Boolean(status?.source_count)&&<p className="field">{status!.source_count} 个来源 · {status!.pending_sources?'正在补读':'等待新消息'}{status!.observed_through?` · 已扫描至 ${new Date(status!.observed_through*1000).toLocaleString('zh-CN')}`:''}</p>}
      {status?.errors.map(code=><p key={code} className="error-notice">补读暂未继续：{code}。已有记录保留；请检查当前连接与消息来源。</p>)}
      {error&&<p className="error-notice" role="alert">{error}</p>}
      <div className="actions"><button className="button" disabled={busy||!previous.length} onClick={()=>void load(previous[previous.length-1],previous.slice(0,-1))}>上一页</button><button className="button" disabled={busy} onClick={()=>void load(after,previous)}>刷新本页</button><button className="button" disabled={busy||!page?.has_more} onClick={()=>void load(page!.next,[...previous,after])}>下一页</button><button className="button" disabled={busy||!status?.pages} onClick={()=>void load(Math.max(0,(status?.pages??1)-1),[])}>最新补读</button></div>
      <ol className="message-timeline" aria-label="自动补读记录">{page?.data.flatMap(batch=>batch.rows.map(row=><li key={`${batch.position}:${row.id}`}><div className="message-heading"><strong>{row.username}</strong><time>{new Date(row.message.timestamp*1000).toLocaleString('zh-CN')}</time></div><div className="message-bubble"><span className="message-body" dir="auto">{row.message.text??`消息类型 ${row.message.message_type}`}</span></div><details><summary>消息信息</summary><p className="field">服务器编号：{row.message.server_id??'未提供'}</p></details></li>))}</ol>
      {!busy&&page&&!page.data.length&&<p className="empty-inline">还没有保存补读记录。</p>}
    </>}
  </details>;
}
