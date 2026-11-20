import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
interface VideoAsset {id:string;size:number;video:{width:number;height:number;duration_ms:number;audio:boolean}}
export function ReceivedVideo({accountId,position,connected}:{accountId:string;position:number;connected:boolean}) {
  const [asset,setAsset]=useState<VideoAsset|null>(null),[busy,setBusy]=useState(false),[loading,setLoading]=useState(true);
  const [error,setError]=useState(''),[url,setUrl]=useState(''),[path,setPath]=useState(''),[exported,setExported]=useState(false);
  const generation=useRef(0),objectUrl=useRef(''),player=useRef<HTMLVideoElement>(null),inFlight=useRef(false);
  function release(){player.current?.pause();if(objectUrl.current)URL.revokeObjectURL(objectUrl.current);objectUrl.current='';}
  function close(){release();setUrl('');}
  useEffect(()=>{
    const token=++generation.current;inFlight.current=false;setAsset(null);setBusy(false);setLoading(true);setError('');setUrl('');setPath(`视频-${position}.mp4`);setExported(false);
    void command<VideoAsset|null>('native_received_video_asset',{accountId,position}).then(a=>{if(token===generation.current)setAsset(a);},e=>{if(token===generation.current)setError(e instanceof Error?e.message:String(e));}).finally(()=>{if(token===generation.current)setLoading(false);});
    return()=>{generation.current++;release();};
  },[accountId,position]);
  async function perform(action:'view'|'save'|'delete') {
    if(inFlight.current)return;
    inFlight.current=true;const token=generation.current;setBusy(true);setError('');setExported(false);
    try {
      if(action==='save'){await command('native_received_video_save',{accountId,position,path});if(token===generation.current)setExported(true);return;}
      if(action==='delete'){await command('native_received_video_delete',{accountId,position});if(token===generation.current){close();setAsset(null);}return;}
      if(!asset){const next=await command<VideoAsset>('native_received_video_download',{accountId,position});if(token!==generation.current)return;setAsset(next);}
      const buffer=await command<ArrayBuffer>('native_received_video_preview',{accountId,position});
      if(!(buffer instanceof ArrayBuffer))throw new Error('视频响应格式无效');
      const bytes=new Uint8Array(buffer);
      try {
        if(token!==generation.current)return;
        if(!bytes.length||bytes.length>16*1024*1024)throw new Error('视频内容大小无效');
        const next=URL.createObjectURL(new Blob([buffer],{type:'video/mp4'}));release();objectUrl.current=next;setUrl(next);
      }finally{bytes.fill(0);}
    }catch(e){if(token===generation.current){const message=e instanceof Error?e.message:String(e);setError(message);if(message.includes('ASSET_NOT_FOUND')){close();setAsset(null);}}}
    finally{if(token===generation.current){inFlight.current=false;setBusy(false);}}
  }
  return <div className="received-video" aria-busy={busy||loading}>
    <div className="button-row">{url?<button className="button" disabled={busy} onClick={close}>收起视频</button>:<button className="button" disabled={loading||busy||(!asset&&!connected)} onClick={()=>void perform('view')}>{loading?'正在检查缓存…':busy?'正在读取视频…':asset?'查看已保存视频':'下载并查看视频'}</button>}
      <HelpTip label="视频查看说明">点击后下载并在本机加密缓存24小时。支持 H.264 / AAC MP4，单段最多16MiB、10分钟；需原生连接才能下载，已保存内容可离线查看。总缓存64MiB / 32项。另存会在系统下载目录创建普通视频文件，不覆盖同名文件。</HelpTip>
    </div>
    {url&&<video ref={player} src={url} controls playsInline preload="metadata" aria-label="收到的视频" style={{display:'block',width:'100%',maxHeight:480,objectFit:'contain'}} onError={()=>setError('当前设备无法播放这段视频。可尝试另存到电脑后使用本机播放器打开。')}/>}
    {!loading&&!asset&&!connected&&<span className="field">连接原生通道后可下载</span>}
    {asset&&<details><summary>更多视频操作</summary><span className="field">{asset.video.width} × {asset.video.height} · {(asset.video.duration_ms/1000).toFixed(1)} 秒 · {(asset.size/1024/1024).toFixed(2)} MiB</span>
      <form onSubmit={e=>{e.preventDefault();void perform('save');}}><label>下载目录内文件名<input aria-label="接收视频保存文件名" value={path} onChange={e=>{setPath(e.target.value);setExported(false);}} disabled={busy}/></label><button className="button" disabled={busy||!path.trim()}>另存到电脑</button></form>
      <button className="button" disabled={busy} onClick={()=>void perform('delete')}>清除本机视频缓存</button>
    </details>}
    {exported&&<p role="status">视频已保存到系统下载目录。</p>}{error&&<p className="error-notice" role="alert">{error}</p>}
  </div>;
}
