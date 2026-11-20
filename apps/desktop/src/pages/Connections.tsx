import { useState, type FormEvent } from 'react';
import { Cable, Plus, Trash2 } from 'lucide-react';
import { command, desktopAvailable } from '../lib/desktop';
import type { Workbench } from '../hooks/useWorkbench';
import { useFeedbackPresence } from '../hooks/useFeedbackMotion';
import { ConnectionSetup } from './ConnectionSetup';
import { NativeConnection } from './NativeConnection';

const accountStates: Record<string,string> = {registered:'未连接',needs_key:'待配置',ready:'密钥首验通过',unsupported:'未适配'};

export function Connections(workbench: Workbench) {
  const {data,busy,act}=workbench;
  const slot=data.current_connection;
  const current=data.accounts.find(a=>a.id===slot.account_id);
  const archives=data.accounts.filter(a=>a.id!==slot.account_id);
  const [configure,setConfigure]=useState(false),[label,setLabel]=useState('');
  const [choice,setChoice]=useState(''),[confirmed,setConfirmed]=useState(false);
  const [removing,setRemoving]=useState<string|null>(null);
  const confirmation=useFeedbackPresence(removing);
  async function register(event:FormEvent) {
    event.preventDefault();
    await act(async()=>{await command('register_account',{label});setLabel('');setConfigure(true);});
  }
  return <>
    <div className="section-title"><h2>当前微信</h2><span className="state-tag">单机 / 单微信</span></div>
    {slot.selection_required ? <form className="register-form" onSubmit={e=>{e.preventDefault();void act(async()=>{await command('select_current_connection',{accountId:choice,revision:slot.revision,confirmed});setConfirmed(false);setChoice('');});}}>
      <h3>确认原有资料</h3><p>选择唯一当前来源，其他旧资料只作归档。密钥和记录保持不变。</p>
      <label>当前来源<select aria-label="迁移当前来源" value={choice} disabled={busy||!desktopAvailable} onChange={e=>{setChoice(e.target.value);setConfirmed(false);}}><option value="">请选择</option>{data.accounts.map(a=><option key={a.id} value={a.id}>{a.label} / {a.id}</option>)}</select></label>
      <label className="permission-choice"><input type="checkbox" checked={confirmed} disabled={busy||!choice} onChange={e=>setConfirmed(e.target.checked)}/>确认此来源；旧 API 令牌保持撤销</label>
      <button className="button primary" disabled={busy||!desktopAvailable||!choice||!confirmed}>确定当前来源</button>
    </form> : current ? <>
      <ul className="account-list"><li><div className="account-avatar">{current.label.slice(0,1)}</div><div className="account-content"><strong>{current.label}</strong><details><summary>来源信息</summary><code>{current.id}</code></details></div><span className="state-tag">{accountStates[current.state]??current.state}</span><button className="button" disabled={busy||!desktopAvailable} aria-label={`配置 ${current.label} 来源`} onClick={()=>setConfigure(v=>!v)}>配置</button><button className="icon-button" aria-label={`移除 ${current.label} 登记`} disabled={busy||!desktopAvailable} onClick={()=>setRemoving(current.id)}><Trash2 size={17}/></button></li></ul>
      <p className="field">本机配置已保存 / 无需导入历史</p>
      {configure&&<ConnectionSetup key={current.id} accountId={current.id} close={()=>setConfigure(false)} {...workbench}/>}
      <NativeConnection key={`native/${current.id}`} accountId={current.id} {...workbench}/>
    </> : <>
      <div className="empty-state"><Cable size={36} strokeWidth={1.3}/><h3>尚未配置微信</h3><p>先配置当前客户端，无需导入历史。</p></div>
      <form onSubmit={register} className="register-form"><label htmlFor="account-label">本机标签</label><div className="input-action"><input id="account-label" maxLength={80} placeholder="我的微信" value={label} onChange={e=>setLabel(e.target.value)} disabled={!desktopAvailable||busy}/><button className="button primary" disabled={!desktopAvailable||busy||!label.trim()}><Plus size={16}/>配置当前微信</button></div><p className="field form-hint">仅保存本机配置，不自动读取微信。</p></form>
    </>}
    {archives.length>0&&<details className="permission-details"><summary>保留的旧资料 / {archives.length}</summary><ul>{archives.map(a=><li key={a.id}><strong>{a.label}</strong><code>{a.id}</code></li>)}</ul><p className="field">在会话页按归档范围浏览和导出，不提供微信连接切换。</p></details>}
    {confirmation.value&&<section className="confirmation feedback-surface" data-motion={confirmation.animated} data-exiting={confirmation.exiting} inert={confirmation.exiting} aria-hidden={confirmation.exiting||undefined} role={confirmation.exiting?undefined:'alertdialog'} aria-labelledby="remove-title" onTransitionEnd={e=>{if(e.target===e.currentTarget&&e.propertyName==='opacity'&&confirmation.exiting)confirmation.finish();}}><h3 id="remove-title">移除本机配置？</h3><p>会删除此范围在本软件内的密钥、导入记录、音频和任务，并撤销令牌。不会修改微信原数据。</p><div className="actions"><button className="button" onClick={()=>setRemoving(null)}>取消</button><button className="button danger" disabled={busy||!desktopAvailable} onClick={()=>void act(async()=>{await command('remove_account',{id:removing});setRemoving(null);setConfigure(false);})}>确认移除</button></div></section>}
  </>;
}
