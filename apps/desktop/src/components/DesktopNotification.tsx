import {useEffect,useState} from 'react';
import {command} from '../lib/desktop';
export interface NotificationNavigation {account:string;conversation:string;position:number;page:string}
interface Notice {id:string;title:string;body:string}
export function DesktopNotification(){
  const [notice,setNotice]=useState<Notice|null>(null),[error,setError]=useState('');
  useEffect(()=>{let alive=true,pending=false;const read=async()=>{if(pending)return;pending=true;try{const n=await command<Notice|null>('desktop_notification');if(alive){setNotice(n);setError('');}}catch{if(alive)setNotice(null);}finally{pending=false;}};void read();const timer=setInterval(()=>void read(),500);return()=>{alive=false;clearInterval(timer);};},[]);
  async function act(name:string){if(!notice)return;try{await command(name,{id:notice.id});setNotice(null);}catch(e){setError(String(e));}}
  return <main className="desktop-notification" aria-live="polite">{notice&&<><button className="notification-open" onClick={()=>void act('activate_desktop_notification')}><strong>{notice.title}</strong><span>{notice.body}</span></button><button className="notification-close" aria-label="关闭通知" onClick={()=>void act('dismiss_desktop_notification')}>×</button></>}{error&&<p role="alert">{error}</p>}</main>;
}
