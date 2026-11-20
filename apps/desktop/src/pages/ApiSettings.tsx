import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Check, Copy, KeyRound, Power, X } from 'lucide-react';
import { command, desktopAvailable, type IssuedToken } from '../lib/desktop';
import type { Workbench } from '../hooks/useWorkbench';
import { useFeedbackMotion } from '../hooks/useFeedbackMotion';
import {HelpTip} from '../components/HelpTip';
import {rememberApi} from '../lib/connection-resume';
import {readScopes, botScopes, botConnection, mcpConnection, mcpStdioConnection} from '../lib/bot-connection';
import { permanentTokenExpiry, useTokenValidity } from '../components/TokenValidity';
const draftChoices = [['messages:send','文字'],['messages:send_voice','语音'],['messages:send_image','图片'],['messages:send_file','文件'],['messages:send_video','视频'],['messages:send_emoticon','表情']] as const;

export function ApiSettings({ data, busy, act, setError }: Workbench) {
  const [port, setPort] = useState(() => data.api_address ? Number(new URL(data.api_address).port) : 5031);
  useEffect(() => {
    if (data.api_address) setPort(Number(new URL(data.api_address).port));
  }, [data.api_address]);
  const [label, setLabel] = useState('我的 Bot');
  const [preset, setPreset] = useState<'bot'|'read'|'custom'>('bot');
  const { validity, fields } = useTokenValidity(preset === 'bot' ? 90 : 7, !desktopAvailable || busy);
  const [issuedPreset, setIssuedPreset] = useState<'bot'|'read'|'custom'>('bot');
  const [audioAccess, setAudioAccess] = useState(false);
  const [exportAccess, setExportAccess] = useState(false);
  const [settingsAccess,setSettingsAccess]=useState(false);
  const [draftScopes,setDraftScopes]=useState<string[]>([]);
  const [groupNamesAccess,setGroupNamesAccess]=useState(false);
  const baseScopes = preset === 'bot' ? [...botScopes, ...(settingsAccess?['wechat:settings:write']:[])] : preset === 'read' ? readScopes : [...new Set([...readScopes, ...(audioAccess ? ['media:write', 'operations:create'] : []), ...(exportAccess ? ['exports:create'] : []), ...draftScopes, ...(draftScopes.length?['operations:create']:[])])];
  const [friendsAccess,setFriendsAccess]=useState(false);
  const [directoryAccess,setDirectoryAccess]=useState(false);
  const scopes = preset === 'bot' ? baseScopes : [...baseScopes, ...(groupNamesAccess?['group-names:read']:[]), ...(friendsAccess?['friends:read']:[]), ...(directoryAccess?['directory:read']:[])];
  const accountId = data.current_connection.account_id ?? '';
  const [secret, setSecret] = useState('');
  const [copied, setCopied] = useState(false);
  const [copyMotion, setCopyMotion] = useState(false);
  const generation=useRef(0);
  useEffect(() => { generation.current++;setSecret('');setCopied(false);setAudioAccess(false);setExportAccess(false);setSettingsAccess(false);setGroupNamesAccess(false);setFriendsAccess(false);setDirectoryAccess(false);setDraftScopes([]);return()=>{generation.current++;}; }, [accountId]);
  const canAnimate = useFeedbackMotion();
  async function createToken(event: FormEvent) {
    event.preventDefault();
    if (!validity) return;
    const ticket=generation.current;
    await act(async () => {
      const token = await command<IssuedToken>('issue_token', { label, accountIds: [accountId], scopes, validity });
      if(ticket===generation.current){setSecret(token.secret); setIssuedPreset(preset); setCopied(false);}
    });
  }
  async function copy() {
    const animate = canAnimate();
    setCopied(false);
    try { await navigator.clipboard.writeText(secret); setCopyMotion(animate); setCopied(true); }
    catch { setError('无法访问剪贴板，请手动复制。'); }
  }
  async function copyConfiguration(mcp: boolean, stdio = false) {
    if (!data.api_address || !secret) return;
    const ticket=generation.current;
    try {
      const location = stdio ? await command<{executable:string}>('runtime_location') : null;
      if(ticket!==generation.current)return;
      const executable = location?.executable.replace(/[^\\/]+$/, 'werc-mcp-stdio.exe');
      const config = stdio ? mcpStdioConnection(data.api_address, secret, issuedPreset !== 'bot', executable) : mcp ? mcpConnection(data.api_address, secret, issuedPreset !== 'bot') : botConnection(data.api_address, accountId, secret);
      await navigator.clipboard.writeText(JSON.stringify(config, null, 2));
      setCopied(true);
    } catch { setError('无法复制配置，请复制地址和令牌到客户端。'); }
  }
  return <>
    <div className="section-title"><h2>HTTP 服务</h2><span className={`state-tag ${data.status.api_enabled ? 'ready' : ''}`}>{data.status.api_enabled ? '运行中' : '已停止'}</span></div>
    <div className="server-controls">
      <label>端口<input aria-label="API监听端口" type="number" min={1024} max={65535} value={port} disabled={data.status.api_enabled || busy} onChange={e => setPort(Number(e.target.value))}/></label>
      <code>{data.api_address ?? `http://127.0.0.1:${port}`}</code>
      <button className={`button ${data.status.api_enabled ? '' : 'primary'}`} disabled={!desktopAvailable || busy || !Number.isInteger(port) || port < 1024 || port > 65535} onClick={() => act(async() => {await command('set_api_enabled', { enabled: !data.status.api_enabled, port });if(accountId)rememberApi(accountId,data.status.api_enabled?null:port);})}><Power size={16}/>{data.status.api_enabled ? '停止服务' : '启动服务'}</button>
    </div>
    <div className="section-title"><h2>连接 Bot 客户端 <HelpTip label="Bot连接说明">一次授权后，Bot可以直接请求文字、图片、语音、视频、文件和表情发送，不需要逐条回桌面批准。平台转发当前账号全部可观测消息；白黑名单、免打扰和自动回复在Bot客户端设置。此处保留鉴权、幂等和未知结果不重发。</HelpTip></h2><span className="field">可随时撤销</span></div>
    <label>连接用途<select aria-label="连接用途" value={preset} disabled={busy} onChange={e=>setPreset(e.target.value as typeof preset)}>
      <option value="bot">Bot 收发（推荐）</option><option value="read">只读资料</option><option value="custom">自选权限 / 仅草稿</option>
    </select></label>
    {preset==='bot'&&<label className="permission-choice"><input type="checkbox" aria-label="允许调整微信设置" checked={settingsAccess} disabled={busy||!desktopAvailable} onChange={e=>setSettingsAccess(e.target.checked)}/>允许调整微信设置 <HelpTip label="微信设置权限说明">仅开放当前微信版本已验证的后台原生设置。首次默认关闭；勾选仅影响随后签发的新令牌，不修改已有授权。支持项由 Agent 查询能力接口取得。</HelpTip></label>}
    {preset==='bot' ? <p className="field">Bot 令牌默认包含群名同步、好友列表和分类通讯录。</p> : <>
    <label className="permission-choice"><input type="checkbox" checked={groupNamesAccess} disabled={busy||!desktopAvailable} onChange={e=>setGroupNamesAccess(e.target.checked)}/>允许同步群名（仅群标识和名称）</label>
    <label className="permission-choice"><input type="checkbox" checked={friendsAccess} disabled={busy||!desktopAvailable} onChange={e=>setFriendsAccess(e.target.checked)}/>允许读取好友列表（仅好友标识、昵称和备注）</label>
    <label className="permission-choice"><input type="checkbox" checked={directoryAccess} disabled={busy||!desktopAvailable} onChange={e=>setDirectoryAccess(e.target.checked)}/>允许读取分类通讯录（包括非好友，仅标识、名称、备注和类别）</label>
    </>}
    <form className="token-form" onSubmit={createToken}>
      <label>名称<input value={label} maxLength={80} onChange={e => setLabel(e.target.value)} placeholder="调用方名称" disabled={!desktopAvailable || busy}/></label>
      <label>授权范围<input aria-label="令牌当前来源" readOnly value={data.accounts.find(a=>a.id===accountId)?.label??'请先配置当前微信'}/></label>
      {fields}
      <button className="button" disabled={!desktopAvailable || busy || !label.trim() || !accountId || !validity}><KeyRound size={16}/>{preset==='bot'?'授权 Bot 收发':'生成令牌'}</button>
    </form>
    {preset==='custom'&&<details className="permission-settings"><summary>额外权限（可选）</summary>
    <label className="permission-choice"><input type="checkbox" checked={audioAccess} disabled={busy || !desktopAvailable} onChange={e => setAudioAccess(e.target.checked)}/>允许上传、删除和处理本地媒体</label>
    <label className="permission-choice"><input type="checkbox" checked={exportAccess} disabled={busy || !desktopAvailable} onChange={e => setExportAccess(e.target.checked)}/>允许下载筛选后的消息导出，不含来源路径访问</label>
    <fieldset className="draft-permissions"><legend>允许创建消息草稿 <HelpTip label="API消息草稿权限说明">自选权限仅创建待批准草稿，需在桌面批准。需要自动收发请选“Bot收发”。上传新媒体还需本地媒体权限；可用目标与类型由原生连接报告。</HelpTip></legend>
      {draftChoices.map(([scope,label])=><label className="permission-choice" key={scope}><input type="checkbox" aria-label={`允许API创建${label}草稿`} checked={draftScopes.includes(scope)} disabled={busy||!desktopAvailable} onChange={e=>setDraftScopes(previous=>e.target.checked?[...previous,scope]:previous.filter(s=>s!==scope))}/>{label}</label>)}
    </fieldset></details>}
    <details className="permission-details"><summary>权限范围</summary><div className="scope-line">{scopes.map(scope => <code key={scope}>{scope}</code>)}</div></details>
    {secret && <div className="secret-box"><strong>仅显示一次</strong><div className="input-action">
      <input type="password" value={secret} readOnly aria-label="新签发令牌"/>
      <button className="button copy-feedback" onClick={copy}>{copied ? <Check className="copy-check" data-motion={copyMotion} size={15} aria-hidden="true"/> : <Copy size={15} aria-hidden="true"/>}<span aria-live="polite">{copied ? '已复制' : '复制'}</span></button>
      <button className="icon-button" aria-label="清除令牌显示" onClick={() => setSecret('')}><X size={16}/></button>
    </div><div className="actions"><button className="button" disabled={!data.api_address} onClick={()=>copyConfiguration(false)}>复制 Bot 连接配置</button><button className="button" disabled={!data.api_address} onClick={()=>copyConfiguration(true)}>复制 MCP 配置</button><button className="button" disabled={!data.api_address} onClick={()=>copyConfiguration(true,true)}>复制 STDIO 配置</button><HelpTip label="连接配置说明">配置含本次令牌，只粘贴到你选择的客户端。消息流和平台事件流分别保存游标；重连用Last-Event-ID续读。先启动HTTP服务才能复制可用地址。</HelpTip></div></div>}
    {data.tokens.length ? <ul className="token-list">{data.tokens.map(token => <li key={token.id}>
      <KeyRound size={17}/><div><strong>{token.label}</strong><p className="field">{token.revoked ? '已撤销' : token.expires_at === permanentTokenExpiry ? '永久有效' : new Date(token.expires_at).getTime() <= Date.now() ? '已过期' : `到期 ${new Date(token.expires_at).toLocaleString('zh-CN', { hour12: false })}`}</p></div>
      <button className="button" disabled={token.revoked || busy} onClick={() => act(() => command('revoke_token', { id: token.id }))}>撤销</button>
    </li>)}</ul> : <p className="empty-inline">暂无令牌</p>}
  </>;
}
