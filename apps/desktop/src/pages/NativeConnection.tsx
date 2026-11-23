import {useEffect,useRef,useState} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
import type {Workbench} from '../hooks/useWorkbench';
import {HelpTip} from '../components/HelpTip';
import {componentRepairHint,nativeConnectionErrorText,rememberConnection} from '../lib/connection-resume';
export interface NativeStatus {component_available:boolean;connected:boolean;account_id:string|null;error_code:string|null;scope:string;process_id?:number|null}
export function NativeConnection({accountId,...workbench}:Workbench&{accountId:string}) {
  const {data,busy,act}=workbench;
  const [status,setStatus]=useState<NativeStatus|null>(null),[processes,setProcesses]=useState<number[]>([]),[pid,setPid]=useState(''),[error,setError]=useState('');
  const generation=useRef(0);
  useEffect(()=>{generation.current++;setProcesses([]);setPid('');setError('');setStatus(null);return()=>{generation.current++;};},[accountId]);
  const binding=data.bindings.find(b=>b.account_id===accountId);
  useEffect(()=>{if(!desktopAvailable)return;let alive=true,pending=false;
    const refresh=async()=>{if(pending)return;pending=true;try{const s=await command<NativeStatus>('native_status');if(alive)setStatus(s);}catch(e){if(alive)setError(e instanceof Error?e.message:String(e));}finally{pending=false;}};
    void refresh();const timer=window.setInterval(()=>void refresh(),2000);return()=>{alive=false;window.clearInterval(timer);};
  },[accountId]);
  async function detect(){if(!binding)return [];const ticket=generation.current;const ids=await command<number[]>('native_processes',{accountId});if(ticket===generation.current){setProcesses(ids);setPid(ids.length===1?String(ids[0]):'');}return ids;}
  async function connect(selected?:number){
    const ticket=generation.current;setError('');
    const ids=selected?[selected]:await detect();
    if(ticket!==generation.current)return;
    if(ids.length!==1){setError(ids.length?'检测到多个微信进程，请在下方选择需要连接的进程。':'未检测到已绑定的微信进程，请先打开并登录电脑微信。');return;}
    const next=await command<NativeStatus>('native_connect',{accountId,revision:data.current_connection.revision,processId:ids[0]}).catch(e=>{throw new Error(nativeConnectionErrorText(e));});
    if(ticket===generation.current)setStatus(next);
    if(next.connected)rememberConnection(accountId,true);
  }
  return <section className="native-connection" aria-label="原生连接"><div className="section-title"><h3>原生消息通道 <HelpTip label="原生连接是什么">连接电脑上已登录的微信，接收新消息并使用后台发送通道。无需先导入历史；支持范围按当前微信版本和能力检查结果显示。</HelpTip></h3><span className="state-tag">{status?.connected?'原生通道已连接':'原生通道未连接'}</span></div>
    <p className="field">接收新消息与原生发送 · 仅个人非商业自用</p>
    {status?.connected?<button className="button" disabled={busy} onClick={()=>void act(async()=>{rememberConnection(accountId,false);setStatus(await command<NativeStatus>('native_disconnect'));})}>断开原生通道</button>:<div className="actions">
      <button className="button primary" disabled={busy||!desktopAvailable||!binding||!status?.component_available} onClick={()=>void act(()=>connect())}>连接当前微信</button>
      {processes.length>1&&<><label>当前进程<select aria-label="原生微信进程" disabled={busy} value={pid} onChange={e=>setPid(e.target.value)}><option value="">请选择</option>{processes.map(p=><option key={p} value={p}>{p}</option>)}</select></label><button className="button" disabled={busy||!pid} onClick={()=>void act(()=>connect(Number(pid)))}>连接所选进程</button></>}
    </div>}
    {!binding&&<p className="field">先在“配置”中绑定本机客户端。</p>}
    {status&&!status.component_available&&<p className="field">{componentRepairHint}</p>}
    {(status?.error_code||error)&&<p className="error-notice" role="alert">{error||(['INTERNAL_ERROR','KEY_PROTECTION_FAILED'].includes(status?.error_code??'')&&status?.connected?'接收保存暂时失败，保留待确认消息并自动重试。':status?.error_code==='NATIVE_ADAPTER_PENDING'?'当前微信版本的原生通道待适配':status?.error_code==='NATIVE_BUILD_UNSUPPORTED'?'当前微信构建暂不支持原生通道':status?.error_code)}{status?.error_code&&!status.connected&&<HelpTip label="连接状态详情">错误码：{status.error_code}。精确版本支持范围见下方说明。</HelpTip>}{status?.connected&&status.error_code&&<HelpTip label="接收保存失败说明">{status.error_code==='KEY_PROTECTION_FAILED'?'Windows密钥保护暂时失败，请检查当前Windows用户环境。':'请检查可用磁盘空间与产品数据文件是否可写。'}错误码：{status.error_code}。尚未保存的消息不会被确认消费；原生缓冲有限，持续故障仍可能产生缺口。</HelpTip>}</p>}
    <details><summary>支持范围</summary><p className="field">自动检测实际加载的微信版本与 DLL 指纹，并选择匹配的适配档案。支持 4.2.0.53、4.2.0.52、4.1.15.13 和 4.1.13.7 的匹配 x64 构建。以连接时逐项检测的结果为准。文字、图片、语音、文件、视频、表情、引用与群提及通过同一发送队列处理；接收消息与按需取得的媒体加密保存。连接中断后自动补读已发现的来源，未同步或已删除的内容可能无法恢复。</p><HelpTip label="格式与发送结果说明">文字最多10000个UTF-8字节；语音最长60秒，导入16kHz单声道PCM16；普通文件、图片和发送视频各有格式与大小限制，预览时会检查。桌面发送先预览批准，授权Bot可直接提交。提交、服务端接受与接收端确认分别记录；结果未知时只回查，不自动重发。</HelpTip></details>
  </section>;
}
