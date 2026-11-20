import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {recipientAvailable,type PreparedRecipient} from '../lib/recipients';
import {HelpTip} from './HelpTip';

export function RecipientPicker({accountId,enabled,busy,value,onChange,onReady}:{accountId:string;enabled:boolean;busy:boolean;value:string;onChange:(value:string)=>void;onReady:(ready:boolean)=>void}){
  const [recipients,setRecipients]=useState<PreparedRecipient[]>([]),[loading,setLoading]=useState(false),[error,setError]=useState(''),[time,setTime]=useState(Date.now());
  const generation=useRef(0);
  const callbacks=useRef({onChange,onReady});callbacks.current={onChange,onReady};
  async function refresh(){
    const ticket=++generation.current;setLoading(true);setError('');callbacks.current.onReady(false);
    try{const rows=await command<PreparedRecipient[]>('native_prepared_recipients',{accountId});
      if(generation.current===ticket){setRecipients(rows);setTime(Date.now());}
    }catch(e){if(generation.current===ticket){setRecipients([]);setError(e instanceof Error?e.message:String(e));}}
    finally{if(generation.current===ticket)setLoading(false);}
  }
  useEffect(()=>{
    setRecipients([]);callbacks.current.onReady(false);
    if(enabled)void refresh();
    return()=>{generation.current++;};
  },[accountId,enabled]);
  useEffect(()=>{const timer=window.setInterval(()=>setTime(Date.now()),1000);return()=>window.clearInterval(timer);},[]);
  const current=recipients.find(r=>r.target_id===value),ready=enabled&&!loading&&Boolean(current&&recipientAvailable(current,time));
  useEffect(()=>{callbacks.current.onReady(ready);},[ready]);
  return <section className="recipient-picker" aria-label="发送收件人">
    <div className="section-title"><label>收件人<select aria-label="草稿收件标识" value={value} disabled={busy||loading||!enabled} onChange={e=>onChange(e.target.value)}>
      {!recipients.some(r=>r.target_id===value)&&<option value={value}>{value==='filehelper'?'文件传输助手':value||'选择收件人'}</option>}
      {recipients.map(r=><option key={r.target_id} value={r.target_id} disabled={!recipientAvailable(r,time)}>{r.display_name}{!recipientAvailable(r,time)?'（已过期）':''}</option>)}
    </select></label><HelpTip label="如何选择收件人">文字、语音、图片、文件、视频和表情使用同一个收件人。要发给联系人或群，请在「当前微信」打开会话，点击「设为收件人」，再返回这里选择。准备的会话五分钟内有效。创建草稿不会外发，预览中会再次显示完整收件标识。</HelpTip>
    <button className="button" disabled={!enabled||busy||loading} onClick={()=>void refresh()}>{loading?'正在核验…':'刷新收件人'}</button></div>
    {current&&<p className="field recipient-id" dir="auto"><span>{current.target_id.length>28?`${current.target_id.slice(0,24)}…`:current.target_id}</span><HelpTip label="收件人有效期"><span dir="auto">{current.target_id}</span><br/>{current.expires_at?'会话选择五分钟后过期，重新选择会使原草稿的批准失效。':'来自当前已核验的登录通道；断开连接后失效。'}</HelpTip></p>}
    {enabled&&!loading&&!ready&&!error&&<p role="status">收件人已过期或尚未核验。请从会话重新选择；不会自动切换到其他收件人。</p>}
    {error&&<p className="error-notice" role="alert">{error}</p>}
  </section>;
}
