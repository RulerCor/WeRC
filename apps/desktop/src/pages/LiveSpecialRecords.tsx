import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {safeTime} from '../lib/browse';
const sources={friend_request:'好友申请',search_chat:'聊天搜索',search_web:'网页搜索',search_brand:'公众号搜索',revoke:'撤回记录',transfer:'转账记录',red_packet:'红包记录'};
type Source=keyof typeof sources;
type Query={source:Source;text:string;from:number|null;until:number|null};
type Item={anchor:{row_id:string;raw_time:number};timestamp:number|null;username:string|null;text:string;direction:string;fields:Record<string,string>};
type Page={rows:Item[];next:string|null;query:Query;scanned:number;expires_in_seconds:number};
export function LiveSpecialRecords({accountId,viewId}:{accountId:string;viewId:string}){
  const [source,setSource]=useState<Source>('friend_request'),[text,setText]=useState(''),[from,setFrom]=useState(''),[until,setUntil]=useState('');
  const [page,setPage]=useState<Page|null>(null),[index,setIndex]=useState(0),[busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const [format,setFormat]=useState('xlsx'),[savedPath,setSavedPath]=useState('');
  const request=useRef<string|null>(null),generation=useRef(0),cursors=useRef<(string|null)[]>([null]),frozen=useRef<Query|null>(null),expires=useRef(0);
  useEffect(()=>{clear();setSavedPath('');cursors.current=[null];frozen.current=null;expires.current=0;return()=>{generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});};},[accountId,viewId]);
  useEffect(()=>{if(!page)return;const timer=setInterval(()=>{if(Date.now()>=expires.current){clear();setError('来源窗口已过期，请重新准备消息来源。');}},1000);return()=>clearInterval(timer);},[page]);
  function clear(){generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});request.current=null;setPage(null);setBusy(false);setIndex(0);setNotice('');setError('');setSavedPath('');}
  function change(action:()=>void){clear();action();}
  function query():Query{return {source,text:text.trim(),from:from?Math.floor(new Date(from).getTime()/1000):null,until:until?Math.floor(new Date(until).getTime()/1000):null};}
  async function read(at=0,cursor:string|null=null){
    if(request.current)return;const g=++generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');
    if(at===0)frozen.current=query();
    try{const result=await command<Page>('native_special_records',{accountId,viewId,query:cursor?null:frozen.current,cursor,requestId});if(g!==generation.current)return;
      cursors.current=cursors.current.slice(0,at+1);cursors.current[at]=cursor;expires.current=Date.now()+result.expires_in_seconds*1000;setPage(result);setIndex(at);
    }catch(e){if(g===generation.current){setPage(null);setError(String(e));}}finally{if(g===generation.current){request.current=null;setBusy(false);}}
  }
  async function save(){
    if(request.current)return;const g=++generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');setNotice('');
    try{const path=await command<string|null>('choose_directory_export_path',{format,suggestedName:`WeRC-${sources[source]}-${new Date().toISOString().replace(/[:.]/g,'-')}.${format}`});if(!path||g!==generation.current)return;
      const result=await command<{rows:number;bytes:number}>('save_special_records',{accountId,viewId,query:query(),format,path,requestId});if(g===generation.current){setSavedPath(path);setNotice(`已保存全部匹配记录，共${result.rows}条、${result.bytes.toLocaleString()}字节。`);}
    }catch(e){if(g===generation.current)setError(String(e));}finally{if(g===generation.current){request.current=null;setBusy(false);}}
  }
  return <section aria-label="微信专项记录">
    <div className="actions"><h3>微信专项记录</h3><HelpTip label="专项记录使用说明">选择记录类型后点击查看；展开单条可看详情。读取当前微信同步到此电脑的专项数据库，与聊天卡片记录分开。不会接受好友、撤回消息或操作资金。原始状态码只按来源展示，不推断好友关系或收付款结果；未记录时间的红包不参与日期筛选。搜索只匹配可显示正文和账号。跨页不是同一时刻快照，本机未同步的记录不会出现。</HelpTip></div>
    <form className="history-query" onSubmit={e=>{e.preventDefault();void read();}}>
      <label>记录类型<select disabled={busy} value={source} onChange={e=>change(()=>{setSource(e.target.value as Source);setFrom('');setUntil('');})}>{Object.entries(sources).map(([id,label])=><option key={id} value={id}>{label}</option>)}</select></label>
      <label>搜索记录<input disabled={busy} value={text} maxLength={128} placeholder="正文或账号" onChange={e=>change(()=>setText(e.target.value))}/></label>
      <button className="button primary" disabled={busy}>查看记录</button>
    </form>
    {source!=='red_packet'&&<details><summary>日期筛选</summary><div className="history-query"><label>起始时间<input type="datetime-local" disabled={busy} value={from} onChange={e=>change(()=>setFrom(e.target.value))}/></label><label>截止时间（不含）<input type="datetime-local" disabled={busy} value={until} onChange={e=>change(()=>setUntil(e.target.value))}/></label></div></details>}
    {page&&<><ul className="recovery-messages">{page.rows.map(item=><li key={item.anchor.row_id}><details><summary><strong dir="auto">{item.text.slice(0,80)||item.username||sources[source]}</strong><span className="field"> · {item.timestamp?safeTime(item.timestamp):'未记录时间'}</span></summary><p className="message-body" dir="auto">{item.text||'无可显示正文'}</p><dl><dt>账号或会话</dt><dd className="message-body">{item.username||'未记录'}</dd><dt>方向</dt><dd>{{incoming:'收到',outgoing:'发出',unknown:'未记录'}[item.direction]||'未记录'}</dd>{Object.entries(item.fields).map(([k,v])=><div key={k}><dt>{k}</dt><dd className="message-body">{v}</dd></div>)}<dt>本地行编号</dt><dd>{item.anchor.row_id}</dd></dl></details></li>)}</ul>
      {!page.rows.length&&<p role="status">{page.next?'本批没有匹配项，可继续扫描。':'没有匹配的本地记录。'}</p>}
      <div className="actions"><button className="button" disabled={busy||index===0} onClick={()=>void read(index-1,cursors.current[index-1])}>上一页记录</button><span role="status">第{index+1}页 · 扫描{page.scanned}项</span><button className="button" disabled={busy||!page.next} onClick={()=>void read(index+1,page.next)}>下一页记录</button></div></>}
    <details><summary>导出专项记录</summary><div className="actions"><label>格式<select value={format} disabled={busy} onChange={e=>setFormat(e.target.value)}><option value="xlsx">Excel / XLSX</option><option value="csv">CSV</option><option value="json">JSON</option><option value="txt">TXT</option></select></label><button className="button" disabled={busy} onClick={()=>void save()}>选择位置并导出</button><HelpTip label="专项记录导出说明">导出当前筛选的全部续页，不限于显示页。最多5万条、32MiB；读取失败或取消不会发布部分文件。Excel用文本保留完整编号，CSV防止正文成为公式；JSON保留原始字段。不会包含好友验证票据、付款链接或原始载荷。</HelpTip></div></details>
    {busy&&<button className="button" onClick={clear}>取消当前操作</button>}{notice&&<p role="status">{notice}</p>}{savedPath&&<button className="button" onClick={()=>void command('show_directory_export',{path:savedPath}).catch(e=>setError(String(e)))}>查看保存位置</button>}{error&&<p role="alert" className="error-notice">{error}</p>}
  </section>;
}
