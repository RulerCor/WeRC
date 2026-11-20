import {useEffect,useRef,useState} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
interface Entry{url:string;title:string;published_at:number|null}
interface Page{publisher:string;data:Entry[];next_cursor:string|null;end_reached:boolean;skipped:number}
export function ArticleHistory({accountId,onChoose,initialPublisher}:{accountId:string;onChoose:(url:string)=>void;initialPublisher?:string}){
 const [publisher,setPublisher]=useState(initialPublisher??''),[rows,setRows]=useState<Entry[]>([]),[next,setNext]=useState<string|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[end,setEnd]=useState(false),[skipped,setSkipped]=useState(0);
 const generation=useRef(0),request=useRef<string|null>(null);
 useEffect(()=>{generation.current++;setRows([]);setNext(null);setEnd(false);setError('');setBusy(false);return()=>{generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});};},[accountId]);
 async function read(cursor:string|null){if(busy)return;const gen=generation.current,id=crypto.randomUUID();request.current=id;setBusy(true);setError('');try{
   const p=await command<Page>('article_action',{accountId,requestId:id,operation:{action:'history',publisher:publisher.trim(),cursor}});
   if(gen!==generation.current)return;
   setRows(old=>cursor?[...old,...p.data.filter(a=>!old.some(b=>a.url===b.url))]:p.data);setNext(p.next_cursor);setEnd(p.end_reached);setSkipped(n=>(cursor?n:0)+p.skipped);
 }catch(e){if(gen===generation.current)setError(String(e));}finally{if(request.current===id)request.current=null;if(gen===generation.current)setBusy(false);}}
 return <details open={!!initialPublisher||undefined}><summary>从微信读取公众号历史</summary><div className="section-title"><h3>公众号历史</h3><HelpTip label="公众号历史读取说明">先连接当前微信原生通道。公众号标识以 gh_ 开头，可在联系人资料中复制。逐页读取微信返回的文章目录，点击标题后再读取正文；不会发送消息。页码由当前登录会话绑定，断开后请重新读取。</HelpTip></div>
 <form onSubmit={e=>{e.preventDefault();void read(null);}}><label>公众号标识<input value={publisher} maxLength={127} placeholder="gh_…" disabled={busy} onChange={e=>{setPublisher(e.target.value);setRows([]);setNext(null);setEnd(false);setSkipped(0);}}/></label><button className="button" disabled={busy||!desktopAvailable||!/^gh_[\w.-]+$/.test(publisher.trim())}>读取历史目录</button>{busy&&<button type="button" className="button" onClick={()=>{if(request.current)void command('cancel_browse',{requestId:request.current}).catch(e=>setError(String(e)));}}>取消读取</button>}</form>
 <ul>{rows.map(a=><li key={a.url}><button className="button" disabled={busy} onClick={()=>onChoose(a.url)}>{a.title}</button>{a.published_at&&<time> · {new Date(a.published_at*1000).toLocaleDateString()}</time>}</li>)}</ul>
 {next&&<button className="button" disabled={busy} onClick={()=>void read(next)}>更早的文章</button>}{end&&<p role="status">已读到微信本次返回的末页，共显示 {rows.length} 篇文章。</p>}{skipped>0&&<p>{skipped} 项不是可读取的文章链接。</p>}{error&&<p className="error-notice" role="alert">{error}</p>}</details>;
}
