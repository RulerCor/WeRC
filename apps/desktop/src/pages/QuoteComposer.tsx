import {useEffect,useRef,useState,type FormEvent} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {MAX_TEXT_BYTES,textByteLength,validSendText} from '../lib/send-text';
import type {PreparedRecipient} from '../lib/recipients';
export function QuoteComposer({accountId,viewId,localId,source,username}:{accountId:string;viewId:string;localId:string;source:string|null;username:string}) {
  const [text,setText]=useState(''),[busy,setBusy]=useState(false),[notice,setNotice]=useState(''),[error,setError]=useState('');
  const key=useRef(crypto.randomUUID()),running=useRef(false),alive=useRef(true);
  useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
  async function save(event:FormEvent) {
    event.preventDefault();if(running.current||!validSendText(text))return;
    running.current=true;setBusy(true);setNotice('');setError('');
    try {
      const recipients=await command<PreparedRecipient[]>('native_prepared_recipients',{accountId});
      if(!alive.current)return;
      if(!recipients.some(r=>r.target_id===username))await command('native_prepare_recipient',{accountId,selection:{view_id:viewId,multi:source!==null}});
      if(!alive.current)return;
      await command('create_quote_draft',{accountId,text,idempotencyKey:key.current,selection:{view_id:viewId,local_id:localId,source}});
      if(alive.current){setText('');key.current=crypto.randomUUID();setNotice('引用草稿已保存。前往「发送队列」核对原文、回复和收件人，再批准发送。');}
    }catch(e){if(alive.current)setError(e instanceof Error?e.message:String(e));}
    finally{running.current=false;if(alive.current)setBusy(false);}
  }
  return <details className="quote-compose"><summary>引用回复</summary>
    <HelpTip label="引用回复说明">回复本条普通文字消息。原文从当前已核验的会话中选取；这里只保存草稿，原文和回复都可在发送队列完整预览。提交时还会核对微信中的原文摘要，找不到或内容变化就不发送。回复最多10000个UTF-8字节；结果未知时不会自动重发。</HelpTip>
    <form onSubmit={save}>
      <label>回复文字<textarea aria-label="引用回复文字" disabled={busy} value={text} onChange={e=>{setText(e.target.value);key.current=crypto.randomUUID();setNotice('');}}/></label>
      <span className="field" role="status">{textByteLength(text)} / {MAX_TEXT_BYTES} 字节</span>
      <button className="button" disabled={busy||!validSendText(text)}>{busy?'正在核对并保存…':'保存引用草稿'}</button>
    </form>
    {notice&&<p role="status">{notice}</p>}{error&&<p className="error-notice" role="alert">{error}</p>}
  </details>;
}
