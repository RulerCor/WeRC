import {MentionComposer} from './MentionComposer';
import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {DirectoryExport} from '../components/DirectoryExport';
interface GroupPage {
  username:string;owner_username:string|null;cached_pins?:{server_id:string}[]|null;
  rows:{anchor:{row_id:string;member_id:string|null;username:string|null};nickname:string|null;remark:string|null;group_nickname?:string|null}[];
  next:string|null;expires_in_seconds:number;mode:'local_group_cache';complete_server_membership:false;cross_page_snapshot:false;
}
export function LiveGroups({accountId,viewId,username,recipientViewId,multi=false,onFindPin,pinBusy=false}:{accountId:string;viewId:string;username:string;recipientViewId?:string;multi?:boolean;onFindPin?:(serverId:string)=>void;pinBusy?:boolean}) {
  const [page,setPage]=useState<GroupPage|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[filter,setFilter]=useState(''),[index,setIndex]=useState(0);
  const generation=useRef(0),request=useRef<string|null>(null),cursors=useRef<(string|null)[]>([null]);
  function clear(){generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});setPage(null);setBusy(false);setError('');setFilter('');setIndex(0);cursors.current=[null];}
  useEffect(()=>{clear();return()=>{generation.current++;if(request.current)void command('cancel_browse',{requestId:request.current}).catch(()=>{});};},[accountId,viewId,username]);
  useEffect(()=>{if(!page)return;const timer=window.setTimeout(()=>{clear();setError('群成员窗口已过期，请重新准备消息来源。');},page.expires_in_seconds*1000);return()=>window.clearTimeout(timer);},[page]);
  async function read(at:number,cursor:string|null){
    if(request.current)return;
    const expected=++generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');
    try{const result=await command<GroupPage>('native_live_group_members',{accountId,viewId,username,cursor,requestId});if(expected!==generation.current)return;
      cursors.current=cursors.current.slice(0,at+1);cursors.current[at]=cursor;setPage(result);setIndex(at);setFilter('');
    }catch(e){if(expected===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(expected===generation.current){request.current=null;setBusy(false);}}
  }
  const query=filter.trim().toLocaleLowerCase();
  const visible=page?.rows.filter(row=>!query||[row.anchor.username,row.nickname,row.remark,row.group_nickname].some(s=>s?.toLocaleLowerCase().includes(query)))??[];
  return <details className="live-groups" onToggle={e=>{if(e.target===e.currentTarget&&!e.currentTarget.open)clear();}}><summary>群资料与置顶</summary>
    <div className="actions"><button className="button" disabled={busy} onClick={()=>void read(0,null)}>{busy?'正在读取群成员…':page?'重新读取群成员':'查看本地群成员'}</button>
      {busy&&<button className="button" onClick={clear}>取消群成员读取</button>}
      <HelpTip label="本地群成员说明">读取所选群在当前账号本地缓存中的成员，每页最多200条。群内昵称仅从所选群的本地成员数据按精确账号匹配；无法识别的数据布局不提供昵称。联系人昵称和备注单独保留；没有匹配身份的记录保留未知。它不代表实时服务端成员或完整成员列表。翻页独立读取，关闭或过期会清除显示资料；不会发消息或修改群。</HelpTip>
    </div>
    {error&&<p className="error-notice" role="alert">{error}</p>}
    <DirectoryExport key={`${accountId}:${viewId}:${username}`} accountId={accountId} viewId={viewId} group={username} disabled={busy}/>
    {page&&<><details className="group-pins"><summary>本地置顶消息{page.cached_pins?` · ${page.cached_pins.length}条`:''}</summary>
      <HelpTip label="本地置顶消息说明">读取本机已缓存的群置顶消息编号，不代表实时服务端列表。按精确服务器编号查找所选消息分片中的原记录；原消息未缓存时不会按文字或昵称猜测，也不会恢复从未保存的内容。</HelpTip>
      {page.cached_pins==null?<p className="empty-inline">未取得可识别的置顶缓存。</p>:page.cached_pins.length?<ul className="history-messages" aria-label="本地置顶消息列表">{page.cached_pins.map(pin=><li key={pin.server_id}><code>{pin.server_id}</code>{onFindPin&&<button type="button" className="button" disabled={busy||pinBusy} onClick={()=>onFindPin(pin.server_id)}>查找原消息</button>}</li>)}</ul>:<p className="empty-inline">这份本地缓存的置顶列表为空。</p>}
    </details>{recipientViewId&&!busy&&<MentionComposer key={`${recipientViewId}:${index}:${generation.current}`} accountId={accountId} parentViewId={viewId} recipientViewId={recipientViewId} username={username} multi={multi} cursor={cursors.current[index]} rows={page.rows}/>} {page.owner_username&&<p className="field">本地记录的群主 <HelpTip label="查看本地群主标识"><span dir="auto">{page.owner_username}</span></HelpTip></p>}
      <label className="field">筛选本页<input aria-label="筛选本页群成员" value={filter} onChange={e=>setFilter(e.target.value)} maxLength={256} placeholder="群内昵称、昵称、备注或账号"/></label>
      <ul className="history-messages" aria-label="本地群成员列表">{visible.map(row=><li key={row.anchor.row_id}>
        <div className="group-member-heading"><strong dir="auto">{row.remark||row.group_nickname||row.nickname||row.anchor.username||'成员身份未解析'}</strong><HelpTip label="查看成员资料"><dl><dt>账号</dt><dd dir="auto">{row.anchor.username??'未解析'}</dd><dt>群内昵称</dt><dd dir="auto">{row.group_nickname||'未记录'}</dd><dt>本地昵称</dt><dd dir="auto">{row.nickname||'未记录'}</dd><dt>本地备注</dt><dd dir="auto">{row.remark||'未记录'}</dd></dl></HelpTip></div>
        {row.anchor.username&&<code dir="auto">{row.anchor.username}</code>}
      </li>)}</ul>
      {!visible.length&&<p className="empty-inline">{page.rows.length?'本页没有匹配的成员。':'此来源没有成员记录。'}</p>}
      <div className="actions"><button className="button" disabled={busy||index===0} onClick={()=>void read(index-1,cursors.current[index-1])}>上一页群成员</button><span className="field" role="status" aria-label="群成员分页">第{index+1}页 · 本页{page.rows.length}条</span><button className="button" disabled={busy||!page.next} onClick={()=>void read(index+1,page.next)}>下一页群成员</button></div>
    </>}
  </details>;
}
