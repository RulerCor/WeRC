import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {relatedMomentInteractions,type MomentRow,type RelatedInteraction} from '../lib/moment-people';
import {safeTime} from '../lib/browse';
import {HelpTip} from '../components/HelpTip';
import {MomentActions} from './MomentActions';
import type {MomentQuery} from './MomentTools';

interface Result {post:MomentRow;interaction:RelatedInteraction}
export function MomentInteractions({accountId,viewId,query,disabled,onBusy}:{accountId:string;viewId:string;query:MomentQuery;disabled:boolean;onBusy:(busy:boolean)=>void}){
  const [rows,setRows]=useState<Result[]>([]),[busy,setBusy]=useState(false),[error,setError]=useState(''),[scanned,setScanned]=useState(0),[started,setStarted]=useState(false),[next,setNext]=useState<string|null>(null);
  const request=useRef<string|null>(null),generation=useRef(0),self=useRef('');
  function reset(){generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});request.current=null;self.current='';setRows([]);setNext(null);setScanned(0);setStarted(false);setError('');setBusy(false);onBusy(false);}
  useEffect(()=>{reset();return()=>{generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});request.current=null;onBusy(false);};},[accountId,viewId,query]);
  async function read(more=false){
    if(request.current||disabled)return;
    const g=++generation.current,id=crypto.randomUUID();request.current=id;setBusy(true);onBusy(true);setError('');
    if(!more){setRows([]);setScanned(0);setNext(null);setStarted(false);}
    try{
      if(!more)self.current=(await command<{username:string}>('native_current_profile',{accountId,viewId,requestId:id})).username;
      if(g!==generation.current)return;
      const page=await command<{rows:MomentRow[];next:string|null}>('native_live_moments',{accountId,viewId,query:more?null:query,cursor:more?next:null,requestId:id});
      if(g!==generation.current)return;
      const incoming=page.rows.flatMap(post=>relatedMomentInteractions(post,self.current).map(interaction=>({post,interaction})));
      setRows(old=>[...(more?old:[]),...incoming]);setScanned(n=>(more?n:0)+page.rows.length);setNext(page.next);setStarted(true);
    }catch(e){if(g===generation.current)setError(String(e));}
    finally{if(g===generation.current){request.current=null;setBusy(false);onBusy(false);}}
  }
  return <details className="local-tools" onToggle={e=>{if(!e.currentTarget.open)reset();}}><summary>与我有关的互动</summary>
    <div className="actions"><button className="button" disabled={busy||disabled} onClick={()=>void read()}>查看与我有关的互动</button><HelpTip label="与我有关的互动说明">使用当前微信的准确账号，查找他人对我动态的点赞、评论，以及明确回复我的评论。不会根据正文中的昵称或“@”猜测身份。按上方作者和日期范围逐页扫描本机已缓存动态，可继续查看；没有缓存到的互动无法显示，正文提及和服务端完整通知不在此列表中。</HelpTip>{busy&&<button className="button" onClick={reset}>取消互动读取</button>}</div>
    {started&&<p className="field" role="status">已检查 {scanned} 条缓存动态 · {rows.length} 项相关互动{next?' · 仍可继续扫描':''}</p>}
    <ul className="history-messages">{rows.map(({post,interaction:r},i)=><li key={`${post.anchor.id}:${r.kind}:${r.value.id64??r.value.id??i}:${i}`}><strong dir="auto">{r.value.nickname||r.value.username}</strong> · {r.kind==='like'?'赞了我的动态':r.reason==='reply_to_me'?'回复了我':'评论了我的动态'}{r.value.timestamp!==null&&<time> · {safeTime(r.value.timestamp)}</time>}{r.value.text&&<p className="message-body" dir="auto">{r.value.text}</p>}<details><summary>原动态与准确身份</summary><p className="message-body" dir="auto">{post.content?.text||'无文字正文'}</p><p className="message-body">动态编号：{post.anchor.id} · 互动账号：{r.value.username}</p></details>{r.kind==='comment'&&r.value.username&&(r.value.id64||r.value.id)&&<MomentActions accountId={accountId} snsId={post.anchor.id} reply={{username:r.value.username,...(r.value.id&&r.value.id!=='0'?{id:r.value.id}:{}),...(r.value.id64&&r.value.id64!=='0'?{id64:r.value.id64}:{})}}/>}</li>)}</ul>
    {started&&!rows.length&&<p>目前扫描的缓存中没有匹配互动。{next?'可继续扫描下一页。':''}</p>}{next&&<button className="button" disabled={busy||disabled} onClick={()=>void read(true)}>继续扫描互动</button>}{error&&<p role="alert">{error}</p>}
  </details>;
}
