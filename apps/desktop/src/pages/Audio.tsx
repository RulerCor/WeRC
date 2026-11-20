import { useEffect, useRef, useState, type FormEvent } from 'react';
import { AudioLines, Upload } from 'lucide-react';
import { command, desktopAvailable } from '../lib/desktop';
import type { Workbench } from '../hooks/useWorkbench';
import { AudioPreview } from './AudioPreview';
import { Transcription } from './Transcription';
import { ModelManager } from './ModelManager';
const states: Record<string, string> = { queued: '排队中', preparing: '处理中', succeeded: '完成 / 未发送', failed: '失败', canceled: '已取消', unknown: '结果未知', submitting: '提交中' };

export function Audio({ data, busy, act, refresh, setError }: Workbench) {
  const [accountId, setAccountId] = useState(data.current_connection.account_id??'');
  const [file, setFile] = useState<File | null>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(()=>{setAccountId(current=>data.accounts.some(account=>account.id===current)?current:data.current_connection.account_id??'');},[data.current_connection.account_id,data.accounts]);
  const assets = data.audio_assets.filter(a => a.account_id === accountId);
  const operations = data.operations.filter(o => o.account_id === accountId);
  const processing = operations.some(o => o.state === 'queued' || o.state === 'preparing');
  useEffect(() => {
    if (!processing) return;
    const timer = window.setInterval(() => { refresh().catch(() => setError('无法刷新本地任务状态。')); }, 2000);
    return () => window.clearInterval(timer);
  }, [processing, refresh, setError]);
  async function upload(event: FormEvent) {
    event.preventDefault();
    if (!file || file.size === 0 || file.size > 8 * 1024 * 1024) { setError('请选择非空且不超过 8 MiB 的 PCM16 WAV 文件。'); return; }
    await act(async () => {
      const bytes = new Uint8Array(await file.arrayBuffer());
      try { await command('upload_audio', { accountId, bytes: Array.from(bytes) }); }
      finally { bytes.fill(0); }
      setFile(null);
      if (input.current) input.current.value = '';
    });
  }
  return <>
    <div className="section-title"><h2>本地音频</h2><span className="state-tag">导入和预听不会发送</span></div>
    <form className="audio-form" onSubmit={upload}>
      <label>账号<select aria-label="音频账号" value={accountId} disabled={busy || !desktopAvailable} onChange={e => { setAccountId(e.target.value); setFile(null); if (input.current) input.current.value = ''; }}><option value="">选择账号</option>{data.accounts.map(a => <option key={a.id} value={a.id}>{a.label}</option>)}</select></label>
      <label>音频文件<input ref={input} aria-label="音频文件" type="file" accept=".wav,audio/wav,audio/x-wav" disabled={busy || !desktopAvailable || !accountId} onChange={e => setFile(e.target.files?.[0] ?? null)}/></label>
      <button className="button primary" disabled={busy || !desktopAvailable || !accountId || !file}><Upload size={16}/>导入音频</button>
    </form>
    <p className="field">PCM16 WAV / 最多 8 MiB / 24 小时后清理</p>
    <ModelManager/>
    {assets.length ? <ul className="audio-list">{assets.map(asset => <li key={asset.id}>
      <AudioLines size={20}/><div className="audio-content"><strong>{(asset.audio.duration_ms / 1000).toFixed(2)} 秒 / {asset.audio.sample_rate} Hz / {asset.audio.channels} 声道</strong><code>{asset.id}</code><p className="field">{(asset.size / 1024).toFixed(1)} KiB / 加密保存 / 未发送</p></div>
      <AudioPreview key={`${accountId}/${asset.id}`} accountId={accountId} assetId={asset.id} disabled={busy} onError={setError}/>
      <Transcription accountId={accountId} source={{kind:'uploaded',asset_id:asset.id}} disabled={busy}/>
      <div className="audio-actions"><button className="button" disabled={busy} onClick={() => act(() => command('inspect_audio_task', { accountId, assetId: asset.id, key: crypto.randomUUID() }))}>本地分析</button><button className="button" disabled={busy} onClick={() => act(() => command('delete_audio', { accountId, id: asset.id }))}>删除音频</button></div>
    </li>)}</ul> : <p className="empty-inline">{accountId ? '导入的音频将在这里显示。' : '选择账号后管理音频。'}</p>}
    <div className="section-title"><h2>处理任务</h2><span className="field">{operations.length} 项</span></div>
    {operations.length ? <ul className="operation-list">{operations.map(op => <li key={op.id}><div><strong>音频分析</strong><code>{op.id}</code>{op.error_code && <p className="field">{op.error_code}</p>}{op.result && <p className="field">峰值 {op.result.audio.peak.toFixed(4)} / 均方根 {op.result.audio.rms.toFixed(4)} / 仅本地</p>}</div><span className="state-tag">{states[op.state] ?? op.state}</span>{['queued', 'preparing'].includes(op.state) && <button className="button" disabled={busy} onClick={() => act(() => command('cancel_operation', { accountId, id: op.id }))}>取消</button>}</li>)}</ul> : <p className="empty-inline">本地分析任务将在这里显示，不会发送消息。</p>}
  </>;
}
