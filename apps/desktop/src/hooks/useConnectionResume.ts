import {useEffect,useState} from 'react';
import {command,desktopAvailable,type Bootstrap} from '../lib/desktop';
import {readResume} from '../lib/connection-resume';
import type {NativeStatus} from '../pages/NativeConnection';

type ResumeNotice = {accountId:string;message:string;kind:'connection'|'api'};
const retryMs=5000;

// Only the account and options explicitly approved by the user may be restored.
export function useConnectionResume(enabled:boolean,data:Bootstrap,nativeStatus:NativeStatus|null,refresh:()=>Promise<void>){
  const accountId=data.current_connection.account_id;
  const connected=Boolean(accountId&&nativeStatus?.connected&&nativeStatus.account_id===accountId);
  const [state,setState]=useState<ResumeNotice|null>(null);
  useEffect(()=>{
    if(!enabled||!desktopAvailable||!accountId)return;
    let alive=true,busy=false,failedProcess:number|null=null,failedMessage='',retryAfter=0;
    const preference=()=>{
      const saved=readResume();
      return alive&&saved?.accountId===accountId?saved:null;
    };
    const clear=()=>{if(alive)setState(previous=>previous?.accountId===accountId?null:previous);};
    const show=(kind:ResumeNotice['kind'],message:string)=>{
      if(alive)setState(previous=>previous?.accountId===accountId&&previous.kind===kind&&previous.message===message?previous:{accountId,kind,message});
    };
    async function restore(){
      const saved=preference();
      if(busy)return;
      if(!saved||(!saved.connect&&saved.apiPort===null)){failedProcess=null;clear();return;}
      busy=true;
      let kind:ResumeNotice['kind']='connection';
      try{
        let status=await command<NativeStatus>('native_status');
        if(!preference())return;
        if(!status.connected&&saved.connect&&status.component_available){
          const current=await command<Bootstrap>('bootstrap');
          if(current.current_connection.account_id!==accountId||current.current_connection.selection_required||!preference()?.connect)return;
          const ids=await command<number[]>('native_processes',{accountId});
          if(!preference()?.connect)return;
          if(ids.length!==1){
            failedProcess=null;
            show(kind,ids.length?'发现多个微信进程，请到连接页选择。':'等待同一账号登录微信，登录后自动恢复连接。');
            return;
          }
          if(ids[0]===failedProcess&&Date.now()<retryAfter){show(kind,failedMessage);return;}
          try{
            status=await command<NativeStatus>('native_connect',{accountId,revision:current.current_connection.revision,processId:ids[0]});
            if(!status.connected)throw Object.assign(new Error(status.error_code??'微信连接尚未恢复。'),{code:status.error_code});
            await refresh();
          }catch(error){
            const code=(error as {code?:string}|null)?.code;
            if(['NATIVE_LOGIN_REQUIRED','NATIVE_IDENTITY_UNVERIFIED','NATIVE_PROCESS_UNAVAILABLE','NATIVE_BUSY'].includes(code??'')){
              failedProcess=ids[0];retryAfter=Date.now()+30000;
              failedMessage=code==='NATIVE_BUSY'?'微信连接正在释放，稍后自动恢复。':'等待同一账号登录微信，登录后自动恢复连接。';
              show(kind,failedMessage);
            }else if(!['BUSY','NATIVE_ALREADY_CONNECTED','CONNECTION_CHANGED'].includes(code??'')){
              retryAfter=Infinity;
              failedProcess=ids[0];
              failedMessage=`自动连接未成功：${error instanceof Error?error.message:String(error)}。可在连接页重试。`;
              show(kind,failedMessage);
            }
            return;
          }
        }
        if(!preference()||!status.connected||status.account_id!==accountId){clear();return;}
        failedProcess=null;
        if(preference()?.apiPort==null||data.status.api_enabled){clear();return;}
        const ready=await command<Bootstrap>('bootstrap');
        const port=preference()?.apiPort;
        if(port!=null&&!ready.status.api_enabled&&ready.current_connection.account_id===accountId&&!ready.current_connection.selection_required){
          kind='api';
          await command('set_api_enabled',{enabled:true,port});
          await refresh();
        }
        clear();
      }catch(e){show(kind,e instanceof Error?e.message:String(e));}
      finally{busy=false;}
    }
    void restore();
    const timer=window.setInterval(()=>void restore(),retryMs);
    return()=>{alive=false;window.clearInterval(timer);};
  },[enabled,accountId,refresh,data.status.api_enabled]);
  return enabled&&state?.accountId===accountId&&!(state.kind==='connection'&&connected)&&!(state.kind==='api'&&data.status.api_enabled)?state.message:'';
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
