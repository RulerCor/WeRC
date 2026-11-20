import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {calendarMonth,currentMonth,shiftMonth,type CalendarDay} from '../lib/conversation-calendar';
import type {ConversationQuery} from '../lib/conversation-query';
interface CalendarPage {days:CalendarDay[];counts:number[];source_count:number;source_snapshot_atomic:boolean;expires_in_seconds:number}
export function ConversationCalendar({accountId,parentViewId,username,busy,available,onFind,onBusyChange}:{accountId:string;parentViewId:string;username:string;busy:boolean;available:boolean;onFind:(query:ConversationQuery)=>void;onBusyChange:(busy:boolean)=>void}) {
  const [month,setMonth]=useState(currentMonth),[page,setPage]=useState<CalendarPage|null>(null),[loading,setLoading]=useState(false),[error,setError]=useState('');
  const request=useRef<string|null>(null),generation=useRef(0),canceled=useRef(false);
  useEffect(()=>()=>{generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});onBusyChange(false);},[accountId,parentViewId,username]);
  useEffect(()=>{if(!page)return;const id=window.setTimeout(()=>{setPage(null);setError('日历统计已过期，请重新加载。');},page.expires_in_seconds*1000);return()=>window.clearTimeout(id);},[page]);
  async function read(){
    if(busy||request.current)return;
    let days:CalendarDay[];try{days=calendarMonth(month).days;}catch(e){setError(String(e instanceof Error?e.message:e));return;}
    const id=crypto.randomUUID(),expected=++generation.current;request.current=id;canceled.current=false;setLoading(true);onBusyChange(true);setPage(null);setError('');
    try{const result=await command<CalendarPage>('native_live_calendar',{accountId,viewId:parentViewId,selection:{username,days},requestId:id});if(expected===generation.current&&!canceled.current)setPage(result);}
    catch(e){if(expected===generation.current&&!canceled.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(expected===generation.current){request.current=null;setLoading(false);onBusyChange(false);}}
  }
  async function cancel(){const id=request.current;if(!id)return;canceled.current=true;setError('已请求取消日历读取。');try{await command('cancel_browse',{requestId:id});}catch(e){if(request.current===id)setError(e instanceof Error?e.message:String(e));}}
  function selectMonth(value:string){setMonth(value);setPage(null);setError('');}
  const disabled=busy||loading;
  const offset=page?(new Date(page.days[0].from_timestamp*1000).getDay()+6)%7:0;
  return <details className="conversation-calendar"><summary>消息日历</summary>
    <div className="actions"><label>月份<input aria-label="消息日历月份" type="month" min="1970-02" max="9998-12" value={month} disabled={disabled} onChange={e=>selectMonth(e.target.value)}/></label><HelpTip label="消息日历说明">按本电脑时区统计所选来源内此会话每天的全部消息，包含当天起点，不包含次日起点。点击日期会清除其他筛选条件并查看当天记录，可继续翻页。空白日期显示0。多分片分别读取后相加，不是同一时刻的快照，也不合并重复记录；未选择来源和未下载历史不计入。更改月份后点击加载，统计约两分钟内有效。</HelpTip></div>
    <div className="actions"><button className="button" disabled={disabled||!month||month<='1970-02'} onClick={()=>selectMonth(shiftMonth(month,-1))}>上月</button><button className="button" disabled={disabled||!month||month>='9998-12'} onClick={()=>selectMonth(shiftMonth(month,1))}>下月</button><button className="button" disabled={disabled||!available||!month} onClick={()=>void read()}>{loading?'正在统计…':'加载日历'}</button>{loading&&<button className="button" onClick={()=>void cancel()}>取消统计</button>}</div>
    {error&&<p role="alert" className="error-notice">{error}</p>}
    {page&&<><p role="status">{month} · {page.counts.reduce((a,b)=>a+BigInt(b),0n).toLocaleString()}条 · {page.source_count}个来源</p><div className="conversation-calendar-grid" role="group" aria-label={`${month}消息日历`}>
      {['一','二','三','四','五','六','日'].map(day=><span className="calendar-weekday" key={day} aria-label={`星期${day}`}>{day}</span>)}
      {Array.from({length:offset},(_,i)=><span key={`empty-${i}`} aria-hidden="true"/>)}
      {page.days.map((day,i)=><button type="button" className={`calendar-day${page.counts[i]?' has-messages':''}`} key={day.from_timestamp} disabled={disabled||!available} aria-label={`${month}-${String(i+1).padStart(2,'0')}，${page.counts[i]}条消息`} onClick={()=>onFind(day)}><span>{i+1}</span><small>{page.counts[i].toLocaleString()}</small></button>)}
    </div></>}
  </details>;
}
