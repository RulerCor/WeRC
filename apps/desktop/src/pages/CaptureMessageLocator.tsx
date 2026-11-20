import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';

export function CaptureMessageLocator({accountId,id,disabled=false,onLocate}:{accountId:string;id:string;disabled?:boolean;onLocate:(position:{page:number;row:number})=>void}){
 const [username,setUsername]=useState(''),[serverId,setServerId]=useState(''),[busy,setBusy]=useState(false),[error,setError]=useState('');
 const generation=useRef(0),request=useRef<string|null>(null);
 useEffect(()=>{setUsername('');setServerId('');setBusy(false);setError('');return()=>{generation.current++;const pending=request.current;request.current=null;if(pending)void command('cancel_browse',{requestId:pending}).catch(()=>{});};},[accountId,id]);
 async function locate(){
  if(request.current)return;const g=generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');
  try{const value=await command<{position:{page:number;row:number}}>('capture_message_location',{accountId,id,username:username.trim(),serverId:serverId.trim(),requestId});if(g===generation.current)onLocate(value.position);}
  catch(e){if(g===generation.current)setError(String(e));}
  finally{if(g===generation.current){request.current=null;setBusy(false);}}
 }
 return <details className="local-tools"><summary>按消息编号定位</summary><div className="actions"><HelpTip label="精确定位说明">输入会话账号及服务器消息编号，在当前归档全部页中查找。编号始终按文字处理，避免长数字精度丢失。相同消息的多个一致副本只定位一处；副本冲突会提示。不能按昵称或相似正文猜测；未同步、已删除或未归档的消息无法定位。</HelpTip></div><form className="history-query" onSubmit={e=>{e.preventDefault();void locate();}}><label>会话账号<input value={username} disabled={busy||disabled} maxLength={256} onChange={e=>setUsername(e.target.value)}/></label><label>服务器消息编号<input value={serverId} disabled={busy||disabled} maxLength={20} inputMode="numeric" onChange={e=>setServerId(e.target.value)}/></label><button className="button" disabled={busy||disabled||!username.trim()||!serverId.trim()}>定位原消息</button></form>{busy&&<button className="button" onClick={()=>{const pending=request.current;request.current=null;generation.current++;setBusy(false);if(pending)void command('cancel_browse',{requestId:pending}).catch(()=>{});}}>取消查找</button>}{error&&<p role="alert" className="error-notice">{error}</p>}</details>;
}
