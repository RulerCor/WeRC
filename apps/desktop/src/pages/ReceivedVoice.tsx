import {useEffect,useRef,useState} from 'react';
import {command,type FileAsset} from '../lib/desktop';
import {Transcription} from './Transcription';

export function ReceivedVoice({accountId,position}:{accountId:string;position:number}) {
  const [saved,setSaved]=useState(false),[loaded,setLoaded]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState(''),[url,setUrl]=useState('');
  const generation=useRef(0),objectUrl=useRef(''),audio=useRef<HTMLAudioElement>(null);
  useEffect(()=>{const token=++generation.current;setSaved(false);setLoaded(false);setBusy(false);setError('');setUrl('');void command<FileAsset|null>('native_received_voice_asset',{accountId,position}).then(a=>{if(token===generation.current){setSaved(Boolean(a));setLoaded(true);}},e=>{if(token===generation.current){setError(e instanceof Error?e.message:String(e));setLoaded(true);}});return()=>{generation.current++;if(objectUrl.current)URL.revokeObjectURL(objectUrl.current);objectUrl.current='';};},[accountId,position]);
  async function prepare() {
    const token=generation.current;
    setBusy(true);setError('');
    try {
      const values=await command<number[]>('native_received_voice_preview',{accountId,position});
      const bytes=new Uint8Array(values);values.fill(0);
      try {
        if(token!==generation.current)return;
        if(bytes.length<44||bytes.length>1920044)throw new Error('语音内容大小无效');
        const next=URL.createObjectURL(new Blob([bytes],{type:'audio/wav'}));
        if(objectUrl.current)URL.revokeObjectURL(objectUrl.current);objectUrl.current=next;setUrl(next);
      }finally{bytes.fill(0);}
    }catch(e){if(token===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(token===generation.current)setBusy(false);}
  }
  function close(){audio.current?.pause();if(objectUrl.current)URL.revokeObjectURL(objectUrl.current);objectUrl.current='';setUrl('');}
  return <div className="received-voice">
    {url?<><audio ref={audio} controls preload="metadata" src={url} aria-label="收到的语音" style={{display:'block',maxWidth:'100%'}}/><button className="button" onClick={close}>收起语音</button></>:<button className="button" disabled={!saved||busy} onClick={()=>void prepare()}>{busy?'正在读取语音…':'准备预听'}</button>}
    {loaded&&!saved&&<span className="field">未保存音频内容或缓存已过期</span>}
    {saved&&<span className="field">本机加密缓存 / 24小时 · 点击播放器播放</span>}
    {saved&&<Transcription accountId={accountId} source={{kind:'received',position}} disabled={busy}/>}
    {error&&<p className="error-notice" role="alert">{error}</p>}
  </div>;
}
