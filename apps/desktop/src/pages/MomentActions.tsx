import {useEffect,useRef,useState} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {momentActionLabels,type MomentActionKind,type MomentReplyTarget,type SendStatus} from '../lib/outbox';

export function MomentActions({accountId,snsId,reply}:{accountId:string;snsId:string;reply?:MomentReplyTarget}) {
  const initial=reply?'reply':'like';
  const [action,setAction]=useState<MomentActionKind>(initial),[text,setText]=useState('');
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[saved,setSaved]=useState<string|null>(null);
  const key=useRef(crypto.randomUUID()),running=useRef(false),generation=useRef(0);
  useEffect(()=>{
    generation.current++;key.current=crypto.randomUUID();running.current=false;
    setSaved(null);setError('');setBusy(false);setText('');setAction(reply?'reply':'like');
    return()=>{generation.current++;};
  },[accountId,snsId,reply?.id,reply?.id64,reply?.username]);
  function changed(){key.current=crypto.randomUUID();setSaved(null);setError('');}
  const bytes=new TextEncoder().encode(text).length;
  const hasText=action==='comment'||action==='reply',limit=action==='reply'?895:1023;
  const valid=!hasText||(text.trim().length>0&&bytes<=limit&&!text.includes('\0'));
  async function save(){
    if(running.current||saved||!valid)return;
    const current=generation.current;
    running.current=true;setBusy(true);setError('');
    try {
      const status=await command<SendStatus>('create_moment_action_draft',{
        accountId,action:{sns_id:snsId,action,...(action==='reply'?{reply}:{})},text:hasText?text:'',idempotencyKey:key.current,
      });
      if(current===generation.current)setSaved(status.metadata.id);
    }catch(e){if(current===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(current===generation.current){running.current=false;setBusy(false);}}
  }
  return <details className="moment-actions"><summary>{reply?'回复这条评论':'互动'}</summary>
    <div className="button-row">{reply?<span className="message-body">回复 {reply.username}</span>:<label>操作<select value={action} disabled={busy} onChange={e=>{setAction(e.target.value as MomentActionKind);changed();}}>{(['like','comment','unlike','delete'] as const).map(kind=><option key={kind} value={kind}>{momentActionLabels[kind]}</option>)}</select></label>}
      <HelpTip label="朋友圈互动说明">操作绑定这条动态的完整标识。保存后请到“发送队列”预览并批准一次。原生提交成功仍可能显示“结果未知”，请刷新朋友圈查看缓存；不要为等待结果重复创建任务。Bot可使用同一接口和幂等键直接执行已授权操作。</HelpTip></div>
    {hasText&&<label>{action==='reply'?'回复内容':'评论内容'}<textarea value={text} disabled={busy} onChange={e=>{setText(e.target.value);changed();}}/><span className="field">{bytes} / {limit} 字节</span></label>}
    {action==='delete'&&<p role="note">删除会移除微信上的本人动态。请在队列预览中核对动态编号并确认。</p>}
    <button className="button" disabled={!desktopAvailable||busy||!valid||!!saved} onClick={()=>void save()}>{busy?'正在保存…':saved?'已保存':'保存到发送队列'}</button>
    {saved&&<p role="status">请在“发送队列”预览和批准。<code>{saved}</code></p>}
    {error&&<p className="error-notice" role="alert">{error}</p>}
  </details>;
}
