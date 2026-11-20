import {useEffect,useRef,useState} from 'react';
import {listen} from '@tauri-apps/api/event';
import {command,desktopAvailable} from '../lib/desktop';
export function ClosePrompt(){
  const dialog=useRef<HTMLDialogElement>(null),[busy,setBusy]=useState(false),[error,setError]=useState('');
  useEffect(()=>{if(!desktopAvailable)return;let alive=true,dispose:(()=>void)|undefined;void listen('werc-close-request',()=>{if(alive&&!dialog.current?.open)dialog.current?.showModal();}).then(fn=>{if(alive)dispose=fn;else fn();});return()=>{alive=false;dispose?.();};},[]);
  async function close(action:string){setBusy(true);setError('');try{await command('close_desktop',{action});dialog.current?.close();}catch(e){setError(String(e));}finally{setBusy(false);}}
  return <dialog ref={dialog} aria-labelledby="close-title"><h2 id="close-title">关闭 WeRC</h2><p>留在托盘会继续接收消息并提供 Bot 接口；退出会停止本机服务。</p><div className="actions"><button className="button primary" disabled={busy} onClick={()=>void close('tray')}>留在托盘</button><button className="button" disabled={busy} onClick={()=>void close('exit')}>退出程序</button><button className="button" disabled={busy} onClick={()=>dialog.current?.close()}>返回</button></div>{error&&<p role="alert">{error}</p>}</dialog>;
}
