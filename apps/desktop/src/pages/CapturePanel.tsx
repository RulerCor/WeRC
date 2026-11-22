import {useEffect,useRef,useState} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {waitForOnlineRead} from '../lib/busy-read';
import type {LiveMessage} from './LiveChat';
import {CaptureTranscripts} from './CaptureTranscripts';
import {CaptureMedia} from './CaptureMedia';
import {CaptureMediaCatalog} from './CaptureMediaCatalog';
import {CaptureRecords} from './CaptureRecords';
import {ContactTimeline} from './ContactTimeline';
import {CaptureMessageLocator} from './CaptureMessageLocator';
import {CaptureTools} from './CaptureTools';
import {CaptureEditor,CaptureAdd} from './CaptureEdits';
export interface CaptureStatus {id:string;imported:boolean;can_resume:boolean;started_at:string;rows:number;pages:number;source_count:number;source_index:number;complete_selected_sources:boolean}
interface CapturePage {revision:number;rows:{username:string;source_index:number;message:LiveMessage}[];next_page:number|null}
export function CapturePanel({accountId,viewId,initiallyOpen=false}:{accountId:string;viewId?:string|null;initiallyOpen?:boolean}){
  const [opened,setOpened]=useState(initiallyOpen);
  const [items,setItems]=useState<CaptureStatus[]>([]),[next,setNext]=useState<number|null>(null),[active,setActive]=useState<CaptureStatus|null>(null),[page,setPage]=useState<CapturePage|null>(null),[index,setIndex]=useState(0);
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[from,setFrom]=useState(''),[until,setUntil]=useState('');
  const [focusRow,setFocusRow]=useState<number|null>(null);const panel=useRef<HTMLDetailsElement>(null);
  useEffect(()=>{if(focusRow===null||!page)return;const item=panel.current?.querySelector<HTMLElement>(`[data-capture-row="${focusRow}"]`);item?.scrollIntoView({block:"center",behavior:"instant"});item?.focus({preventScroll:true});},[page,index,focusRow]);
  const running=useRef(false),request=useRef<string|null>(null),generation=useRef(0);
  useEffect(()=>{setItems([]);setPage(null);setFocusRow(null);setActive(null);setNext(null);setBusy(false);setError('');return()=>{generation.current++;running.current=false;const pending=request.current;request.current=null;if(pending)void command('cancel_browse',{requestId:pending}).catch(()=>{});};},[accountId]);
  async function list(before:number|null=null){const g=generation.current;setBusy(true);setError('');try{const p=await command<{data:CaptureStatus[];next_cursor:number|null}>('capture_list',{accountId,before});if(g!==generation.current)return;setItems(old=>before?[...old,...p.data]:p.data);setNext(p.next_cursor);}catch(e){if(g===generation.current)setError(String(e));}finally{if(g===generation.current)setBusy(false);}}
  async function run(existing?:CaptureStatus){const g=generation.current;running.current=true;setBusy(true);setError('');setPage(null);try{
    const canceled=()=>!running.current||g!==generation.current;
    let current=existing??await waitForOnlineRead(()=>command<CaptureStatus>('capture_start',{accountId,viewId:viewId??null,from:from?Math.floor(new Date(from).getTime()/1000):null,until:until?Math.floor(new Date(until).getTime()/1000):null}),canceled);
    if(!current||g!==generation.current)return;setActive(current);
    while(running.current&&!current.complete_selected_sources&&g===generation.current){const requestId=crypto.randomUUID();request.current=requestId;const id:string=current.id;const next=await waitForOnlineRead(()=>command<CaptureStatus>('capture_step',{accountId,id,requestId}),canceled);if(g!==generation.current)return;request.current=null;if(!next)break;current=next;setActive(current);}
    if(g===generation.current)setItems(old=>[current,...old.filter(i=>i.id!==current.id)]);
  }catch(e){if(g===generation.current&&running.current)setError(String(e));}finally{if(g===generation.current){running.current=false;request.current=null;setBusy(false);}}}
  function stop(){running.current=false;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(e=>setError(String(e)));}
  async function read(item:CaptureStatus,n:number,locatedRow:number|null=null){const g=generation.current;setBusy(true);setError('');try{const p=await command<CapturePage>('capture_page',{accountId,id:item.id,index:n});if(g===generation.current){setActive(item);setPage(p);setIndex(n);setFocusRow(locatedRow);}}catch(e){if(g===generation.current)setError(String(e));}finally{if(g===generation.current)setBusy(false);}}
  async function added(){if(!active)return;const g=generation.current;try{const status=await command<CaptureStatus>('capture_status',{accountId,id:active.id});if(g!==generation.current)return;setActive(status);setItems(old=>old.map(i=>i.id===status.id?status:i));await read(status,status.pages-1);}catch(e){if(g===generation.current)setError(String(e));}}
  return <details ref={panel} className="recovery-panel local-tools" open={opened} onToggle={e=>setOpened(e.currentTarget.open)}><summary>全会话补读与归档</summary>
    <div className="actions"><button className="button" disabled={busy||!desktopAvailable} onClick={()=>void list()}>查看补读任务</button><HelpTip label="全会话补读说明">读取所选消息分片的所有会话，每页和进度一起加密保存。暂停、关闭软件或重新连接同一账号后可继续。开始时固定结束时间；本地已删除、尚未下载或从未同步的内容无法由本地来源补出。此处的保存页不会清除原生接收缺口，也不充当发送回执。</HelpTip></div>
    <details><summary>新建补读</summary><div className="history-query"><label>开始时间<input type="datetime-local" value={from} disabled={busy} onChange={e=>setFrom(e.target.value)}/></label><label>结束时间<input type="datetime-local" value={until} disabled={busy} onChange={e=>setUntil(e.target.value)}/></label><button className="button primary" disabled={busy||!desktopAvailable} onClick={()=>void run()}>读取全部会话并保存</button></div></details>
    {running.current&&<button className="button" onClick={stop}>暂停并保留进度</button>}
    {active&&<p role="status">已保存 {active.rows} 条 / {active.pages} 页 · {active.complete_selected_sources?'所选来源扫描完成':`已扫描 ${active.source_index}/${active.source_count} 个分片`}</p>}
    {error&&<p role="alert" className="error-notice">{error}</p>}
    {active&&active.pages>0&&<ContactTimeline key={`timeline-${active.id}`} accountId={accountId} id={active.id} disabled={busy} onLocate={position=>void read(active,position.page,position.row)}/>}
    {active&&active.pages>0&&<CaptureMessageLocator key={`locate-${active.id}`} accountId={accountId} id={active.id} disabled={busy} onLocate={position=>void read(active,position.page,position.row)}/>}
    {active&&active.pages>0&&<CaptureTools key={active.id} accountId={accountId} id={active.id} viewId={viewId} disabled={busy} onLocate={position=>void read(active,position.page,position.row)}/>}
    {active&&active.pages>0&&<CaptureMedia key={`media-${active.id}`} accountId={accountId} id={active.id}/>}
    {active&&active.pages>0&&<CaptureRecords key={`records-${active.id}`} accountId={accountId} id={active.id} onLocate={position=>void read(active,position.page,position.row)}/>}
    {active&&active.pages>0&&<CaptureMediaCatalog key={`catalog-${active.id}`} accountId={accountId} id={active.id} onLocate={position=>void read(active,position.page,position.row)}/>}
    {active&&active.pages>0&&<CaptureTranscripts key={`asr-${active.id}`} accountId={accountId} id={active.id}/>}
    {active&&(active.complete_selected_sources||active.imported)&&<CaptureAdd key={`add-${active.id}`} accountId={accountId} id={active.id} onChanged={()=>void added()}/>}
    <ul>{items.map(item=><li key={item.id}><span>{new Date(item.started_at).toLocaleString()} · {item.rows} 条{item.imported?" · 导入副本":""}</span> <button className="button" disabled={busy||!item.pages} onClick={()=>void read(item,0)}>浏览</button>{item.can_resume&&<button className="button" disabled={busy} onClick={()=>void run(item)}>继续补读</button>}</li>)}</ul>
    {next!==null&&<button className="button" disabled={busy} onClick={()=>void list(next)}>更多任务</button>}
    {page&&active&&<><CaptureMedia key={`media-${active.id}-${index}`} accountId={accountId} id={active.id} index={index}/><ol className="recovery-messages">{page.rows.map((row,rowIndex)=><li data-capture-row={rowIndex} tabIndex={-1} style={focusRow===rowIndex?{outline:"2px solid var(--accent)",outlineOffset:4}:undefined} key={`${row.source_index}:${row.username}:${row.message.local_id}`}><div>{row.username} · {new Date(row.message.timestamp*1000).toLocaleString()}</div><p className="message-body">{row.message.content_status==='deleted_local'?'[本地已删除，原文可从变更记录恢复]':row.message.text??`类型 ${row.message.message_type} · ${row.message.content_status}`}</p><CaptureEditor key={`${active.id}:${index}:${rowIndex}:${page.revision}`} accountId={accountId} id={active.id} index={index} row={rowIndex} revision={page.revision} message={row.message} onChanged={()=>void read(active,index)}/></li>)}</ol><div className="actions"><button className="button" disabled={busy||!index} onClick={()=>void read(active,index-1)}>上一页</button><span>第 {index+1} 页</span><button className="button" disabled={busy||page.next_page===null} onClick={()=>void read(active,page.next_page!)}>下一页</button></div></>}
  </details>;
}
