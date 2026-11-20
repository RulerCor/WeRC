import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
type Filter={message_kinds:string[];keyword:string;view:string;exclude_conversations:string[];conversations:string[];from:number|null;until:number|null;deduplicate:boolean};
type Contact={username:string;messages:number;sent:number;received:number;posts:number;likes_given:number;likes_received:number;comments_given:number;comments_received:number};
type Report={generated_at:string;contacts:Contact[];own_posts:number;observed_posts:number;unreadable_posts:number;unmatched_login_posts:number;interactions_without_time:number;complete_selected_sources:boolean};
export function CaptureSocial({accountId,id,filter,disabled}:{accountId:string;id:string;filter:Filter;disabled:boolean}) {
  const [report,setReport]=useState<Report|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState(''),[search,setSearch]=useState(''),[sort,setSort]=useState('messages');
  const generation=useRef(0),request=useRef<string|null>(null),signature=JSON.stringify(filter);
  useEffect(()=>{setReport(null);setBusy(false);setError('');setNotice('');setSearch('');return()=>{generation.current++;const pending=request.current;request.current=null;if(pending)void command('cancel_browse',{requestId:pending}).catch(()=>{});};},[accountId,id,signature]);
  async function act(format?:string){
    if(request.current||disabled)return;
    const g=generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');setNotice('');
    try {
      if(!format){const value=await command<Report>('capture_social_report',{accountId,id,filter,requestId});if(g===generation.current)setReport(value);}
      else {const path=`WeRC-聊天朋友圈关联-${Date.now()}.${format}`;await command('capture_social_report_export',{accountId,id,filter,requestId,format,path});if(g===generation.current)setNotice(`已保存到系统下载目录：${path}`);}
    }catch(e){if(g===generation.current)setError(String(e));}
    finally{if(g===generation.current){request.current=null;setBusy(false);}}
  }
  const rows=report?.contacts.filter(r=>r.username.toLocaleLowerCase().includes(search.toLocaleLowerCase())).sort((a,b)=>sort==='messages'?b.messages-a.messages:sort==='likes'?(b.likes_given+b.likes_received)-(a.likes_given+a.likes_received):(b.comments_given+b.comments_received)-(a.comments_given+a.comments_received));
  return <details className="local-tools"><summary>聊天与朋友圈互动关联</summary><div className="actions"><button className="button" disabled={busy||disabled} onClick={()=>void act()}>生成关联报告</button><HelpTip label="朋友圈关联说明">使用当前账号加密保存的朋友圈副本，与上方报告范围内的聊天按精确账号关联。请先在朋友圈中保留需要的动态。统计当前保存快照中的赞和评论，不代表完整历史或实时关系；互相点赞以“我点赞 / 收到赞”分别展示。指定年份时，互动须有该年时间，缺少时间的赞和评论不会挪用动态发布时间。关键词和消息类型仅筛选聊天。</HelpTip></div>
    {report&&<><p role="status">本人动态 {report.own_posts} 条 · 所选作者动态 {report.observed_posts} 条 · {report.contacts.length} 位联系人</p>
      {(!report.complete_selected_sources||report.interactions_without_time>0||report.unmatched_login_posts>0||report.unreadable_posts>0)&&<details><summary>资料覆盖情况</summary><p>{!report.complete_selected_sources?'聊天补读尚未完成。 ':''}缺少时间的互动 {report.interactions_without_time} 项（指定日期时不计入）；登录身份不匹配的副本 {report.unmatched_login_posts} 条；无法读取的动态 {report.unreadable_posts} 条。</p></details>}
      <div className="history-query"><label>查找联系人<input value={search} disabled={busy} placeholder="输入账号标识" onChange={e=>setSearch(e.target.value)}/></label><label>排序<select value={sort} onChange={e=>setSort(e.target.value)}><option value="messages">聊天最多</option><option value="likes">点赞最多</option><option value="comments">评论最多</option></select></label></div>
      <ul className="recovery-messages">{rows?.map(r=><li key={r.username}><strong className="message-body">{r.username}</strong><p>聊天 {r.messages} 条 · 动态 {r.posts} 条</p><details><summary>收发与互动明细</summary><dl><div><dt>聊天</dt><dd>我 {r.sent} / 对方 {r.received}</dd></div><div><dt>点赞</dt><dd>我点赞 {r.likes_given} / 收到赞 {r.likes_received}</dd></div><div><dt>评论</dt><dd>我评论 {r.comments_given} / 收到评论 {r.comments_received}</dd></div></dl></details></li>)}</ul>
      {!rows?.length&&<p>当前资料中没有匹配联系人；可以先保留朋友圈副本或调整报告范围。</p>}
      <details><summary>导出关联报告</summary><div className="actions">{['html','txt','json'].map(format=><button className="button" key={format} disabled={busy||disabled} onClick={()=>void act(format)}>保存 {format.toUpperCase()} 关联报告</button>)}</div><p className="field">导出文件不加密，包含联系人统计。</p></details>
    </>}
    {busy&&<button className="button" onClick={()=>{if(request.current)void command('cancel_browse',{requestId:request.current}).catch(e=>setError(String(e)));}}>取消关联分析</button>}
    {error&&<p className="error-notice" role="alert">{error}</p>}{notice&&<p role="status">{notice}</p>}
  </details>;
}
