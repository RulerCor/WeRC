import {useEffect,useState} from 'react';
import {command,desktopAvailable} from '../lib/desktop';
import {HelpTip} from './HelpTip';
interface Detection {available:boolean;reason:string;devices:{name:string;driver:string;compute_capability:string;compatible:boolean}[];download_bytes:number;plan:string;installed:boolean}
interface Progress {id:string;model?:string;phase:string;bytes:number;total:number;error:{message:string;code:string}|null}
const running=(p:Progress|null)=>!!p&&['starting','downloading','importing','verifying'].includes(p.phase);
const phases:Record<string,string>={starting:'准备中',downloading:'下载中',verifying:'校验与解压',complete:'组件已安装',canceled:'安装已取消',failed:'安装失败'};
export function GpuAcceleration(){
  const [detected,setDetected]=useState<Detection|null>(null),[busy,setBusy]=useState(false),[progress,setProgress]=useState<Progress|null>(null),[error,setError]=useState('');
  const active=running(progress);
  useEffect(()=>{let alive=true;if(desktopAvailable)void command<{job:Progress|null}>('model_catalog').then(value=>{if(alive&&value.job?.model==='CUDA 识别组件')setProgress(current=>current??value.job);}).catch(e=>{if(alive)setError(String(e));});return()=>{alive=false;};},[]);
  useEffect(()=>{if(!active||!progress)return;let alive=true,timer:ReturnType<typeof setTimeout>;const id=progress.id;async function poll(){try{const value=await command<{job:Progress|null}>('model_catalog');if(alive&&value.job?.id===id)setProgress(value.job);}catch(e){if(alive)setError(String(e));}finally{if(alive)timer=setTimeout(()=>void poll(),700);}}void poll();return()=>{alive=false;clearTimeout(timer);};},[active,progress?.id]);
  async function detect(){setBusy(true);setError('');try{const value=await command<Detection>('cuda_detect');setDetected(value);if(!value.installed)setProgress(null);}catch(e){setError(String(e));}finally{setBusy(false);}}
  async function install(){setBusy(true);setError('');try{setProgress(await command<Progress>('install_cuda_runtime'));}catch(e){setError(String(e));}finally{setBusy(false);}}
  return <div className="gpu-setup"><div className="actions"><button type="button" className="button" disabled={!desktopAvailable||busy||active} onClick={()=>void detect()}>{busy?'正在处理…':'检测本机 GPU'}</button><HelpTip label="GPU 加速准备说明">CUDA 仅适用于 NVIDIA。检测不会联网；确认安装后才下载匹配的识别运行时、CUDA、cuDNN 库和一个小型中文识别模型，不安装开发工具包、不修改显卡驱动。AMD、Intel 和不满足条件的设备继续选 CPU。CPU 与 CUDA 组件分别保存，已有模型可以共用。短语音不一定更快，建议保留默认 CPU。</HelpTip></div>
    {detected&&<><p role="status">{detected.reason}</p>{detected.devices.map((d,i)=><p key={i}>{d.name} · 驱动 {d.driver} · {d.compatible?'符合当前方案':'不符合当前方案'}</p>)}{detected.available&&!detected.installed&&progress?.phase!=='complete'&&<div><p>下载约 {(detected.download_bytes/1024**3).toFixed(2)} GiB，安装期间需额外解压空间。</p><button type="button" className="button" disabled={busy||active} onClick={()=>void install()}>下载并安装配套 CUDA 组件</button></div>}{(detected.installed||progress?.phase==='complete')&&<p role="status">CUDA 组件已安装。保存计算设备设置后，即可使用随附的 Paraformer 中文模型；更多模型可在离线模型管理中导入。</p>}</>}
    {progress&&<div role="status"><p>{phases[progress.phase]??progress.phase} · {(progress.bytes/1024**2).toFixed(1)} / {(progress.total/1024**2).toFixed(1)} MiB</p><progress max={progress.total||1} value={progress.bytes}/>{active&&<button type="button" className="button" onClick={()=>void command('cancel_model_install',{id:progress.id}).catch(e=>setError(String(e)))}>取消安装</button>}{progress.error&&<p role="alert">{progress.error.code} · {progress.error.message}</p>}</div>}{error&&<p role="alert" className="error-notice">{error}</p>}
  </div>;
}
