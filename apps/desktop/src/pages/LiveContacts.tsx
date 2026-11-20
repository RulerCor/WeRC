import {Articles} from './Articles';
import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {SearchHighlight} from '../components/SearchHighlight';
import {ConversationMessages} from './ConversationMessages';
import {DirectoryExport} from '../components/DirectoryExport';
import {ContactAvatar} from '../components/ContactAvatar';
import {contactCategories,contactCategory,contactCategoryLabel} from '../lib/contact-category';

export interface ContactPage {
  rows:{anchor:{row_id:string;username:string};nickname:string|null;remark:string|null;alias:string|null;delete_flag:number|null;local_type?:number|null;contact_flag?:number|null;business_type?:number|null;avatar_url?:string|null;description?:string|null}[];
  search_query?:string|null;
  next:string|null;expires_in_seconds:number;mode:'local_contact_cache';complete_address_book:false;cross_page_snapshot:false;
}
export function LiveContacts({accountId,viewId,multi=false,embedded=false}:{accountId:string;viewId:string;multi?:boolean;embedded?:boolean}) {
  const [page,setPage]=useState<ContactPage|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const [publisher,setPublisher]=useState<string|null>(null);
  const [category,setCategory]=useState('all');
  const [input,setInput]=useState(''),[index,setIndex]=useState(0),[selected,setSelected]=useState<string|null>(null),[notice,setNotice]=useState('');
  const generation=useRef(0),request=useRef<string|null>(null),cursors=useRef<(string|null)[]>([null]);
  useEffect(()=>{clear();return()=>{generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});};},[accountId,viewId]);
  async function read(at:number,cursor:string|null,searchQuery:string|null){
    if(request.current)return;
    const query=searchQuery?.trim()||null;
    if(query&&(new TextEncoder().encode(query).length>256||query.includes('\0'))){setError('搜索文字最多256个UTF-8字节，不能包含空字符。');return;}
    const expected=++generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');setNotice('');setSelected(null);setPublisher(null);
    try{
      const result=await command<ContactPage>('native_live_contacts',{accountId,viewId,cursor,searchQuery:query,requestId});
      if(expected!==generation.current)return;
      cursors.current=cursors.current.slice(0,at+1);cursors.current[at]=cursor;setPage(result);setIndex(at);setInput(result.search_query??'');
    }catch(e){if(expected===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(expected===generation.current){request.current=null;setBusy(false);}}
  }
  function cancel(){generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});setBusy(false);setError('');}
  function clear(){cancel();setPage(null);setPublisher(null);setCategory('all');setInput('');setIndex(0);setSelected(null);setNotice('');cursors.current=[null];}
  async function copy(username:string){const expected=generation.current;try{await navigator.clipboard.writeText(username);if(expected===generation.current)setNotice('账号已复制');}catch{if(expected===generation.current)setNotice('复制失败，可展开资料后选中账号复制。');}}
  const query=page?.search_query??'';
  const categories=[['all','全部'],...contactCategories] as const;
  const visible=page?.rows.filter(r=>category==='all'||contactCategory(r)===category)??[];
  return <details open={embedded||undefined} className="live-contacts" onToggle={e=>{if(!e.currentTarget.open)clear();}}>
    <summary hidden={embedded}>联系人资料</summary>
    <form className="contact-search" onSubmit={e=>{e.preventDefault();void read(0,null,input);}}>
      <label className="field">查找联系人<input type="search" aria-label="查找联系人" value={input} onChange={e=>setInput(e.target.value)} maxLength={256} placeholder="昵称、备注、微信号或账号" disabled={busy}/></label>
      <div className="actions"><button type="submit" className="button" disabled={busy}>{busy?'正在读取…':input.trim()?'查找联系人':page?'重新读取联系人':'查看本地联系人'}</button>
        {query&&<button type="button" className="button" disabled={busy} onClick={()=>void read(0,null,null)}>清除搜索</button>}
        {busy&&<button type="button" className="button" onClick={cancel}>取消读取</button>}
        <HelpTip label="本地联系人说明">在当前来源的本地联系人缓存中查找昵称、备注、微信号和账号，不限于本页200条。翻页保持本次搜索条件；修改文字后点击查找会从头读取。缓存可能包含群聊、服务账号或旧资料，不能代表当前好友关系或完整通讯录。翻页会重新读取，跨页不是同一时刻快照；来源窗口过期后请重新读取消息。关闭此面板会清除显示的资料。查看消息仅打开已准备来源中的记录，不会发送或修改微信。</HelpTip>
      </div>
    </form>
    {error&&<p className="error-notice" role="alert">{error}</p>}
    {notice&&<p role="status">{notice}</p>}
    {!selected&&!publisher&&<DirectoryExport key={`${accountId}:${viewId}:${query}`} accountId={accountId} viewId={viewId} searchQuery={query} disabled={busy}/>}
    {selected&&<ConversationMessages key={selected} accountId={accountId} parentViewId={viewId} username={selected} multi={multi} displayName={page?.rows.find(row=>row.anchor.username===selected)?.remark||page?.rows.find(row=>row.anchor.username===selected)?.nickname||undefined} onClose={()=>setSelected(null)}/>}
    {publisher&&<section><button className="button" onClick={()=>setPublisher(null)}>返回联系人</button><Articles key={publisher} accountId={accountId} initialPublisher={publisher}/></section>}
    {page&&!selected&&!publisher&&<>
      {query&&<p className="field" role="status">搜索结果：<span dir="auto">{query}</span></p>}
      <div className="actions"><label>本页分类<select value={category} onChange={e=>setCategory(e.target.value)}>{categories.map(([key,label])=><option key={key} value={key}>{label} · {page.rows.filter(r=>key==='all'||contactCategory(r)===key).length}</option>)}</select></label><HelpTip label="联系人分类依据">按本地缓存的账号、类型、关系标记与公众号类型分组；好友和黑名单是本地标记，历史/非好友仅是残留资料线索，不等于服务器实时关系。订阅号与服务号仅在公众号类型明确时区分，缺失或冲突保持其他公众号。计数覆盖本页；导出可读取所有续页并按分类筛选。</HelpTip></div>
      <ul className="history-messages" aria-label="本地联系人列表">{visible.map(row=><li key={row.anchor.row_id}>
        <ContactAvatar key={`${viewId}:${row.anchor.username}`} accountId={accountId} viewId={viewId} username={row.anchor.username} name={row.remark||row.nickname||row.anchor.username} available={Boolean(row.avatar_url)}/>
        <div className="session-row-heading"><strong dir="auto"><SearchHighlight text={row.remark||row.nickname||row.anchor.username} needle={query}/></strong><button className="button" disabled={busy} onClick={()=>setSelected(row.anchor.username)}>查看消息</button>{row.anchor.username.startsWith('gh_')&&<button className="button" disabled={busy} onClick={()=>setPublisher(row.anchor.username)}>浏览文章</button>}</div>
        <code dir="auto"><SearchHighlight text={row.anchor.username} needle={query}/></code>
        <details className="contact-details"><summary>查看资料</summary><dl>
          <dt>昵称</dt><dd dir="auto">{row.nickname||'未提供'}</dd><dt>备注</dt><dd dir="auto">{row.remark||'未设置'}</dd><dt>微信号</dt><dd dir="auto">{row.alias||'未提供'}</dd><dt>账号标识</dt><dd dir="auto">{row.anchor.username}</dd>
          <dt>本地分类</dt><dd>{contactCategoryLabel(row)}</dd><dt>分类原始值</dt><dd>类型 {row.local_type??'未知'} · 关系 {row.contact_flag??'未知'} · 公众号 {row.business_type??'未知'}</dd>
          {row.description&&<><dt>简介</dt><dd dir="auto">{row.description}</dd></>}
        </dl><button className="button" onClick={()=>void copy(row.anchor.username)}>复制账号</button></details>
        {row.delete_flag!==null&&row.delete_flag!==0&&<span className="field">本地记录标记为已删除</span>}
      </li>)}</ul>
      {!visible.length&&<p className="empty-inline">{page.rows.length?'本页没有此分类的联系人，可更换分类或翻页。':query?'当前来源未找到匹配的联系人，可以更换关键词或清除搜索。':'此来源暂无联系人资料。'}</p>}
      <div className="actions"><button className="button" disabled={busy||index===0} onClick={()=>void read(index-1,cursors.current[index-1],query)}>上一页联系人</button><span className="field" role="status">第{index+1}页 · 本页{page.rows.length}条</span><button className="button" disabled={busy||!page.next} onClick={()=>void read(index+1,page.next,query)}>下一页联系人</button></div>
    </>}
  </details>;
}
