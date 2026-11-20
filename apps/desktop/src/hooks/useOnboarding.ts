import { useCallback, useEffect, useRef, useState } from 'react';
import { command, desktopAvailable, type ClientBinding, type HistorySource } from '../lib/desktop';
import type { Workbench } from './useWorkbench';
export type SetupOutcome='pending'|'imported'|'already_imported'|'unsupported'|'resource_deferred'|'key_deferred'|'failed';
export interface SetupItem { path:string; key_ready:boolean; skipped:boolean; source:HistorySource|null; error:string|null; outcome?:SetupOutcome }
export interface SetupRun { id:string; revision:number; binding:ClientBinding; process_id:number; key_revision:number; phase:string; items:SetupItem[] }
export interface SetupStatus {run:SetupRun|null;active:boolean;working:boolean;dismissed:boolean}
export function shouldOpenSetup(_status:SetupStatus,_accountCount:number) {
  // 历史归档只能由用户显式打开，首次连接不以全库导入为前置。
  return false;
}
export function useOnboarding(workbench:Workbench) {
  const [status,setStatus]=useState<SetupStatus|null>(null),[visible,setVisible]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState(''),[restored,setRestored]=useState(false);
  const started=useRef(false),gate=useRef(false),mounted=useRef(true);
  const {loaded,refresh}=workbench;
  useEffect(()=>{mounted.current=true;return ()=>{mounted.current=false;if(desktopAvailable) void command('setup_pause').catch(()=>{});};},[]);
  useEffect(()=>{
    if(!loaded||!desktopAvailable||started.current)return;
    started.current=true;
    void (async()=>{
      await command('setup_pause');
      const next=await command<SetupStatus>('setup_status');
      if(!mounted.current)return;
      setStatus(next);
      setRestored(Boolean(next.run));
      setVisible(shouldOpenSetup(next,workbench.data.accounts.length));
    })().catch(e=>{setError(String(e.message));});
  },[loaded,workbench.data.accounts.length]);
  const action=useCallback(async(name:string,args?:Record<string,unknown>)=>{
    if(gate.current)return;
    gate.current=true;setBusy(true);setError('');
    try {await command(name,args); const next=await command<SetupStatus>('setup_status'); if(mounted.current){setStatus(next);if(['setup_start','setup_resume','setup_reset'].includes(name))setRestored(false);}await refresh();}
    catch(e){setError(e instanceof Error?e.message:String(e));await command('setup_pause').catch(()=>{});setStatus(old=>old?{...old,active:false}:old);}
    finally {gate.current=false;if(mounted.current)setBusy(false);}
  },[refresh]);
  useEffect(()=>{
    if(!visible||!status?.active||busy||!status.run)return;
    const timer=setTimeout(()=>{void action('setup_step',{id:status.run!.id});},status.run.phase==='waiting'?1500:100);
    return ()=>clearTimeout(timer);
  },[visible,status,busy,action]);
  useEffect(()=>{
    const hide=()=>{if(document.hidden&&status?.active){void command('setup_pause').catch(()=>{});setStatus(old=>old?{...old,active:false}:old);}};
    document.addEventListener('visibilitychange',hide);return ()=>document.removeEventListener('visibilitychange',hide);
  },[status?.active]);
  // Read saved progress while a native step is running; never start another step.
  useEffect(()=>{
    if(busy?!visible:!status?.working||status.active)return;
    let cancelled=false,timer:ReturnType<typeof setTimeout>;
    async function observe(){
      try {
        const next=await command<SetupStatus>('setup_status');
        if(!cancelled&&mounted.current&&(busy?gate.current:!gate.current))setStatus(old=>old?.run&&next.run&&old.run.id===next.run.id&&old.run.revision>next.run.revision?old:next);
      }catch(e){if(!busy&&!cancelled&&mounted.current)setError(e instanceof Error?e.message:String(e));}
      finally {if(!cancelled)timer=setTimeout(()=>void observe(),750);}
    }
    timer=setTimeout(()=>void observe(),250);
    return ()=>{cancelled=true;clearTimeout(timer);};
  },[busy,visible,status?.working,status?.active]);
  async function pause(){await command('setup_pause');const next=await command<SetupStatus>('setup_status');if(mounted.current)setStatus(next);}
  async function close(){await pause();await command('setup_dismiss');setVisible(false);}
  return {status,visible,busy,error,restored,action,pause,close,open:()=>setVisible(true)};
}
export type OnboardingController=ReturnType<typeof useOnboarding>;
