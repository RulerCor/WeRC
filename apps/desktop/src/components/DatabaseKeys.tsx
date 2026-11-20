import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from './HelpTip';
interface Key {salt:string|null;key_hex:string;verified:boolean}
export function DatabaseKeys({accountId,disabled}:{accountId:string;disabled:boolean}){
  const [keys,setKeys]=useState<Key[]|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const generation=useRef(0),timer=useRef<ReturnType<typeof setTimeout>|null>(null),working=useRef(false);
  function clear(){generation.current++;working.current=false;setBusy(false);if(timer.current)clearTimeout(timer.current);timer.current=null;setKeys(null);setNotice('');}
  useEffect(()=>{clear();working.current=false;setBusy(false);setError('');return()=>{generation.current++;if(timer.current)clearTimeout(timer.current);};},[accountId]);
  async function read(){
    if(working.current)return;working.current=true;setBusy(true);setError('');const expected=++generation.current;
    try{const result=await command<Key[]>('reveal_database_keys',{accountId});if(expected!==generation.current)return;setKeys(result);timer.current=setTimeout(clear,30000);}
    catch(e){if(expected===generation.current)setError(String(e));}finally{if(expected===generation.current){working.current=false;setBusy(false);}}
  }
  async function copy(key:string){const expected=generation.current;try{await navigator.clipboard.writeText(key);if(expected===generation.current)setNotice('已复制所选数据库密钥。');}catch{if(expected===generation.current)setError('复制失败，可选中显示的密钥手动复制。');}}
  return <details className="permission-details" onToggle={e=>{if(!e.currentTarget.open)clear();}}><summary>查看受保护的数据库密钥</summary><div className="actions"><button className="button" disabled={disabled||busy} onClick={()=>void read()}>{busy?'正在读取…':keys?'重新显示30秒':'显示30秒'}</button>{keys&&<button className="button" onClick={clear}>立即隐藏</button>}<HelpTip label="数据库密钥说明">只在此处主动显示本机已保存的数据库密钥。不同数据库可能使用不同密钥，以数据库salt区分。显示30秒后自动收起，关闭面板或离开页面也会清除显示。复制后内容进入系统剪贴板，不会随面板关闭而自动删除。</HelpTip></div>
    {keys?.map((key,i)=><details key={key.salt??i}><summary>{key.salt?`数据库salt ${key.salt}`:'手动密钥（尚未验真）'}</summary><code className="message-body" style={{userSelect:'text',overflowWrap:'anywhere'}}>{key.key_hex}</code><button className="button" onClick={()=>void copy(key.key_hex)}>复制此密钥</button></details>)}
    {notice&&<p role="status">{notice}</p>}{error&&<p className="error-notice" role="alert">{error}</p>}
  </details>;
}
