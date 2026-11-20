// Primary: one next action; secondary: detected readiness; tertiary: paths and permissions.
// Hierarchy/spacing §2.1/§3.6: one stage, 16/24/32/48 gaps, existing paper/ink/signal
// tokens and short mechanical motion. No new theme, dependency, or password requirement.
import { useTokenValidity } from '../components/TokenValidity';
import {useEffect,useRef,useState} from 'react';
import {ArrowLeft,ArrowRight,Check,Copy,LoaderCircle,RefreshCw} from 'lucide-react';
import {command,type Account,type Bootstrap,type ClientInstallation,type DiscoveryReport,type IssuedToken} from '../lib/desktop';
import type {Workbench} from '../hooks/useWorkbench';
import type {NativeStatus} from './NativeConnection';
import {botConnection,botScopes,readScopes} from '../lib/bot-connection';
import {readFirstRun,saveFirstRun,type FirstRunStep} from '../lib/first-run';
import {waitForOnlineRead} from '../lib/busy-read';
import {rememberApi,rememberConnection} from '../lib/connection-resume';
import {HelpTip} from '../components/HelpTip';
import appIcon from '../../src-tauri/icons/128x128.png';
import './first-run.css';

const steps:FirstRunStep[]=['wechat','prepare','bot','done'];
const names=['找到微信','准备连接','接入 Bot','开始使用'];
export function FirstRun({workbench,finish}:{workbench:Workbench;finish:(page:'history'|'connect'|'api',complete:boolean)=>void}) {
  const [step,setStep]=useState<FirstRunStep>(()=>{const saved=readFirstRun();return saved==='complete'?'wechat':saved;});
  const [clients,setClients]=useState<ClientInstallation[]>([]),[choice,setChoice]=useState(''),[detected,setDetected]=useState(false);
  const [processes,setProcesses]=useState<number[]>([]),[pid,setPid]=useState('');
  const [busy,setBusy]=useState(false),[activity,setActivity]=useState(''),[error,setError]=useState('');
  const [canceling,setCanceling]=useState(false);
  const [connected,setConnected]=useState(false),[prepared,setPrepared]=useState(false),[apiReady,setApiReady]=useState(false);
  const [secret,setSecret]=useState(''),[address,setAddress]=useState(''),[copied,setCopied]=useState(false),[saved,setSaved]=useState(false);
  const [access,setAccess]=useState<'bot'|'read'>('bot'),[port,setPort]=useState(5031),[manual,setManual]=useState('');
  const { validity, fields } = useTokenValidity(access === 'bot' ? 90 : 7, busy, 'first-run-field');
  const gate=useRef(false),mounted=useRef(true),request=useRef<string|null>(null),initial=useRef(false),heading=useRef<HTMLHeadingElement>(null);
  const canceled=useRef(false);
  const client=clients.find(c=>c.id===choice),index=steps.indexOf(step);
  const accountId=workbench.data.current_connection.account_id??'';
  const eligible=client&&['supported','native_check_required'].includes(client.compatibility);
  useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;const id=request.current;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});};},[]);
  useEffect(()=>{heading.current?.focus();},[step]);
  function next(value:FirstRunStep){saveFirstRun(value);setStep(value);setError('');}
  async function task(action:()=>Promise<void>){
    if(gate.current)return;gate.current=true;setBusy(true);setError('');
    try{await action();}catch(e){if(mounted.current)setError(e instanceof Error?e.message:String(e));}
    finally{gate.current=false;if(mounted.current){setBusy(false);setActivity('');}await workbench.refresh().catch(()=>{});}
  }
  async function detect(){await task(async()=>{
    setActivity('正在查找本机微信');
    const report=await command<DiscoveryReport>('discover_clients');
    if(!mounted.current)return;
    setClients(report.installations);setDetected(true);
    const running=report.installations.filter(c=>c.process_ids.length>0);
    const selected=running.length===1?running[0]:report.installations.length===1?report.installations[0]:null;
    setChoice(selected?.id??'');setProcesses([]);setPid('');
  });}
  useEffect(()=>{if(initial.current)return;initial.current=true;void detect();},[]);
  async function connect(){await task(async()=>{
    if(!client)throw Error('请选择本机微信。');
    setActivity('正在保存连接并核验登录身份');
    let data=await command<Bootstrap>('bootstrap');
    if(data.current_connection.selection_required)throw Error('存在多个旧资料来源，请进入手工设置确认当前来源。');
    let id=data.current_connection.account_id;
    if(!id){const account=await command<Account>('register_account',{label:'我的微信'});id=account.id;}
    const binding=data.bindings.find(b=>b.account_id===id);
    if(binding&&binding.installation.executable_path.toLowerCase()!==client.executable_path.toLowerCase())throw Error('已绑定其他客户端，请进入手工设置核对来源。');
    if(!binding)await command('bind_client',{accountId:id,path:client.executable_path});
    const ids=await command<number[]>('native_processes',{accountId:id});
    setProcesses(ids);
    const selected=ids.length===1?ids[0]:Number(pid);
    if(!ids.includes(selected)){setPid('');throw Error(ids.length?'检测到多个微信进程，请选择后重试。':'请打开并登录微信，再点击重试。');}
    data=await command<Bootstrap>('bootstrap');
    const status=await command<NativeStatus>('native_status');
    const result=status.connected&&status.account_id===id&&status.process_id===selected?status:await command<NativeStatus>('native_connect',{accountId:id,revision:data.current_connection.revision,processId:selected});
    if(!result.connected)throw Error(result.error_code??'微信尚未连接。');
    rememberConnection(id,true);
    setConnected(true);next('prepare');
    await prepare(id);
  });}
  async function prepare(id=accountId){
    const status=await command<NativeStatus>('native_status');
    if(!status.connected||status.account_id!==id){setConnected(false);throw Error('微信连接已断开，请返回上一步重新连接。');}
    setConnected(true);setActivity('正在核验账号并准备聊天资料');
    const requestId=crypto.randomUUID();request.current=requestId;canceled.current=false;setCanceling(false);
    try{
      const result=await waitForOnlineRead(()=>command('native_live_auto_prepare',{accountId:id,requestId,single:false}),()=>!mounted.current||canceled.current,30000);
      if(result&&mounted.current&&!canceled.current){setPrepared(true);next('bot');}
    }finally{request.current=null;setCanceling(false);}
  }
  async function cancelPrepare(){canceled.current=true;setCanceling(true);setActivity('正在取消资料准备');const id=request.current;if(id)await command('cancel_browse',{requestId:id}).catch(e=>setError(e instanceof Error?e.message:String(e)));}
  async function authorize(){
    if (!validity) return;
    await task(async()=>{
    const data=await command<Bootstrap>('bootstrap'),id=data.current_connection.account_id;
    const native=await command<NativeStatus>('native_status');
    if(!id||!native.connected||native.account_id!==id)throw Error('请先返回连接步骤，确认已登录微信身份。');
    setActivity('正在启动本机 API');
    if(!data.status.api_enabled)await command('set_api_enabled',{enabled:true,port});
    const active=await command<Bootstrap>('bootstrap');
    if(!active.status.api_enabled||!active.api_address)throw Error('API 尚未启动，请重试。');
    setAddress(active.api_address);setApiReady(true);
    rememberApi(id,Number(new URL(active.api_address).port));
    if(!secret){
      setActivity('正在生成连接配置');
      const issued=await command<IssuedToken>('issue_token',{label:'我的 Bot',accountIds:[id],scopes:access==='bot'?botScopes:readScopes,validity});
      setSecret(issued.secret);setCopied(false);setSaved(false);
    }
  });}
  async function copy(){await task(async()=>{
    if(!secret||!address||!accountId)return;
    await navigator.clipboard.writeText(JSON.stringify(botConnection(address,accountId,secret),null,2));setCopied(true);
  });}
  function complete(){setSecret('');saveFirstRun('complete');finish('history',true);}
  const titles={wechat:'先登录微信，其余交给 WeRC',prepare:'准备聊天资料',bot:'把微信接到你的 Bot',done:'设置已就绪'};
  return <div className="ef first-run" data-step={step} aria-label="首次设置">
    <header className="first-run-header"><div className="brand"><img className="brand-mark" src={appIcon} alt=""/><span><strong>WeRC</strong><small>FIRST SETUP</small></span></div><button className="button" disabled={busy} onClick={()=>{setSecret('');finish('connect',false);}}>稍后继续</button></header>
    <div className="first-run-layout"><aside className="first-run-rail"><p className="eyebrow">// INITIALIZE</p><h2>一次设置<br/>即刻开始</h2><ol>{names.map((name,i)=><li key={name} aria-current={index===i?'step':undefined} data-complete={index>i}><span>{index>i?<Check size={16}/>:String(i+1).padStart(2,'0')}</span>{name}</li>)}</ol><p className="field">本机连接 · 应用密码默认关闭</p></aside>
    <main className="first-run-main"><div className="first-run-stage" key={step}><p className="eyebrow">// STEP {String(index+1).padStart(2,'0')} / 04</p><h1 ref={heading} tabIndex={-1}>{titles[step]}</h1>
      {error&&<p className="error-notice" role="alert">{error}</p>}
      {busy&&<p className="first-run-activity" role="status"><LoaderCircle className="spin" size={18}/>{activity||'正在处理'}</p>}
      {step==='wechat'&&<>
        <p className="first-run-lead">打开电脑微信并登录，确认后自动连接。</p>
        {client?<div className="first-run-detection"><span className="field">检测到微信</span><strong>{client.product_name} <span>{client.version}</span></strong><span className="state-tag">{client.process_ids.length?'微信正在运行':'等待打开微信'}</span></div>:detected&&<p className="first-run-detection">{clients.length?'发现多个安装，请选择当前登录的微信。':'未发现微信，请先安装或在高级设置中选择。'}</p>}
        {clients.length>1&&<label className="first-run-field">当前微信<select aria-label="设置微信版本" value={choice} disabled={busy} onChange={e=>{setChoice(e.target.value);setProcesses([]);setPid('');}}><option value="">请选择</option>{clients.map(c=><option key={c.id} value={c.id}>{c.version} / {c.executable_path}</option>)}</select></label>}
        {client&&!eligible&&<p className="error-notice" role="status">此版本的原生收发尚未适配。请手动安装分发包 vendor-installers/wechat 中的匹配版本，再重新检测。<HelpTip label="版本适配详情">{client.reason}。检测到版本不代表可用；连接时还会验证实际加载的 DLL 指纹。</HelpTip></p>}
        {processes.length>1&&<label className="first-run-field">当前微信进程<select aria-label="设置微信进程" value={pid} disabled={busy} onChange={e=>setPid(e.target.value)}><option value="">请选择</option>{processes.map(p=><option key={p} value={p}>{p}</option>)}</select></label>}
        <div className="first-run-actions"><button className="button primary" disabled={busy||!eligible||!client?.process_ids.length} onClick={()=>void connect()}>已登录，自动连接<ArrowRight size={16}/></button><button className="button" disabled={busy} onClick={()=>void detect()}><RefreshCw size={16}/>重新检测</button><HelpTip label="自动连接会做什么">确认后自动登记本机标签、绑定客户端、核验登录身份，并只读准备聊天资料和保护密钥。无需导入全部历史；不会要求退出微信。首次打开只自动检测安装元数据。</HelpTip></div>
        <details><summary>高级设置</summary><label className="first-run-field">微信 EXE 路径<input aria-label="设置微信路径" value={manual} disabled={busy} onChange={e=>setManual(e.target.value)}/></label><button className="button" disabled={busy||!manual.trim()} onClick={()=>void task(async()=>{const item=await command<ClientInstallation>('inspect_client',{path:manual.trim()});setClients([item]);setChoice(item.id);setDetected(true);setProcesses([]);setPid('');})}>检查此客户端</button><button className="button" disabled={busy} onClick={()=>finish('connect',false)}>进入手工设置</button></details>
      </>}
      {step==='prepare'&&<><p className="first-run-lead">自动选择当前账号和数据来源，无需填写路径。</p><div className="first-run-detection"><strong>{connected?'微信通道已连接':'将检查当前连接'}</strong><span className="field">首次准备可能需要一些时间</span></div>{busy&&<button className="button" disabled={canceling} onClick={()=>void cancelPrepare()}>{canceling?'正在取消…':'取消资料准备'}</button>}<div className="first-run-actions"><button className="button primary" disabled={busy||!accountId} onClick={()=>void task(()=>prepare())}>重试准备<ArrowRight size={16}/></button><button className="button" disabled={busy} onClick={()=>next('wechat')}><ArrowLeft size={16}/>返回连接</button><button className="button" disabled={busy} onClick={()=>next('bot')}>暂不读取历史</button></div><HelpTip label="资料准备遇到问题">后台补读占用来源时自动等待，最多约 30 秒。来源不唯一或未找到时，可在主界面“会话 → 微信会话”的高级区域选择来源。暂不读取历史仍可使用已连接的实时通道。</HelpTip></>}
      {step==='bot'&&<><p className="first-run-lead">需要自动收发时，授权并复制配置到 Bot 客户端。</p>
        {!secret?<><div className="first-run-detection"><strong>本机 API + 一份连接配置</strong><span className="field">自动启动服务、生成令牌和填写账号</span></div><details><summary>权限、有效期与端口</summary><label className="first-run-field">用途<select aria-label="引导授权用途" value={access} disabled={busy} onChange={e=>setAccess(e.target.value as typeof access)}><option value="bot">Bot 自动收发</option><option value="read">只读资料</option></select></label>{fields}<label className="first-run-field">本机端口<input type="number" aria-label="引导API端口" min={1024} max={65535} value={port} disabled={busy} onChange={e=>setPort(Number(e.target.value))}/></label></details><div className="first-run-actions"><button className="button primary" disabled={busy||!accountId||!validity||!Number.isInteger(port)||port<1024||port>65535} onClick={()=>void authorize()}>启动并授权{access==='bot'?' Bot 收发':'只读访问'}<ArrowRight size={16}/></button><button className="button" disabled={busy} onClick={()=>next('done')}>暂不接 Bot</button><button className="button" disabled={busy} onClick={()=>next('wechat')}>返回连接</button><HelpTip label="授权说明">Bot 收发允许客户端读取消息并直接发送六类消息，无需逐条批准。只读用途不能发送。自动回复、名单和免打扰由 Bot 配置。可在 Bot 接入页撤销令牌；应用密码仍默认关闭。</HelpTip></div>{workbench.data.tokens.some(t=>!t.revoked)&&<p className="field">已有令牌保留；需要新配置时再授权。令牌密文不会恢复显示。</p>}</>:<><div className="first-run-detection"><strong>连接配置已生成</strong><code>{address}</code><input aria-label="引导连接令牌" type="password" readOnly value={secret}/></div><div className="first-run-actions"><button className="button" disabled={busy} onClick={()=>void copy()}><Copy size={16}/>{copied?'已复制连接配置':'复制连接配置'}</button></div><label className="permission-choice"><input type="checkbox" checked={saved} onChange={e=>setSaved(e.target.checked)}/>我已将配置粘贴到 Bot，或自行保存</label><div className="first-run-actions"><button className="button primary" disabled={busy||!saved} onClick={()=>{setSecret('');next('done');}}>配置已保存，继续<ArrowRight size={16}/></button></div><HelpTip label="令牌保存说明">配置包含私密令牌，仅在本次页面显示。刷新或离开后需在 Bot 接入页重新签发。复制成功不代表 Bot 已成功连接。</HelpTip></>}
      </>}
      {step==='done'&&<><p className="first-run-lead">日常从会话开始，其他功能按需打开。<HelpTip label="下次启动说明">下次启动会尝试恢复本次确认的微信连接和已启用 API；需微信保持登录。手动断开或停止 API 后会记住停用选择。不会重新签发令牌或重发旧任务。</HelpTip></p><ul className="first-run-results"><li><Check size={18}/><span>微信连接<strong>{connected?'本次已核验':'进入后可查看连接状态'}</strong></span></li><li><Check size={18}/><span>聊天资料<strong>{prepared?'本次已准备':'按需打开微信会话'}</strong></span></li><li><Check size={18}/><span>Bot 接入<strong>{apiReady?'服务已启动，等待客户端连接':'稍后在 Bot 接入中设置'}</strong></span></li></ul><div className="first-run-actions"><button className="button primary" onClick={complete}>进入会话<ArrowRight size={16}/></button><button className="button" onClick={()=>next('bot')}>返回 Bot 设置</button></div></>}
    </div></main></div><footer className="first-run-footer"><span>WeRC / LOCAL WORKSPACE</span><span>设置 → 重新运行首次设置</span></footer>
  </div>;
}
