import {textByteLength,MAX_TEXT_BYTES} from '../lib/send-text';
import {useEffect,useRef,useState,type FormEvent} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {validMentionDraft,validMentionMember} from '../lib/mentions';
import type {PreparedRecipient} from '../lib/recipients';
interface Member {anchor:{username:string|null};nickname:string|null;remark:string|null;group_nickname?:string|null}
export function MentionComposer({accountId,parentViewId,recipientViewId,username,multi,cursor,rows}:{accountId:string;parentViewId:string;recipientViewId:string;username:string;multi:boolean;cursor:string|null;rows:Member[]}) {
  const [members,setMembers]=useState<string[]>([]),[text,setText]=useState(''),[busy,setBusy]=useState(false),[notice,setNotice]=useState(''),[error,setError]=useState('');
  const key=useRef(crypto.randomUUID()),running=useRef(false),alive=useRef(true);
  useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
  const options=rows.filter(row=>validMentionMember(row.anchor.username)).filter((row,i,all)=>all.findIndex(other=>other.anchor.username===row.anchor.username)===i);
  async function save(event:FormEvent){
    event.preventDefault();if(running.current||!validMentionDraft(members,text))return;
    running.current=true;setBusy(true);setError('');setNotice('');
    try{
      const recipients=await command<PreparedRecipient[]>('native_prepared_recipients',{accountId});
      if(!alive.current)return;
      if(!recipients.some(r=>r.target_id===username))await command('native_prepare_recipient',{accountId,selection:{view_id:recipientViewId,multi}});
      if(!alive.current)return;
      await command('create_mention_draft',{accountId,text,idempotencyKey:key.current,selection:{recipient:{view_id:recipientViewId,multi},parent_view_id:parentViewId,cursor,members}});
      if(alive.current){setMembers([]);setText('');key.current=crypto.randomUUID();setNotice('草稿已保存。前往「发送队列」预览正文、群和被提醒成员，再批准发送。');}
    }catch(e){if(alive.current)setError(e instanceof Error?e.message:String(e));}
    finally{running.current=false;if(alive.current)setBusy(false);}
  }
  return <details className="mention-compose"><summary>提醒群成员（@）</summary>
    <HelpTip label="群成员提醒说明">选择本页已识别的1至8位成员。保存时会重新核对成员页、当前会话和登录身份；这里只保存草稿，之后需在发送队列预览并批准。它使用原生@成员字段，本地名单可能已过时，不支持@所有人。文字上限10000个UTF-8字节。</HelpTip>
    <form onSubmit={save}>
      <fieldset className="mention-members"><legend>被提醒成员</legend>{options.map(row=>{const id=row.anchor.username!;return <label className="mention-member-choice" key={id}><input type="checkbox" value={id} checked={members.includes(id)} disabled={busy||(!members.includes(id)&&members.length>=8)} onChange={e=>{setMembers(current=>e.target.checked?[...current,id]:current.filter(item=>item!==id));key.current=crypto.randomUUID();setNotice('');}}/><span dir="auto">{row.remark||row.group_nickname||row.nickname||id}<small>{id}</small></span></label>;})}</fieldset>
      <span className="field" role="status">已选{members.length}/8位</span>
      <label>提醒文字<textarea aria-label="群提醒文字" disabled={busy} value={text} onChange={e=>{setText(e.target.value);key.current=crypto.randomUUID();setNotice('');}}/></label>
      <span className="field" role="status">{textByteLength(text)} / {MAX_TEXT_BYTES} 字节</span>
      <button className="button" disabled={busy||!validMentionDraft(members,text)}>{busy?'正在核对并保存…':'保存 @ 草稿'}</button>
    </form>
    {notice&&<p role="status">{notice}</p>}{error&&<p className="error-notice" role="alert">{error}</p>}
  </details>;
}
