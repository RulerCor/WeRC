import {useEffect,useState} from 'react';
import {command} from '../lib/desktop';
import {ContactAvatar} from './ContactAvatar';
export function CurrentProfile({accountId,viewId}:{accountId:string;viewId:string}){
  const [profile,setProfile]=useState<{username:string;nickname:string|null;avatar_available:boolean}|null>(null);
  useEffect(()=>{let alive=true;const requestId=crypto.randomUUID();setProfile(null);
    void command<{username:string;nickname:string|null;avatar_available:boolean}>('native_current_profile',{accountId,viewId,requestId}).then(value=>{if(alive)setProfile(value);},()=>{});
    return()=>{alive=false;void command('cancel_browse',{requestId}).catch(()=>{});};
  },[accountId,viewId]);
  if(!profile)return null;
  return <div className="actions" aria-label="当前微信身份"><ContactAvatar accountId={accountId} viewId={viewId} username={profile.username} name={profile.nickname||profile.username} available={profile.avatar_available}/><span>当前微信：<strong dir="auto">{profile.nickname||profile.username}</strong></span><details><summary>账号标识</summary><code>{profile.username}</code></details></div>;
}
