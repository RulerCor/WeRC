import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {AudioPreview} from './AudioPreview';
import {Transcription} from './Transcription';

export function LiveVoice({accountId,viewId,localId}:{accountId:string;viewId:string;localId:string}) {
  const [asset,setAsset]=useState<{id:string}|null>(null),[busy,setBusy]=useState(false),[canceling,setCanceling]=useState(false),[error,setError]=useState('');
  const generation=useRef(0),request=useRef<string|null>(null);
  useEffect(()=>{
    generation.current++;setAsset(null);setBusy(false);setCanceling(false);setError('');
    return()=>{generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});};
  },[accountId,viewId,localId]);
  async function save(){
    if(request.current)return;
    const expected=generation.current,requestId=crypto.randomUUID();request.current=requestId;setBusy(true);setError('');
    try{const value=await command<{id:string}>('native_live_voice_save',{accountId,viewId,localId,requestId});if(expected===generation.current)setAsset(value);}
    catch(e){if(expected===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally{if(expected===generation.current){request.current=null;setBusy(false);setCanceling(false);}}
  }
  async function cancel(){const id=request.current;if(!id)return;const expected=generation.current;setCanceling(true);try{await command('cancel_browse',{requestId:id});}catch(e){if(expected===generation.current){setError(e instanceof Error?e.message:String(e));setCanceling(false);}}}
  return <div className="live-voice">
    <div className="actions">{asset?<span className="field" role="status">语音已加密保存</span>:<button className="button" disabled={busy} onClick={()=>void save()}>{busy?'正在读取语音…':'读取并保存语音'}</button>}
      {busy&&<button className="button" disabled={canceling} onClick={()=>void cancel()}>{canceling?'正在取消…':'取消'}</button>}
      <HelpTip label="本地语音读取说明">从当前微信本地媒体库读取这条语音，按需临时取钥并完整解码。保存后可在“工具与诊断 → 音频”中回听，有效期24小时。不会发送消息或播放音频；本地未缓存的语音暂时无法读取。取钥期间取消需要等待当前步骤结束。</HelpTip>
    </div>
    {asset&&<AudioPreview key={`${accountId}:${asset.id}`} accountId={accountId} assetId={asset.id} disabled={false} onError={setError}/>}
    {asset&&<Transcription accountId={accountId} source={{kind:'uploaded',asset_id:asset.id}}/>}
    {error&&<p className="error-notice" role="alert">{error}</p>}
  </div>;
}
