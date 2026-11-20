import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {HelpTip} from '../components/HelpTip';
import {EmoticonPreview} from './EmoticonPreview';

type Source = {viewId:string;localId:string;position?:never} | {position:number;viewId?:never;localId?:never};
export function ReceivedEmoticon({accountId,connected=true,...source}:{accountId:string;connected?:boolean}&Source) {
  const [asset,setAsset]=useState<{id:string}|null>(null),[busy,setBusy]=useState(false),[canceling,setCanceling]=useState(false);
  const [error,setError]=useState(''),[path,setPath]=useState('received-emoticon.gif'),[exported,setExported]=useState(false);
  const generation=useRef(0),request=useRef<string|null>(null),working=useRef(false);
  const {position,viewId,localId}=source;
  useEffect(()=>{
    const expected=++generation.current;setAsset(null);setBusy(false);setCanceling(false);setError('');setExported(false);setPath('received-emoticon.gif');working.current=false;
    if(position!==undefined)void command<{id:string}|null>('native_received_emoticon_asset',{accountId,position}).then(v=>{if(expected===generation.current)setAsset(v);},e=>{if(expected===generation.current)setError(e instanceof Error?e.message:String(e));});
    return()=>{generation.current++;const id=request.current;request.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});};
  },[accountId,position,viewId,localId]);
  useEffect(()=>{let active=true;if(asset)void command<{file_name:string}>('read_file_preview',{accountId,id:asset.id}).then(v=>{if(active)setPath(v.file_name.endsWith('.wxgf')?'received-emoticon.png':'received-emoticon.gif');},e=>{if(active)setError(e instanceof Error?e.message:String(e));});return()=>{active=false;};},[accountId,asset]);
  async function run(action:'read'|'export'|'delete') {
    if(working.current)return;
    working.current=true;const expected=++generation.current;setBusy(true);setError('');setExported(false);
    try {
      if(action==='read') {
        const requestId=crypto.randomUUID();request.current=requestId;
        const value=position!==undefined
          ? await command<{id:string}>('native_received_emoticon_read',{accountId,position,requestId})
          : await command<{id:string}>('native_live_emoticon_save',{accountId,viewId,localId,requestId});
        if(expected===generation.current)setAsset(value);
      } else if(asset) {
        if(action==='export') {await command('export_emoticon_content',{accountId,id:asset.id,path});if(expected===generation.current)setExported(true);}
        else {await command('delete_file',{accountId,id:asset.id});if(expected===generation.current)setAsset(null);}
      }
    } catch(e) {if(expected===generation.current)setError(e instanceof Error?e.message:String(e));}
    finally {if(expected===generation.current){working.current=false;request.current=null;setBusy(false);setCanceling(false);}}
  }
  async function cancel(){const id=request.current;if(!id)return;const expected=generation.current;setCanceling(true);try{await command('cancel_browse',{requestId:id});}catch(e){if(expected===generation.current){setError(e instanceof Error?e.message:String(e));setCanceling(false);}}}
  return <div className="received-emoticon">
    <div className="actions">{asset?<span className="field" role="status">表情已加密保存</span>:<button className="button" disabled={busy||!connected} onClick={()=>void run('read')}>{busy?'正在读取表情…':'读取表情'}</button>}
      {busy&&request.current&&<button className="button" disabled={canceling} onClick={()=>void cancel()}>{canceling?'正在取消…':'取消'}</button>}
      <HelpTip label="接收表情说明">点击后从当前微信账号的本地缓存读取这条表情，完整校验后加密保存24小时。保存后点播放即可预览，默认不会播放。微信压缩缓存按原始帧时长和透明度生成 PNG 动画预览，是缓存的转换副本。未缓存或不支持的格式会说明原因。此操作不会发送消息；删除只移除 WeRC 中的副本。</HelpTip>
    </div>
    {asset&&<><EmoticonPreview key={`${accountId}:${asset.id}`} accountId={accountId} assetId={asset.id} onError={setError}/>
      <details><summary>另存与管理</summary><form onSubmit={e=>{e.preventDefault();void run('export');}}>
        <label>保存到下载目录<input aria-label="表情保存文件名" value={path} disabled={busy} onChange={e=>{setPath(e.target.value);setExported(false);}}/></label>
        <button className="button" disabled={busy||!path.trim()}>另存动画</button><HelpTip label="表情保存位置说明">填写文件名或下载目录内的相对路径。会使用这台电脑的系统下载目录，不覆盖已有文件。GIF 保持 .gif；转换后的动画使用 .png，可在支持动画 PNG 的查看器中播放。</HelpTip>
      </form><button className="button danger" disabled={busy} onClick={()=>void run('delete')}>删除本地副本</button></details></>}
    {exported&&<p role="status">已保存到下载目录。</p>}
    {error&&<p className="error-notice" role="alert">{error}</p>}
  </div>;
}
