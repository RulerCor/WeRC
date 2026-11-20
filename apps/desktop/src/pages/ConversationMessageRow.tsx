import {useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {SearchHighlight} from '../components/SearchHighlight';
import {LiveSender} from '../components/LiveSender';
import {MessageCard,type ConversationCard} from '../components/MessageCard';
import {safeTime} from '../lib/browse';
import {validSendText} from '../lib/send-text';
import {ConversationMedia,type MediaHint} from './ConversationMedia';
import {QuoteComposer} from './QuoteComposer';
import type {LiveMessage} from './LiveChat';

function placeholder(type:string,status:string){
  if(status==='content_oversized')return '消息内容超过读取上限';
  if(status==='content_invalid')return '消息内容无法解码';
  if(status==='content_missing')return '来源未保存消息正文';
  try{return ({3:'图片消息',34:'语音消息',43:'视频消息',62:'视频消息',47:'表情消息',49:'文件或应用消息',10000:'系统消息',10002:'系统通知'} as Record<string,string>)[String(BigInt(type)&0xffffffffn)]??'此消息类型暂未解析';}catch{return '此消息类型暂未解析';}
}

export function messageDay(timestamp:number):string {
  const date=new Date(timestamp*1000);
  return Number.isFinite(date.getTime())?date.toLocaleDateString(undefined,{year:'numeric',month:'long',day:'numeric'}):'日期未知';
}

export function ConversationMessageRow({accountId,viewId,username,row,source,tag,card,media,dateLabel,searchText,focused,busy,onLocate}:{
  accountId:string;viewId:string;username:string;row:LiveMessage;source:string;tag:string|null;
  card?:ConversationCard;media:MediaHint[];dateLabel?:string;searchText?:string;
  focused?:boolean;busy:boolean;onLocate:()=>void;
}) {
  const operations=useRef<HTMLDetailsElement>(null),[notice,setNotice]=useState('');
  const messageKey=`${viewId}:${tag??''}:${row.local_id}`;
  async function copy(value:string,label:string){try{await navigator.clipboard.writeText(value);setNotice(`${label}已复制`);}catch{setNotice('复制失败，可以展开消息信息后选中文字复制。');}}
  const [saving,setSaving]=useState(false);
  async function favorite(){setSaving(true);try{await command('library_save',{accountId,category:'favorite',id:null,revision:0,entry:{title:`${username} · ${safeTime(row.timestamp)}`,text:row.text??`消息类型 ${row.message_type}`,tags:['聊天收藏'],source:JSON.stringify({conversation:username,server_id:row.server_id,local_id:row.local_id,source})}});setNotice('已保存到资料库 / 本地收藏。');}catch(e){setNotice(String(e));}finally{setSaving(false);}}
  function openOperations(){if(operations.current){operations.current.open=true;operations.current.querySelector('summary')?.focus();}}
  return <li className={focused?'message-located':undefined} data-located={focused||undefined} onContextMenu={event=>{const target=event.target as HTMLElement;if(target.closest('input,textarea,audio,video,a'))return;event.preventDefault();openOperations();}} onKeyDown={event=>{if(event.key==='Escape'&&operations.current?.open){operations.current.open=false;operations.current.querySelector('summary')?.focus();}if(event.key==='ContextMenu'||(event.shiftKey&&event.key==='F10')){event.preventDefault();openOperations();}}}>
    {dateLabel&&<div className="message-date" role="separator" aria-label={dateLabel}>{dateLabel}</div>}
    {focused&&<span className="state-tag">已定位的消息</span>}
    <div className="message-heading"><LiveSender username={row.sender_username}/>{source&&<span className="message-source">{source}</span>}<time>{safeTime(row.timestamp)}</time></div>
    <div className="message-bubble">{card?<MessageCard key={messageKey} card={card.content}
      renderImage={path=><ConversationMedia key={`${messageKey}:forward:${path.join('.')}`} accountId={accountId} viewId={viewId} hint={{local_id:row.local_id,source:tag,kind:'image'}} forwardPath={path}/>}
      renderFile={path=><ConversationMedia key={`${messageKey}:file:${path.join('.')}`} accountId={accountId} viewId={viewId} hint={{local_id:row.local_id,source:tag,kind:'file'}} forwardPath={path}/>}/>:<span className="message-body" dir="auto">{row.text===null?placeholder(row.message_type,row.content_status):<SearchHighlight text={row.text} needle={searchText}/>}</span>}</div>
    {media.map(h=><ConversationMedia key={messageKey} accountId={accountId} viewId={viewId} hint={h}/>)}
    {row.message_type==='1'&&row.server_id&&row.server_id!=='0'&&row.text&&validSendText(row.text)&&<QuoteComposer key={`${messageKey}:quote`} accountId={accountId} viewId={viewId} localId={row.local_id} source={tag} username={username}/>}
    <details className="message-operations" ref={operations} onToggle={()=>{if(!operations.current?.open)setNotice('');}}>
      <summary aria-label="消息操作与信息">消息操作</summary>
      <button type="button" className="button" disabled={busy} onClick={onLocate}>查看前后上下文</button>
      <button type="button" className="button" disabled={busy||saving} onClick={()=>void favorite()}>收藏到资料库</button>
      <div className="actions">{row.text!==null&&<button type="button" className="button" onClick={()=>void copy(row.text!,'消息文字')}>复制文字</button>}<button type="button" className="button" onClick={()=>void copy(row.server_id&&row.server_id!=='0'?row.server_id:row.local_id,row.server_id&&row.server_id!=='0'?'服务器消息ID':'本地消息ID')}>复制消息ID</button><HelpTip label="消息ID与来源说明">服务器ID按原值显示，尚未取得时显示“未取得”。本地ID只在当前消息分片内有效，其他分片可能有相同ID；查询和媒体操作同时绑定来源。服务器ID存在不等于对方已读。右键消息或按Shift+F10也可展开，Escape收起。</HelpTip></div>
      <dl className="message-information"><dt>类型</dt><dd>{row.message_type}</dd><dt>时间</dt><dd>{safeTime(row.timestamp)}</dd><dt>本地ID</dt><dd>{row.local_id}</dd><dt>服务器ID</dt><dd>{row.server_id&&row.server_id!=='0'?row.server_id:'未取得'}</dd><dt>来源</dt><dd>{source||'当前消息分片'}</dd><dt>正文状态</dt><dd>{({text:'普通文字',text_zstd:'压缩文字已解码',metadata:'应用卡片',metadata_zstd:'压缩卡片已解码',unsupported_type:'类型尚未适配',content_missing:'未保存正文',content_invalid:'内容无法解码',content_oversized:'超过读取上限'} as Record<string,string>)[row.content_status]??row.content_status}</dd></dl>
      {row.text!==null&&<details><summary>查看完整文字</summary><pre className="message-plain-content" dir="auto">{row.text}</pre></details>}
      {notice&&<p role="status">{notice}</p>}
    </details>
  </li>;
}
