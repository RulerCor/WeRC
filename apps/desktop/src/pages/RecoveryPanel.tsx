import {HelpTip} from '../components/HelpTip';
import {LiveSender} from '../components/LiveSender';
import {useEffect,useRef,useState} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
import {safeTime} from '../lib/browse';
import type {LiveMessage} from './LiveChat';
interface Summary {id:string;captured_at:string;rows:number;text_rows:number;has_older:boolean;gaps_at_capture:number|null}
interface Page {summary:Summary;rows:LiveMessage[];source_indices:number[];source_count:number;offset:number;next_offset:number|null;mode:'saved_source_observation';gaps_closed:false}
export function RecoveryPanel({accountId,viewId,disabled,multi=false}:{accountId:string;viewId:string|null;disabled:boolean;multi?:boolean}) {
  const [summaries,setSummaries]=useState<Summary[]>([]),[page,setPage]=useState<Page|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const generation=useRef(0);
  useEffect(()=>{generation.current++;setSummaries([]);setPage(null);setError('');setNotice('');setBusy(false);return()=>{generation.current++;};},[accountId]);
  async function action(kind:'save'|'list'|'page',id?:string,offset=0){
    const g=++generation.current;setBusy(true);setError('');setNotice('');
    try {
      if(kind==='save'){
        const s=await command<Summary>(multi?'native_recovery_multi_save':'native_recovery_save',{accountId,viewId,confirmed:true});
        if(g!==generation.current)return;setNotice(`已加密保存本次窗口的${s.rows}条来源观测，其中${s.text_rows}条文字可读。接收缺口仍待核验。`);
      }
      if(kind==='page'){
        const p=await command<Page>('native_recovery_page',{accountId,id,offset});
        if(g===generation.current)setPage(p);
      }else{
        const values=await command<Summary[]>('native_recovery_list',{accountId});
        if(g===generation.current)setSummaries(values);
      }
    }catch(e){if(g===generation.current){setPage(null);setError(e instanceof Error?e.message:String(e));}}
    finally{if(g===generation.current)setBusy(false);}
  }
  return <details className="recovery-panel"><summary>加密补读记录</summary>
    
    <div className="actions"><button className="button" disabled={!viewId||disabled||busy||!desktopAvailable} onClick={()=>void action('save')}>保存本次窗口</button><button className="button" disabled={busy||!desktopAvailable} onClick={()=>void action('list')}>查看已保存窗口</button><HelpTip label="加密补读记录说明">保存本次读取窗口（最多2048条），可在断开或重启后查看。相同窗口不重复保存；内容变化保留另一个版本。来源记录与实时接收分别展示，接收缺口仍待核验。</HelpTip></div>
    {error&&<p className="error-notice" role="alert">{error}</p>}{notice&&<p className="field" role="status">{notice}</p>}
    <ul className="recovery-windows">{summaries.map(s=><li key={s.id}><button className="button" disabled={busy} onClick={()=>void action('page',s.id)}>{new Date(s.captured_at).toLocaleString()} · {s.rows}条来源观测</button></li>)}</ul>
    {page&&<div aria-label="已保存的来源补读"><p className="field">已保存来源 / 非当前实时结果 / 不代表缺口完整补齐或发送回执{page.source_count>1&&` / ${page.source_count}个分片；编号仅用于此保存窗口`}</p><ol className="recovery-messages">{page.rows.map((r,i)=><li key={`${page.source_indices?.[i]??1}:${r.local_id}`}><div className="message-heading"><LiveSender username={r.sender_username}/><time>{safeTime(r.timestamp)}</time>{page.source_count>1&&<span className="field"> / 分片{page.source_indices[i]}</span>}</div><p className="message-body" dir="auto">{r.text??'正文尚未解析'}</p></li>)}</ol><div className="actions"><button className="button" disabled={busy||page.offset===0} onClick={()=>void action('page',page.summary.id,Math.max(0,page.offset-60))}>补读上一页</button><button className="button" disabled={busy||page.next_offset===null} onClick={()=>void action('page',page.summary.id,page.next_offset!)}>补读下一页</button></div></div>}
  </details>;
}
