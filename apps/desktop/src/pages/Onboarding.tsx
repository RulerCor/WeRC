import { useEffect, useRef, useState } from 'react';
import { ArrowRight, Check, Pause, Play, RotateCcw, X } from 'lucide-react';
import { command, desktopAvailable, type DiscoveryReport } from '../lib/desktop';
import { MAX_SETUP_DATABASES, processCandidates, recommendSetup, selectAllDatabases, type SetupDetection } from '../lib/setup-plan';
import { setupProgress } from '../lib/setup-progress';
import type { OnboardingController, SetupItem } from '../hooks/useOnboarding';

const phases=['检测来源','获取密钥','等待退出','导入记录','查看结果'];
function imported(item:SetupItem) {
  return Boolean(item.source)&&!item.skipped&&(!item.outcome||['imported','already_imported'].includes(item.outcome));
}
function resultText(item:SetupItem) {
  if(imported(item))return `${item.outcome==='already_imported'?'已导入（已有记录）':'已导入'} / ${item.source!.message_count} 条消息 / ${item.source!.contact_count} 个联系人 / ${item.source!.session_count} 个会话`;
  const reason=item.outcome==='unsupported'?'未适配':item.outcome==='key_deferred'?'未取得此库密钥 / 暂缓处理':item.outcome==='resource_deferred'?'超预算 / 暂缓处理':item.outcome==='failed'?'失败':item.skipped?'未导入 / 已跳过':item.error?'失败':item.outcome==='imported'||item.outcome==='already_imported'?'未确认导入结果':item.key_ready?'首验通过 / 待导入':'待取钥';
  return item.error?`${reason} / ${item.error}`:reason;
}
export function Onboarding({flow,navigate}:{flow:OnboardingController;navigate:(page:'history'|'api')=>void}) {
  const [report,setReport]=useState<SetupDetection|null>(null),[installation,setInstallation]=useState(''),[source,setSource]=useState(''),[pid,setPid]=useState('');
  const [label,setLabel]=useState('我的微信');
  const [detecting,setDetecting]=useState(false),[error,setError]=useState(''),[root,setRoot]=useState(''),[manual,setManual]=useState('');
  const gate=useRef(false),title=useRef<HTMLHeadingElement>(null);
  const run=flow.status?.run,client=report?.clients.installations.find(i=>i.id===installation),candidate=report?.catalog.sources.find(s=>s.id===source);
  const selected=selectAllDatabases(candidate),pids=processCandidates(client,report?.main_process_ids??[]);
  const working=flow.busy||Boolean(flow.status?.working),disabled=working||detecting||!desktopAvailable;
  const finished=Boolean(run&&['complete','partial'].includes(run.phase));
  const phase=run?finished?4:run.phase==='keys'?1:run.phase==='importing'?3:run.phase==='waiting'?2:run.items.some(i=>!i.key_ready&&!i.skipped)?1:3:0;
  useEffect(()=>{title.current?.focus();},[phase]);
  function chooseClient(id:string){setInstallation(id);const item=report?.clients.installations.find(i=>i.id===id);setPid(String(processCandidates(item,report?.main_process_ids??[])[0]??''));}
  function invalidate(){setReport(null);setInstallation('');setSource('');setPid('');}
  async function detect(){
    if(gate.current||disabled)return;gate.current=true;setDetecting(true);setError('');
    try {
      const next=await command<SetupDetection>('setup_discover',{confirmed:true,fixture:null,root:root.trim()||null});
      if(manual.trim()){const inspected=await command<DiscoveryReport['installations'][number]>('inspect_client',{path:manual.trim()});next.clients.installations=[inspected];}
      const recommendation=recommendSetup(next);
      setReport(next);setInstallation(recommendation.installation);setSource(recommendation.source);setPid(recommendation.pid);
    }catch(e){setError(e instanceof Error?e.message:String(e));}finally{gate.current=false;setDetecting(false);}
  }
  const valid=client&&candidate&&pids.includes(Number(pid))&&selected.length>0&&selected.length<=MAX_SETUP_DATABASES&&label.trim();
  const done=run?.items.filter(imported).length??0,keys=run?.items.filter(i=>i.key_ready).length??0;
  const unsupported=run?.items.filter(i=>i.outcome==='unsupported').length??0,deferred=run?.items.filter(i=>i.outcome==='resource_deferred').length??0;
  const missingKeys=run?.items.filter(i=>i.outcome==='key_deferred').length??0;
  const failed=run?.items.filter(i=>!imported(i)&&(i.outcome==='failed'||(!i.outcome&&Boolean(i.error)))).length??0;
  const headline=finished?(done===0?'配置结束，尚无记录导入':done===run!.items.length?'离线记录已导入':'部分记录已导入'):working&&!flow.status?.active?'正在结束当前步骤':run?.phase==='failed'?'此步骤需要处理':!flow.status?.active?'引导已暂停':phase===1?'正在获取并保护密钥':phase===2?'请从微信菜单正常退出':'正在导入离线记录';
  const progress=run?setupProgress(run,phase===1):null;
  const running=!finished&&Boolean(working||flow.status?.active),handling=running&&phase!==2&&run?.phase!=='failed';
  const progressHint=working&&!flow.status?.active?'正在结束当前步骤，已完成结果会保留。':finished?'已处理包含未适配、暂缓和失败，不等于全部导入。':phase===2&&flow.status?.active?'等待微信正常退出，暂未开始下一库。':handling?(phase===1?'正在逐库验真并保护密钥。':'正在读取并校验当前库，完成后更新进度。'):'已暂停，点击继续后才会处理来源。';
  return <section className="onboarding" aria-label="离线快照导入">
    <div className="setup-heading"><div><p className="eyebrow">// SNAPSHOT IMPORT</p><h2 ref={title} tabIndex={-1}>{finished?'导入结果':run?'离线归档进度':'导入历史快照'}</h2></div><button className="icon-button" aria-label="稍后设置" onClick={()=>void flow.close().catch(e=>setError(String(e)))}><X size={18}/></button></div>
    <ol className="setup-steps">{phases.map((name,index)=><li key={name} aria-current={phase===index?'step':undefined} data-complete={phase>index}><span>{phase>index?<Check size={16}/>:String(index+1).padStart(2,'0')}</span><strong>{name}</strong></li>)}</ol>
    {(error||flow.error)&&<p className="error-notice" role="alert">{error||flow.error}</p>}
    {!run?<div className="setup-stage" key="choose">
      <p>仅在需要历史归档时导入。日常连接无需处理全部数据库。</p>
      <p className="field" id="setup-detection-notice">点击即允许检测本机安装、进程和数据目录元数据。</p>
      <button className="button primary" aria-describedby="setup-detection-notice" aria-busy={detecting} disabled={disabled} onClick={()=>void detect()}>{detecting?'正在检测':report?'重新检测':'开始检测'}<ArrowRight size={16}/></button>
      {report&&<div className="setup-selection">
        <p className="field">推荐客户端 / {client?`${client.product_name} / ${client.version}`:'未发现客户端'}</p>
        <p className="field">推荐来源 / {candidate?`${candidate.directory_name} / ${selected.length} 个库，全部纳入`:'未发现数据目录'}</p>
        <p className="field">可在高级设置调整；推荐不代表身份已确认。</p>
        {!pid&&<p role="status">请启动并登录微信，然后点击“重新检测”。</p>}
        {selected.length>MAX_SETUP_DATABASES&&<p role="alert">发现超过 {MAX_SETUP_DATABASES} 个库，请在高级设置更换来源；未截断或遗漏提交。</p>}
        {(report.clients.warnings.length>0||report.catalog.warnings.length>0)&&<p role="status">部分位置未完成检测，可在高级设置中指定路径。</p>}
        <p className="field" id="setup-start-notice">点击即允许对当前来源只读取钥、保护保存，并在可离线读取后导入全部库。</p>
        <button className="button primary" aria-describedby="setup-start-notice" disabled={disabled||!valid} onClick={()=>{void flow.action('setup_start',{confirmed:true,request:{label:label.trim(),executable:client!.executable_path,process_id:Number(pid),databases:selected.map(p=>`${candidate!.account_directory.replace(/[\\/]$/,'')}/${p}`)}});}}>开始配置<ArrowRight size={16}/></button>
      </div>}
      <details><summary>高级设置（可选）</summary>
        {report&&<div className="setup-fields"><label>客户端<select aria-label="引导客户端" value={installation} disabled={disabled} onChange={e=>chooseClient(e.target.value)}>{report.clients.installations.map(i=><option key={i.id} value={i.id}>{i.product_name} / {i.version} / {i.executable_path}</option>)}</select></label><label>数据来源<select aria-label="引导数据来源" value={source} disabled={disabled} onChange={e=>setSource(e.target.value)}>{report.catalog.sources.map(s=><option key={s.id} value={s.id}>{s.directory_name} / {s.databases.length} 个库 / {s.account_directory}</option>)}</select></label><label>进程 PID<select aria-label="引导进程PID" value={pid} disabled={disabled} onChange={e=>setPid(e.target.value)}>{pids.length===0&&<option value="">请启动并登录微信后重新检测</option>}{pids.map(id=><option key={id} value={id}>{id}{report.main_process_ids.includes(id)?' / 主窗口候选':''}</option>)}</select></label><label>账号标签<input aria-label="引导账号标签" maxLength={80} value={label} disabled={disabled} onChange={e=>setLabel(e.target.value)}/></label></div>}
        <label>数据根目录<input aria-label="引导数据根目录" value={root} disabled={disabled} onChange={e=>{setRoot(e.target.value);invalidate();}}/></label>
        <label>客户端 EXE<input aria-label="引导客户端路径" value={manual} disabled={disabled} onChange={e=>{setManual(e.target.value);invalidate();}}/></label>
      </details>
    </div>:<div className="setup-stage" key={phase}>
      {flow.restored&&!finished&&<p className="field" role="status">上次保存的设置结果，尚未重新执行。</p>}
      <div className="setup-progress-panel">
        <div className="setup-progress-title"><h3>{headline}</h3><strong className="field">{phase===1?'已检查':'已处理'} {progress!.processed}/{progress!.total} 个库</strong></div>
        <div className="setup-progress" role="progressbar" aria-busy={running} aria-label={phase===1?'密钥检查进度':'数据库处理进度'} aria-valuemin={0} aria-valuemax={progress!.total} aria-valuenow={progress!.processed} aria-valuetext={`${progress!.processed}/${progress!.total} 个库；按库计数，不代表全部导入`}><i style={{transform:`scaleX(${progress!.fraction})`}}/></div>
        <div className="setup-current" role="status" aria-live="polite">
          <div>{progress!.current&&!finished&&<p><span className="field">{handling?'当前处理':phase===2?'等待离线':'待继续'} / {progress!.currentIndex+1}</span> <code>{progress!.current.path.split(/[\\/]/).slice(-2).join('/')}</code></p>}<p className="field">{progressHint}</p></div>
          {flow.status?.active&&!finished&&<button className="button" onClick={()=>void flow.pause().catch(e=>setError(String(e)))}><Pause size={16}/>暂停</button>}
        </div>
        {handling&&<div className="setup-activity" data-running="true" aria-hidden="true"><i/></div>}
        <p className="field setup-counts">首验 {keys}/{run.items.length} │ 已导入 {done} │ 未适配 {unsupported} │ 超预算 {deferred} │ 失败 {failed} │ 缺密钥 {missingKeys}</p>
      </div>
      {phase===2&&!finished&&<p>请从微信菜单正常退出；仅关闭窗口不会退出。可离线读取后会继续，不会强制结束微信。</p>}
      {working&&!flow.status?.active&&<p role="status">正在停止，等待当前步骤结束；已完成结果会保留。</p>}
      <ul className="setup-results">{run.items.map((item,index)=><li key={item.path} data-complete={imported(item)} data-current={handling&&index===progress!.currentIndex}><span className="field">{String(index+1).padStart(2,'0')}</span><code>{item.path.split(/[\\/]/).slice(-2).join('/')}</code><span>{handling&&index===progress!.currentIndex?'正在处理…':resultText(item)}</span></li>)}</ul>
      {!finished&&!flow.status?.active&&<div className="setup-actions"><p className="field">点击继续即重新确认当前来源的取钥与离线导入。隐藏或重启后保持暂停。</p><button className="button primary" disabled={disabled} onClick={()=>{void flow.action('setup_resume',{id:run.id,confirmed:true,skip:false});}}><Play size={16}/>继续 / 重试</button>{run.items.some(i=>i.error||i.outcome==='failed')&&<button className="button" disabled={disabled} onClick={()=>{void flow.action('setup_resume',{id:run.id,confirmed:true,skip:true});}}>跳过失败项并继续</button>}</div>}
      {finished&&<div className="setup-actions">{done>0&&<button className="button primary" disabled={disabled} onClick={()=>{void flow.close();navigate('history');}}>查看记录<ArrowRight size={16}/></button>}<button className="button" disabled={disabled} onClick={()=>{void flow.close();navigate('api');}}>配置 API</button><p className="field">配置结果不代表实时收发可用。API 不会自动启动。</p></div>}
      {!flow.status?.active&&!working&&<details><summary>重新选择来源</summary><p className="field">只清除引导进度，已保存的密钥与记录保留。更换安装需先解除旧绑定。</p><button className="button" disabled={disabled} onClick={()=>void flow.action('setup_reset')}><RotateCcw size={16}/>重置引导</button></details>}
    </div>}
    {!run&&<button className="button setup-later" onClick={()=>void flow.close()}>稍后设置，使用手工配置</button>}
  </section>;
}
