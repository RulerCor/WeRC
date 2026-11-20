import {useEffect,useRef,useState,type FormEvent} from 'react';
import {command} from '../lib/desktop';
import {MAX_TEXT_BYTES,textByteLength,validSendText} from '../lib/send-text';
import type {PreparedRecipient} from '../lib/recipients';
import {HelpTip} from '../components/HelpTip';
import {sendStates,type SendPreview,type SendStatus} from '../lib/outbox';

/** Uses the same durable, single-attempt outbox as the full queue page. */
export function ConversationComposer({accountId,viewId,username,multi,disabled}:{accountId:string;viewId:string;username:string;multi:boolean;disabled:boolean}) {
  const [text,setText]=useState(''),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const [preview,setPreview]=useState<SendPreview|null>(null),[confirmed,setConfirmed]=useState(false);
  const key=useRef(crypto.randomUUID()),created=useRef<string|null>(null),running=useRef(false),alive=useRef(true);
  useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
  const setFailure=(e:unknown)=>{if(alive.current)setError(e instanceof Error?e.message:String(e));};
  async function perform(action:()=>Promise<void>){
    if(running.current)return;
    running.current=true;setBusy(true);setError('');
    try{await action();}catch(e){setFailure(e);}finally{running.current=false;if(alive.current)setBusy(false);}
  }
  async function refresh(id:string){
    const value=await command<SendPreview>('preview_send',{accountId,outboxId:id});
    if(alive.current)setPreview(value);
  }
  async function save(event:FormEvent){
    event.preventDefault();if(!validSendText(text)||disabled)return;
    await perform(async()=>{
      if(!created.current){
        const recipients=await command<PreparedRecipient[]>('native_prepared_recipients',{accountId});
        if(!alive.current)return;
        if(!recipients.some(r=>r.target_id===username&&(r.expires_at===null||r.expires_at>Date.now()/1000))){
          await command('native_prepare_recipient',{accountId,selection:{view_id:viewId,multi}});
        }
        if(!alive.current)return;
        const result=await command<SendStatus>('create_text_draft',{accountId,targetId:username,text,idempotencyKey:key.current});
        created.current=result.metadata.id;
      }
      if(alive.current)await refresh(created.current!);
    });
  }
  async function approve(){
    if(!preview||!confirmed||preview.status.state!=='awaiting_approval')return;
    const current=preview;
    await perform(async()=>{
      const status=await command<SendStatus>('approve_send',{accountId,outboxId:current.status.metadata.id,requestDigest:current.status.metadata.request_digest,confirmed:true,scheduledAt:null});
      if(alive.current){setPreview({...current,status});setConfirmed(false);}
    });
  }
  async function cancel(){
    if(!created.current)return;
    const id=created.current;
    await perform(async()=>{
      await command('cancel_send',{accountId,outboxId:id});
      if(alive.current){setPreview(null);created.current=null;key.current=crypto.randomUUID();setConfirmed(false);}
    });
  }
  useEffect(()=>{
    if(!preview||!['queued','submitting'].includes(preview.status.state))return;
    const id=preview.status.metadata.id;
    const timer=window.setInterval(()=>{if(!running.current)void perform(()=>refresh(id));},2000);
    return()=>window.clearInterval(timer);
  },[preview?.status.metadata.id,preview?.status.state]);
  const state=preview?.status.state;
  const canCancel=Boolean(created.current&&(!state||['awaiting_approval','queued'].includes(state)));
  const canWriteNext=Boolean(state&&!['awaiting_approval','queued','submitting'].includes(state));
  function next(){created.current=null;key.current=crypto.randomUUID();setPreview(null);setText('');setConfirmed(false);setError('');}
  return <section className="conversation-compose" aria-label="在当前会话发送文字">
    <div className="section-title"><h3>写消息</h3><HelpTip label="会话内发送说明">先保存草稿并核对当前账号、唯一收件人和文字，再批准一次。草稿和结果与发送队列共用，离开聊天后也能从发送队列找回。输入Enter换行，不会直接发送。文字最多10000个UTF-8字节；媒体、定时发送和已保存草稿可在发送队列处理。结果未知时只回查，不自动重发。</HelpTip></div>
    {!preview&&<form onSubmit={save}>
      <label>消息文字<textarea aria-label="当前会话消息文字" value={text} disabled={busy||disabled||Boolean(created.current)} onChange={e=>{setText(e.target.value);key.current=crypto.randomUUID();setError('');}} placeholder="输入消息，先预览再发送"/></label>
      <div className="actions"><span className="field" role="status">{textByteLength(text)} / {MAX_TEXT_BYTES} 字节</span><button type="submit" className="button primary" disabled={busy||disabled||!validSendText(text)}>{created.current?'重新打开草稿预览':busy?'正在核对并保存…':'预览消息'}</button></div>
    </form>}
    {preview&&<div className="conversation-send-preview" aria-label="当前会话发送预览">
      <dl><dt>发送者</dt><dd dir="auto">{preview.payload.context.sender_id}</dd><dt>收件人</dt><dd dir="auto">{preview.payload.context.target_display_name}<code>{preview.payload.context.target_id}</code></dd><dt>方式</dt><dd>{{native:'后台原生',visible_automation:'可见自动化',synthetic:'自有模拟 / 不发微信'}[preview.payload.context.execution_mode]}</dd><dt>状态</dt><dd role="status">{sendStates[preview.status.state]}</dd></dl>
      <p className="message-body" dir="auto">{preview.payload.text}</p>
      {state==='awaiting_approval'&&<><label className="check-line"><input type="checkbox" checked={confirmed} disabled={busy} onChange={e=>setConfirmed(e.target.checked)}/>确认以上账号、收件人和完整文字</label><button className="button primary" disabled={busy||disabled||!confirmed||!preview.payload.context.identity_verified||!preview.payload.context.target_verified||preview.status.metadata.expires_at<=Date.now()/1000} onClick={()=>void approve()}>批准发送一次</button></>}
      <div className="actions"><button className="button" disabled={busy} onClick={()=>void perform(()=>refresh(preview.status.metadata.id))}>刷新任务状态</button>{['unknown','server_accepted'].includes(preview.status.state)&&<button className="button" disabled={busy} onClick={()=>void perform(async()=>{await command('reconcile_send',{accountId,outboxId:preview.status.metadata.id});if(alive.current)await refresh(preview.status.metadata.id);})}>回查结果 / 不重发</button>}{canWriteNext&&<button className="button" disabled={busy||disabled} onClick={next}>写下一条消息</button>}</div>
    </div>}
    {canCancel&&<button className="button" disabled={busy} onClick={()=>void cancel()}>取消此草稿</button>}
    {error&&<p className="error-notice" role="alert">{error}</p>}
  </section>;
}
