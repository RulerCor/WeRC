import { useCallback, useEffect, useRef, useState } from 'react';
import { command, desktopAvailable } from '../lib/desktop';
import { baseQuery, type BrowseRequest, type ConversationPage, type TimelinePage, type BrowseDirection } from '../lib/browse';

type TaskState<T> = { scope:string; data:T|null; loading:boolean; canceling:boolean; error:string };
export function useBrowseTask<T>(scope:string) {
  const empty:TaskState<T>={scope,data:null,loading:false,canceling:false,error:''};
  const [state,setState]=useState<TaskState<T>>(empty);
  const epoch=useRef(0),active=useRef<string|null>(null),currentScope=useRef(scope);
  currentScope.current=scope;
  const cancel=useCallback(()=>{
    const id=active.current,turn=epoch.current;
    if(id){setState(old=>({...old,canceling:true}));void command('cancel_browse',{requestId:id}).catch(e=>{if(epoch.current===turn)setState(old=>({...old,error:e instanceof Error?e.message:String(e)}));});}
  },[]);
  const run=useCallback(async(name:string,accountId:string,request:Omit<BrowseRequest,'request_id'>)=>{
    if(!desktopAvailable||currentScope.current!==scope)return null;
    const previous=active.current;
    if(previous)void command('cancel_browse',{requestId:previous}).catch(()=>{});
    const turn=++epoch.current,id=crypto.randomUUID();active.current=id;
    setState(old=>({scope,data:old.scope===scope?old.data:null,loading:true,canceling:false,error:''}));
    try {
      const result=await command<T>(name,{accountId,request:{...request,request_id:id}});
      if(epoch.current!==turn||currentScope.current!==scope)return null;
      setState({scope,data:result,loading:false,canceling:false,error:''});return result;
    }catch(e){if(epoch.current===turn&&currentScope.current===scope)setState(old=>({...old,error:e instanceof Error?e.message:String(e)}));return null;}
    finally {if(epoch.current===turn&&currentScope.current===scope){active.current=null;setState(old=>({...old,loading:false,canceling:false}));}}
  },[scope]);
  useEffect(()=>{
    ++epoch.current;setState({scope,data:null,loading:false,canceling:false,error:''});
    return ()=>{++epoch.current;const id=active.current;active.current=null;if(id)void command('cancel_browse',{requestId:id}).catch(()=>{});};
  },[scope]);
  const visible=state.scope===scope?state:empty;
  return {...visible,run,cancel};
}
export function useConversationList(accountId:string,sourceId:string|null,scope:string,search:string) {
  const task=useBrowseTask<ConversationPage>(`${scope}/list/${search}`);
  const load=useCallback((cursor:string|null=null)=>task.run('browse_conversations',accountId,{mode:'imported_snapshot',source_id:sourceId,direction:'latest',anchor_id:null,anchor_timestamp:null,query:{...baseQuery(),search:search||null,cursor,limit:50}}),[task.run,accountId,sourceId,search]);
  useEffect(()=>{void load();},[load,scope]);
  return {...task,load};
}
export function useMessages(accountId:string,conversationId:string,sourceId:string|null,scope:string,search:string) {
  const task=useBrowseTask<TimelinePage>(`${scope}/messages/${conversationId}/${search}`);
  const load=useCallback((direction:BrowseDirection='latest',cursor:string|null=null,anchorId:string|null=null,anchorTimestamp:number|null=null)=>task.run('browse_messages',accountId,{mode:'imported_snapshot',source_id:sourceId,direction,anchor_id:anchorId,anchor_timestamp:anchorTimestamp,query:{...baseQuery(),conversation_id:conversationId,search:search||null,cursor}}),[task.run,accountId,conversationId,sourceId,search]);
  useEffect(()=>{void load();},[load,scope]);
  return {...task,load};
}
