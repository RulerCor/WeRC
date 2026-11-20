import { useState, type FormEvent } from 'react';
import { Search, Link2, X } from 'lucide-react';
import { command, desktopAvailable, type ClientInstallation, type DiscoveryReport } from '../lib/desktop';
import type { Workbench } from '../hooks/useWorkbench';
import { KeyAcquisition } from './KeyAcquisition';
import {DatabaseKeys} from '../components/DatabaseKeys';
import {ImageKeys} from '../components/ImageKeys';

export function ConnectionSetup({ accountId, close, data, busy, act }: Workbench & { accountId: string; close: () => void }) {
  const [path, setPath] = useState('');
  const [report, setReport] = useState<DiscoveryReport | null>(null);
  const [inspected, setInspected] = useState<ClientInstallation | null>(null);
  const [key, setKey] = useState('');
  const [disconnecting, setDisconnecting] = useState(false);
  const binding = data.bindings.find(b => b.account_id === accountId);
  const keyStatus = data.key_statuses.find(k => k.account_id === accountId);
  const installation = inspected ?? binding?.installation;
  const disabled = busy || !desktopAvailable;
  const inspect = (event: FormEvent) => {
    event.preventDefault();
    return act(async () => { setInspected(await command<ClientInstallation>('inspect_client', { path })); });
  };
  const protect = (event: FormEvent) => {
    event.preventDefault();
    const keyHex = key;
    setKey('');
    return act(() => command('import_key', { accountId, keyHex }));
  };
  return <section className="connection-setup" aria-label="客户端来源配置">
    <div className="section-title"><h2>客户端来源</h2><button className="icon-button" aria-label="关闭来源配置" onClick={close}><X size={16}/></button></div>
    <div className="discovery-controls"><button className="button" disabled={disabled} onClick={() => act(async () => { setReport(await command<DiscoveryReport>('discover_clients')); })}><Search size={16}/>检测安装</button><p className="field">只检查安装与进程元数据，不读取聊天。</p></div>
    {report && <div className="discovery-result">
      {report.installations.length ? <label>检测结果<select aria-label="检测到的客户端" defaultValue="" onChange={e => { const item = report.installations.find(i => i.id === e.target.value); if (item) { setPath(item.executable_path); setInspected(item); } }}><option value="">选择安装</option>{report.installations.map(item => <option key={item.id} value={item.id}>{item.version} / {item.architecture} / {item.executable_path}</option>)}</select></label> : <p className="field">未发现可识别安装，可手动填写路径。</p>}
      {report.warnings.length > 0 && <details><summary>检测提示 / {report.warnings.length}</summary>{report.warnings.map((warning, i) => <p className="field" key={i}>{warning}</p>)}</details>}
    </div>}
    <form onSubmit={inspect} className="source-form"><label htmlFor="client-path">客户端 EXE 路径</label><div className="input-action"><input id="client-path" value={path} onChange={e => { setPath(e.target.value); setInspected(null); }} placeholder="Weixin.exe 或 WeChat.exe 的完整路径" disabled={disabled}/><button className="button" disabled={disabled || !path.trim()}>检查文件</button></div></form>
    {installation && <div className="installation-result">
      <div className="source-summary"><strong>{installation.product_name || '客户端'}</strong><span className="state-tag">{installation.version} / {installation.architecture}</span></div>
      <code>{installation.executable_path}</code>
      <p className="field">{installation.compatibility==='adapter_pending'?'此版本的原生收发适配待完成':installation.compatibility==='native_check_required'||installation.compatibility==='supported'?'已找到适配档案，连接时自动核验':'安装信息已识别，尚无可用原生适配'}</p>
      <details><summary>校验详情</summary><p className="field">{installation.reason}</p><code>SHA-256 / {installation.sha256}</code>{binding && <p className="field">绑定检查时间 / {new Date(binding.checked_at).toLocaleString('zh-CN')}</p>}</details>
      {inspected && <button className="button primary" disabled={disabled} onClick={() => act(async () => { await command('bind_client', { accountId, path: inspected.executable_path }); setInspected(null); setKey(''); })}><Link2 size={16}/>{binding ? '更新绑定' : '绑定来源'}</button>}
    </div>}
    {binding && <>
      <div className="key-heading"><h3>数据库密钥</h3><span className="state-tag">{keyStatus?.verified ? `${keyStatus.verified_databases} 库首验通过` : keyStatus?.protected ? '已保护 / 未验真' : '未保存'}</span></div>
      <form onSubmit={protect}><label htmlFor="database-key" className="field">64 位十六进制 / 仅本机保护</label><div className="input-action"><input id="database-key" aria-label="数据库密钥" type="password" autoComplete="off" spellCheck={false} maxLength={64} value={key} onChange={e => setKey(e.target.value)} disabled={disabled}/><button className="button" disabled={disabled || !/^[0-9a-fA-F]{64}$/.test(key)}>保护密钥</button></div></form>
      <p className="field form-hint">更换来源会清除旧密钥。保存不代表数据库验证通过。</p>
      {keyStatus?.verification_error && <p role="status" className="field">受保护内容无法恢复或验证，可清除后重新获取。</p>}
      <KeyAcquisition key={`${accountId}/${binding.installation.sha256}`} accountId={accountId} installation={binding.installation} busy={busy} act={act}/>
      {keyStatus?.protected&&<DatabaseKeys key={accountId} accountId={accountId} disabled={disabled}/>}
      <ImageKeys key={`image-${accountId}`} accountId={accountId} disabled={disabled}/>
      <div className="source-actions">{keyStatus?.protected && <button className="button" disabled={disabled} onClick={() => act(() => command('remove_key', { accountId }))}>清除密钥</button>}<button className="button" disabled={disabled} onClick={() => setDisconnecting(true)}>解除绑定</button></div>
      {disconnecting && <div className="confirmation"><h3>解除来源绑定？</h3><p>同时清除该登记的受保护密钥，不修改微信文件。</p><div className="actions"><button className="button" onClick={() => setDisconnecting(false)}>取消</button><button className="button" disabled={disabled} onClick={() => act(async () => { await command('unbind_client', { accountId }); setInspected(null); setKey(''); setDisconnecting(false); })}>确认解除</button></div></div>}
    </>}
  </section>;
}
