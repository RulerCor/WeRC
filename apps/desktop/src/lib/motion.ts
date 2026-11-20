import { useEffect, useState } from 'react';
export type MotionMode = 'full' | 'calm' | 'off';
const key = 'werc-motion-v1';
export function readMotion(): MotionMode {
  try { const value=localStorage.getItem(key); return value==='calm'||value==='off'?value:'full'; } catch {return 'full';}
}
export function motionAllowed() {
  return document.documentElement.dataset.input !== 'keyboard' && document.documentElement.dataset.motion !== 'off' && !window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}
export function useMotionPolicy() {
  const [mode,setMode]=useState<MotionMode>(readMotion);
  useEffect(()=>{
    document.documentElement.dataset.motion=mode;
    try {localStorage.setItem(key,mode);} catch { /* 设置不可写不阻塞界面。 */ }
    if(mode==='off') document.getAnimations().forEach(animation=>animation.cancel());
  },[mode]);
  useEffect(()=>{
    document.documentElement.dataset.startup='true';
    const startup=setTimeout(()=>{delete document.documentElement.dataset.startup;},1000);
    const media=matchMedia('(prefers-reduced-motion: reduce)');
    const stop=()=>document.getAnimations().forEach(animation=>animation.cancel());
    const pointer=()=>{document.documentElement.dataset.input='pointer';};
    const keyboard=()=>{document.documentElement.dataset.input='keyboard';stop();};
    const reduced=()=>{if(media.matches)stop();};
    const hidden=()=>{if(document.hidden)stop();};
    window.addEventListener('pointerdown',pointer,true);window.addEventListener('keydown',keyboard,true);
    media.addEventListener('change',reduced);document.addEventListener('visibilitychange',hidden);
    return ()=>{clearTimeout(startup);delete document.documentElement.dataset.startup;window.removeEventListener('pointerdown',pointer,true);window.removeEventListener('keydown',keyboard,true);media.removeEventListener('change',reduced);document.removeEventListener('visibilitychange',hidden);stop();};
  },[]);
  return {mode,setMode};
}

// 仅绘制纯色覆盖层，不截图页面，不复制令牌、密钥或聊天内容。
export function revealTheme(button:HTMLElement, apply:()=>void) {
  document.querySelectorAll('.theme-reveal').forEach(node=>node.remove());
  if(!motionAllowed() || document.querySelector('.secret-box, input[type="password"], .history-messages')) {apply();return;}
  const rect=button.getBoundingClientRect(),x=rect.x+rect.width/2,y=rect.y+rect.height/2;
  const cover=document.createElement('div');cover.className='theme-reveal';cover.setAttribute('aria-hidden','true');
  cover.style.background=getComputedStyle(document.documentElement).backgroundColor;
  document.body.append(cover);apply();
  const radius=Math.hypot(Math.max(x,innerWidth-x),Math.max(y,innerHeight-y));
  const animation=cover.animate([{clipPath:`circle(${radius}px at ${x}px ${y}px)`,opacity:1},{clipPath:`circle(0px at ${x}px ${y}px)`,opacity:0}],{duration:Math.max(240,Math.min(600,radius/3)),easing:'cubic-bezier(0.7,0,0.2,1)'});
  animation.onfinish=()=>cover.remove();animation.oncancel=()=>cover.remove();
}
