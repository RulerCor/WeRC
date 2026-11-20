import {useEffect,useRef,useState} from 'react';
import {command,desktopAvailable,type Bootstrap} from '../lib/desktop';
import {readResume} from '../lib/connection-resume';
import type {NativeStatus} from '../pages/NativeConnection';

type ResumeNotice = {accountId:string;message:string;kind:'pending'|'connection'|'api'};
// Restore only the same account explicitly configured by the setup wizard.
// One attempt per mount; never register accounts, issue tokens, or submit sends.
export function useConnectionResume(enabled:boolean,data:Bootstrap,nativeStatus:NativeStatus|null,refresh:()=>Promise<void>){
  const accountId=data.current_connection.account_id;
  const connected=Boolean(accountId&&nativeStatus?.connected&&nativeStatus.account_id===accountId);
  const started=useRef(false),[state,setState]=useState<ResumeNotice|null>(null);
  const visible=enabled&&state?.accountId===accountId&&!(state.kind==='connection'&&connected)&&!(state.kind==='api'&&data.status.api_enabled);
  useEffect(()=>{
    if(state&&!visible)setState(null);
  },[state,visible]);
  useEffect(()=>{
    if(!enabled||!desktopAvailable||started.current||!data.current_connection.account_id)return;
    const preference=readResume();if(!preference||preference.accountId!==data.current_connection.account_id)return;
    started.current=true;let alive=true,kind:'connection'|'api'='connection';
    void(async()=>{
      setState({accountId:preference.accountId,kind:'pending',message:'正在恢复上次连接…'});
      const current=await command<Bootstrap>('bootstrap'),id=current.current_connection.account_id;
      if(!alive)return;
      if(id!==preference.accountId||current.current_connection.selection_required){setState(null);return;}
      let status=await command<NativeStatus>('native_status');
      if(preference.connect&&(!status.connected||status.account_id!==id)){
        const ids=await command<number[]>('native_processes',{accountId:id});
        if(!alive)return;
        if(ids.length!==1)throw Error(ids.length?'发现多个微信进程，请到连接页选择。':'请先登录微信，再到连接页点击连接。');
        status=await command<NativeStatus>('native_connect',{accountId:id,revision:current.current_connection.revision,processId:ids[0]});
        if(!status.connected)throw Error(status.error_code??'微信连接尚未恢复。');
      }
      if(!alive)return;
      const ready=await command<Bootstrap>('bootstrap');
      if(preference.apiPort!==null&&!ready.status.api_enabled&&ready.current_connection.account_id===id&&status.connected&&status.account_id===id){
        kind='api';
        await command('set_api_enabled',{enabled:true,port:preference.apiPort});
      }
      await refresh();if(alive)setState(null);
    })().catch(e=>{if(alive)setState({accountId:preference.accountId,kind,message:e instanceof Error?e.message:String(e)});});
    return()=>{alive=false;started.current=false;};
  },[enabled,data.current_connection.account_id,refresh]);
  return visible?state.message:'';
}

export function useNativeConnectionStatus(accountId:string|null,enabled:boolean){
  const [status,setStatus]=useState<NativeStatus|null>(null);
  useEffect(()=>{
    setStatus(null);if(!enabled||!desktopAvailable||!accountId)return;
    let alive=true,pending=false;
    const read=async()=>{if(pending)return;pending=true;try{const next=await command<NativeStatus>('native_status');if(alive)setStatus(next);}catch{if(alive)setStatus(null);}finally{pending=false;}};
    void read();const timer=setInterval(()=>void read(),2000);return()=>{alive=false;clearInterval(timer);};
  },[accountId,enabled]);
  return status;
}
