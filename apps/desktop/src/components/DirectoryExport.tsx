import {useEffect,useRef,useState} from 'react';
import type {ConversationQuery} from '../lib/conversation-query';
import {command} from '../lib/desktop';
import {HelpTip} from './HelpTip';
import {contactCategories,type ContactCategory} from '../lib/contact-category';
type Category=ContactCategory;
interface Summary {rows:number;pages:number;bytes:number;avatars_written?:number;media_written?:number;source_count?:number;complete_local_selection:boolean;complete_server_directory:boolean;cross_page_snapshot:boolean}
export function DirectoryExport({accountId,viewId,group,searchQuery='',disabled=false,conversation,onBusyChange}:{accountId:string;viewId:string;group?:string;searchQuery?:string;disabled?:boolean;conversation?:{username:string;query?:ConversationQuery|null};onBusyChange?:(busy:boolean)=>void}) {
  const label=conversation?'会话消息':group?'群成员':'通讯录';
  const [format,setFormat]=useState('csv'),[name,setName]=useState(''),[scope,setScope]=useState('filtered');
  const [categories,setCategories]=useState<Category[]>([]),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const [result,setResult]=useState<{summary:Summary;path:string}|null>(null);
  const request=useRef<string|null>(null),generation=useRef(0);
  const [choosing,setChoosing]=useState(false);
  useEffect(()=>{setBusy(false);setChoosing(false);setResult(null);return()=>{generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});onBusyChange?.(false);};},[accountId,viewId,group,conversation?.username]);
  const defaultName=`WeRC-${label}-${new Date().toISOString().replace(/[:.]/g,'-')}.${format}`;
  async function save(){
    if(request.current)return;
    const id=crypto.randomUUID(),version=++generation.current,path=name.trim()||defaultName;
    request.current=id;setBusy(true);onBusyChange?.(true);setError('');setResult(null);
    const selection=conversation?{username:conversation.username,query:scope==='filtered'?conversation.query??{}:{}}:group?{kind:'group',username:group}:{kind:'contacts',search_query:scope==='filtered'?searchQuery||null:null,categories};
    try{const summary=await command<Summary>(conversation?'save_live_message_export':'save_directory_export',{accountId,viewId,selection,format,path,requestId:id});if(version===generation.current)setResult({summary,path});}
    catch(e){if(version===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(version===generation.current){request.current=null;setBusy(false);onBusyChange?.(false);}}
  }
  async function cancel(){if(request.current)await command('cancel_browse',{requestId:request.current}).catch(e=>setError(String(e)));}
  async function choose(){const version=generation.current;setChoosing(true);setError('');try{const path=await command<string|null>(format==='zip'?'choose_media_save_path':'choose_directory_export_path',{format,suggestedName:name.trim()||defaultName});if(version===generation.current&&path)setName(path);}catch(e){if(version===generation.current)setError(e instanceof Error?e.message:String(e));}finally{if(version===generation.current)setChoosing(false);}}
  return <details className="directory-export"><summary>{`导出${label}`}</summary>
    <form onSubmit={e=>{e.preventDefault();void save();}}>
      <div className="actions"><label>格式<select value={format} disabled={busy||disabled||choosing} onChange={e=>{setFormat(e.target.value);setName('');}}>{conversation&&<><option value="html">HTML / 离线阅读</option><option value="md">Markdown / 阅读文本</option></>}{!conversation&&!group&&<option value="zip">ZIP / 通讯录和已缓存头像</option>}<option value="xlsx">Excel / XLSX</option><option value="csv">CSV / 表格</option><option value="json">JSON / 结构化字段</option><option value="txt">TXT / 制表符文本</option></select></label>
        {!group&&<label>范围<select value={scope} disabled={busy||disabled||choosing} onChange={e=>setScope(e.target.value)}><option value="filtered">当前搜索的全部结果</option><option value="all">{conversation?'当前来源中此会话的全部消息':'本地全部联系人'}</option></select></label>}
        <HelpTip label={`${label}导出说明`}>{conversation?'按当前筛选条件读取全部续页，按时间正序保存，最多5万条或32MiB；不局限于当前显示页。包含分片序号、完整消息ID、时间、发送者、类型和可读取文字，不包含媒体文件、原始XML或下载凭据。多个分片保留各自记录，不合并重复ID；跨页不是同一时刻快照，来源或身份变化会中止。Excel按文本保存长ID和正文，不执行公式。选择全部消息会忽略当前筛选。':<>自动读取所有续页，最多5万条或32MiB，包含本地可核对的账号、昵称、备注及原始分类字段；不是只保存当前页。未解析的成员身份保持空值。本地缓存不代表当前完整好友或群成员关系，跨页不是同一时刻快照；选择ZIP可附带此前主动读取并加密保留的联系人头像，逐项报告已保存、未缓存和不可用，不在导出时联网；其他格式不附头像。取消、过期、源变化或读取失败时不会写出一个声称完整的文件。Excel支持筛选和冻结表头，全部字段按文本保存，保留长ID；CSV使用UTF-8，危险公式前缀转为文本。需要原始结构可选择JSON。</>}</HelpTip>
      </div>
      {!group&&!conversation&&<details><summary>按本地分类筛选</summary><fieldset disabled={busy||disabled||choosing}><legend>可选分类（不选则全部）</legend>{contactCategories.map(([key,label])=><label className="permission-choice" key={key}><input type="checkbox" checked={categories.includes(key)} onChange={e=>setCategories(old=>e.target.checked?[...old,key]:old.filter(c=>c!==key))}/>{key==='service'?'全部公众号（兼容旧分类）':label}</label>)}<HelpTip label="导出分类依据">按本地类型、关系标记、公众号类型及账号形态分组，删除标记优先；分类与浏览使用相同依据，覆盖全部续页。其他公众号选项兼容旧接口，包含所有未删除的gh_账号。本地标记不代表服务器当前关系，缺失依据保持未知。</HelpTip></fieldset></details>}
      <label>文件名或保存位置<input aria-label={`${label}导出文件名`} value={name} onChange={e=>setName(e.target.value)} disabled={busy||disabled||choosing} placeholder={defaultName}/></label>
      <div className="actions"><button className="button" type="button" disabled={busy||disabled||choosing} onClick={()=>void choose()}>{choosing?'正在选择…':'选择保存位置'}</button><button className="button" type="submit" disabled={busy||disabled||choosing}>{busy?'正在读取并导出…':'导出并保存'}</button>{busy&&<button className="button" type="button" onClick={()=>void cancel()}>取消导出</button>}<HelpTip label="导出保存位置">选择保存位置后再点击导出。不选择时默认使用当前电脑的系统下载目录，不依赖工作目录；可直接填文件名。目录需已存在，扩展名须与格式一致。同名文件不会覆盖。</HelpTip></div>
    </form>
    {error&&<p role="alert" className="error-notice">{error}</p>}
    {result&&<div role="status"><p>已保存{result.summary.rows}条记录，读取{result.summary.pages}页，{result.summary.bytes.toLocaleString()}字节；{conversation?`媒体${result.summary.media_written??0}个，${result.summary.source_count}个来源。`:`头像${result.summary.avatars_written??0}张。`}</p><button className="button" onClick={()=>void command('show_directory_export',{path:result.path}).catch(e=>setError(String(e)))}>查看保存位置</button></div>}
  </details>;
}
