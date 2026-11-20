import {useState} from 'react';
import {HelpTip} from '../components/HelpTip';
import {makeConversationQuery,messageTypeChoices,queryActive,querySummary,queryFields,type ConversationQuery} from '../lib/conversation-query';

export function ConversationSearch({busy,available,query,onFind,onReset,onCancel,senders=[],defaultOpen=false}:{
  busy:boolean;available:boolean;query?:ConversationQuery|null;
  onFind:(query:ConversationQuery)=>void;onReset:()=>void;onCancel:()=>void;
  senders?:string[];defaultOpen?:boolean;
}) {
  const initial=queryFields(query);
  const [from,setFrom]=useState(initial.from),[through,setThrough]=useState(initial.through),[text,setText]=useState(initial.text),[types,setTypes]=useState<string>(initial.types),[serverId,setServerId]=useState(initial.serverId),[error,setError]=useState('');
  const [sender,setSender]=useState(initial.sender);
  return <details className="conversation-search" open={defaultOpen||undefined}>
    <summary>查找消息{queryActive(query)&&<span className="state-tag">筛选中</span>}</summary>
    <form onSubmit={event=>{event.preventDefault();try{const value=makeConversationQuery(from,through,types,text,serverId,sender);setError('');onFind(value);}catch(e){setError(e instanceof Error?e.message:String(e));}}}>
      <div className="conversation-search-title"><strong>搜索与日期定位</strong><HelpTip label="搜索和日期范围说明">在当前选定的消息来源中查找，不局限于屏幕上的60条。日期按本电脑时区，包含结束日期当天；只填结束日期可从该日向前查看，开始和结束填同一天可只看当天。文字搜索匹配普通文字正文，包含可解码的压缩文字，不搜索图片内容、语音转写或卡片内部字段。文件、链接、音乐、小程序等按来源卡片类型区分；交易卡片类型不等于交易状态。类型与日期、文字可组合；文字条件搭配非文字类型不会有结果。读取有时间和容量限制，超时请缩小日期范围；结果仍可继续翻页和读取更早窗口。跨分片不是同一时刻的快照，不改变微信已读状态。</HelpTip></div>
      <label>文字内容<input type="search" value={text} maxLength={256} onChange={e=>setText(e.target.value)} placeholder="输入要查找的文字" disabled={busy}/></label>
      <details className="conversation-search-filters"><summary>日期、类型与发送者</summary>
      <div className="conversation-search-dates"><label>开始日期<input aria-label="查找开始日期" type="date" min="1970-01-02" max="9998-12-31" value={from} onChange={e=>setFrom(e.target.value)} disabled={busy}/></label><label>结束日期<input aria-label="查找结束日期" type="date" min="1970-01-01" max="9998-12-31" value={through} onChange={e=>setThrough(e.target.value)} disabled={busy}/></label></div>
      <label>消息类型<select value={types} onChange={e=>setTypes(e.target.value)} disabled={busy}>{messageTypeChoices.map(([value,label])=><option key={value} value={value}>{label}</option>)}</select></label>
      <label>发送者账号<input list="conversation-search-senders" value={sender} maxLength={256} onChange={e=>setSender(e.target.value)} disabled={busy} placeholder="可选：账号标识，非昵称"/><datalist id="conversation-search-senders">{senders.map(value=><option key={value} value={value}/>)}</datalist></label>
      <details className="conversation-search-id"><summary>按消息ID定位</summary><label>服务器消息ID<input type="text" inputMode="numeric" value={serverId} maxLength={20} disabled={busy} onChange={e=>setServerId(e.target.value)} placeholder="从消息信息复制的服务器ID"/></label><HelpTip label="按消息ID查找说明">按服务器ID精确匹配当前搜索范围，保留完整数字，避免长ID的精度损失。它与日期、类型、文字条件共同生效；不清楚其他条件时先清除条件。本地ID仅在单一分片内有效，不能在此混用。</HelpTip></details>
      </details>
      <div className="actions"><button className="button primary" disabled={busy||!available} type="submit">{busy?'正在查找…':'查找'}</button>{busy&&<button type="button" className="button" onClick={onCancel}>取消读取</button>}<button type="button" className="button" disabled={busy||!available} onClick={()=>{setFrom('');setThrough('');setText('');setTypes('');setServerId('');setSender('');setError('');onReset();}}>清除条件</button></div>
      {error&&<p role="alert" className="error-notice">{error}</p>}
      {queryActive(query)&&<p className="conversation-query-summary" role="status">{querySummary(query!)}<span> · 当前显示匹配结果</span></p>}
    </form>
  </details>;
}
