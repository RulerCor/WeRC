import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
type Position={page:number;row:number};
type RecordItem={position:Position;username:string;sender:string|null;server_id:string|null;timestamp:number;local_status:string;direction:'incoming'|'outgoing'|'unknown';local_id:string;message_type:string;detail:{kind:string;title:string;text:string;fields:Record<string,string>;url:string|null}};
type Catalog={data:RecordItem[];next:Position|null;scanned:number;unreadable:number};
const categories:Record<string,string>={transfer:'转账消息',red_packet:'红包消息',friend_request:'好友验证记录',call:'通话记录',revoke:'撤回通知',link:'链接',contact:'名片',location:'位置',forward:'合并转发',quote:'引用',share:'分享',system:'系统记录'};

export function CaptureRecords({accountId,id,onLocate}:{accountId:string;id:string;onLocate:(position:Position)=>void}) {
  const [kind,setKind]=useState(''),[text,setText]=useState(''),[username,setUsername]=useState('');
  const [direction,setDirection]=useState(''),[rawStatus,setRawStatus]=useState(''),[selected,setSelected]=useState<Position[]>([]),[exportFormat,setExportFormat]=useState('xlsx');
  const [from,setFrom]=useState(''),[until,setUntil]=useState('');
  const [items,setItems]=useState<RecordItem[]>([]),[next,setNext]=useState<Position|null>(null);
  const [scanned,setScanned]=useState(0),[unreadable,setUnreadable]=useState(0),[loaded,setLoaded]=useState(false);
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const generation=useRef(0),request=useRef<string|null>(null);
  useEffect(()=>{setSelected([]);setItems([]);setNext(null);setLoaded(false);setScanned(0);setUnreadable(0);setBusy(false);setError('');setNotice('');return()=>{generation.current++;const pending=request.current;request.current=null;if(pending)void command('cancel_browse',{requestId:pending}).catch(()=>{});};},[accountId,id]);
  function change(action:()=>void){action();setSelected([]);setItems([]);setNext(null);setLoaded(false);setScanned(0);setUnreadable(0);}
  function query(){return {direction:direction||null,raw_status:rawStatus||null,kind:kind||null,text,username:username.trim()||null,from:from?Math.floor(new Date(from).getTime()/1000):null,until:until?Math.floor(new Date(until).getTime()/1000):null};}
  async function load(after:Position|null=null){
    if(request.current)return;
    const g=generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');
    try {const result=await command<Catalog>('capture_records',{accountId,id,query:query(),after,requestId});if(g===generation.current){setItems(old=>after?[...old,...result.data]:result.data);setNext(result.next);setScanned(old=>(after?old:0)+result.scanned);setUnreadable(old=>(after?old:0)+result.unreadable);setLoaded(true);}}
    catch(e){if(g===generation.current)setError(String(e));}
    finally{if(g===generation.current){request.current=null;setBusy(false);}}
  }
  async function save(format:string,onlySelected=false){
    if(request.current)return;
    const g=generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');setNotice('');
    const path=`WeRC-记录-${Date.now()}.${format}`;
    try {const count=await command<number>('capture_records_export',{accountId,id,query:{...query(),selected:onlySelected?selected:[]},format,path,requestId});if(g===generation.current)setNotice(`已将 ${count} 条匹配记录保存到下载目录：${path}`);}
    catch(e){if(g===generation.current)setError(String(e));}
    finally{if(g===generation.current){request.current=null;setBusy(false);}}
  }
  async function original(item:RecordItem){
    if(request.current)return;
    const g=generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');
    try{const location=await command<{position:Position}>('capture_message_location',{accountId,id,username:item.detail.fields['原会话']||item.username,serverId:item.detail.fields['原消息编号'],requestId});if(g===generation.current)onLocate(location.position);}
    catch(e){if(g===generation.current)setError(String(e));}
    finally{if(g===generation.current){request.current=null;setBusy(false);}}
  }
  return <details className="local-tools"><summary>专项记录中心</summary>
    <div className="actions"><HelpTip label="专项记录说明">从所选聊天归档分类提取记录，可离线浏览。转账和红包只展示消息内的金额、备注及原始状态码，不代表到账、余额或完整账单。撤回通知不代表已找回被撤回的正文。时间范围按本机时间，结束时间不包含在内。</HelpTip></div>
    <div className="history-query">
      <label>记录分类<select disabled={busy} value={kind} onChange={e=>change(()=>setKind(e.target.value))}><option value="">全部记录</option>{Object.entries(categories).map(([key,name])=><option value={key} key={key}>{name}</option>)}</select></label>
      <label>搜索内容<input disabled={busy} value={text} placeholder="标题、备注、可读字段" onChange={e=>change(()=>setText(e.target.value))}/></label>
      <button className="button" disabled={busy} onClick={()=>void load()}>查找记录</button>
    </div>
    <details><summary>方向、状态与时间范围</summary><div className="history-query">
      <label>消息方向<select disabled={busy} value={direction} onChange={e=>change(()=>setDirection(e.target.value))}><option value="">全部方向</option><option value="incoming">接收</option><option value="outgoing">发送</option><option value="unknown">未知</option></select></label>
      <label>原始状态码<input disabled={busy} value={rawStatus} maxLength={64} placeholder="留空不限制" onChange={e=>change(()=>setRawStatus(e.target.value))}/></label>
      <label>会话账号<input disabled={busy} value={username} onChange={e=>change(()=>setUsername(e.target.value))}/></label>
      <label>开始时间<input type="datetime-local" disabled={busy} value={from} onChange={e=>change(()=>setFrom(e.target.value))}/></label>
      <label>结束时间<input type="datetime-local" disabled={busy} value={until} onChange={e=>change(()=>setUntil(e.target.value))}/></label>
    </div></details>
    {loaded&&<p role="status">已扫描 {scanned} 条消息 · 显示 {items.length} 条记录{unreadable?` · ${unreadable} 条原始内容无法解析`:''}{next?' · 可继续扫描':''}</p>}
    <ul className="recovery-messages">{items.map(item=><li key={`${item.position.page}:${item.position.row}`}>
      <label><input type="checkbox" disabled={busy||(selected.length>=1000&&!selected.some(p=>p.page===item.position.page&&p.row===item.position.row))} aria-label={`选择记录 ${item.position.page}-${item.position.row}`} checked={selected.some(p=>p.page===item.position.page&&p.row===item.position.row)} onChange={e=>setSelected(old=>e.target.checked?[...old,item.position]:old.filter(p=>p.page!==item.position.page||p.row!==item.position.row))}/>选择记录</label>
      <div>{categories[item.detail.kind]} · {item.username} · {new Date(item.timestamp*1000).toLocaleString()}</div>
      <strong className="message-body">{item.detail.title}</strong><p className="message-body">{item.detail.text}</p>
      {item.local_status==='edited_local'&&<span className="state-tag">本地已修改</span>}
      <details><summary>详细字段与来源</summary><dl>{Object.entries(item.detail.fields).map(([label,value])=><div key={label}><dt>{label}</dt><dd className="message-body">{value}</dd></div>)}</dl>
        {item.sender&&<p className="message-body">发送者：{item.sender}</p>}{item.server_id&&<p className="message-body">消息编号：{item.server_id}</p>}
        {item.detail.url&&<><p className="message-body">{item.detail.url}</p><button className="button" onClick={()=>void navigator.clipboard.writeText(item.detail.url!).then(()=>setNotice('链接已复制。')).catch(()=>setError('复制失败，可选中文字复制。'))}>复制链接</button></>}
      </details>
      <button className="button" disabled={busy} onClick={()=>onLocate(item.position)}>查看来源消息页</button>
      {item.detail.kind==='revoke'&&item.detail.fields['原消息编号']&&<button className="button" disabled={busy} onClick={()=>void original(item)}>定位被撤回的原消息</button>}
    </li>)}</ul>
    {loaded&&!items.length&&<p>{next?'当前扫描范围没有匹配记录，可以继续扫描。':'没有匹配记录。'}</p>}
    <div className="actions">{next&&<button className="button" disabled={busy} onClick={()=>void load(next)}>继续扫描记录</button>}<details><summary>导出记录</summary><div className="actions"><label>文件格式<select disabled={busy} value={exportFormat} onChange={e=>setExportFormat(e.target.value)}><option value="xlsx">Excel分类工作表</option><option value="csv">CSV</option><option value="json">JSON</option><option value="jsonl">JSONL</option></select></label><button className="button" disabled={busy} onClick={()=>void save(exportFormat)}>导出全部匹配记录</button><button className="button" disabled={busy||!selected.length} onClick={()=>void save(exportFormat,true)}>导出选中记录（{selected.length}）</button><HelpTip label="记录导出说明">扫描整个归档，包括尚未加载的匹配项；也可勾选记录单独导出。Excel按分类分工作表，长正文另存“长文本”，编号保留为文字。CSV防止公式执行；JSON保留完整结构和编号。消息方向来自已保存的发送者，不推断资金流向或到账状态。文件包含个人资料且不加密。</HelpTip></div></details></div>
    {busy&&<button className="button" onClick={()=>{if(request.current)void command('cancel_browse',{requestId:request.current}).catch(e=>setError(String(e)));}}>取消当前操作</button>}
    {notice&&<p role="status">{notice}</p>}{error&&<p role="alert" className="error-notice">{error}</p>}
  </details>;
}
