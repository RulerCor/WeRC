import {useEffect} from 'react';
import {motionAllowed} from '../lib/motion';

// 只观察本地管理列表；不克隆已删除节点，不保留聊天、令牌正文或密码输入。
export function useListMotion() {
  useEffect(()=>{
    const selector='.account-list,.audio-list,.operation-list,.history-sources';
    const tracked=new Map<Element,Map<Element,number>>();
    const animations=new Map<Element,Animation>();
    const main=document.querySelector('main');
    if(!main)return;
    const stop=(row:Element)=>{animations.get(row)?.cancel();animations.delete(row);};
    const reset=()=>{for(const row of animations.keys())stop(row);tracked.clear();};
    const update=(lists:Set<Element>)=>{
      if(!motionAllowed()){reset();return;}
      const style=getComputedStyle(document.documentElement);
      const duration=(name:string,fallback:number)=>{
        const value=Number.parseFloat(style.getPropertyValue(name));
        return Number.isFinite(value)&&value>=100&&value<=650?value:fallback;
      };
      const easing=style.getPropertyValue('--ef-snap').trim()||'cubic-bezier(0.16,1,0.3,1)';
      for(const list of lists) {
        if(!list.isConnected)continue;
        const before=tracked.get(list),rows=[...list.children];
        if(before&&rows.length===before.size&&rows.every(row=>before.has(row)))continue;
        const offsets=new Map<Element,number>();
        // 先保留当前视觉位移，再取消旧效果并测未变换布局，避免二次删除跳回旧起点。
        for(const row of rows)if(animations.has(row)) {
          offsets.set(row,new DOMMatrixReadOnly(getComputedStyle(row).transform).m42);
        }
        for(const row of rows)stop(row);
        const bounds=new Map(rows.map(row=>[row,row.getBoundingClientRect()]));
        const after=new Map<Element,number>();
        let index=0;
        for(const [row,rect] of bounds) {
          after.set(row,rect.top);
          const old=before?.get(row);
          let animation:Animation|undefined;
          if(before&&old===undefined&&index<6&&rect.bottom>0&&rect.top<innerHeight) {
            animation=row.animate([{opacity:0,transform:'translateY(4px)',clipPath:'inset(0 100% 0 0)'},{opacity:1,transform:'translateY(0)',clipPath:'inset(0)'}],{duration:duration('--motion-row',320),delay:index++*38,fill:'backwards',easing});
          } else if(old!==undefined&&before&&before.size>rows.length) {
            const offset=old+(offsets.get(row)??0)-rect.top;
            if(Math.abs(offset)>1)animation=row.animate([{transform:`translateY(${offset}px)`},{transform:'translateY(0)'}],{duration:duration('--motion-row-shift',200),easing});
          }
          if(animation) {
            animations.set(row,animation);
            const completed=()=>{if(animations.get(row)===animation)animations.delete(row);};
            animation.onfinish=completed;animation.oncancel=completed;
          }
        }
        tracked.set(list,after);
      }
      for(const list of tracked.keys())if(!list.isConnected)tracked.delete(list);
      for(const row of animations.keys())if(!row.isConnected)stop(row);
    };
    update(new Set(main.querySelectorAll(selector)));
    const observer=new MutationObserver(records=>{
      const dirty=new Set<Element>();
      for(const record of records) {
        if(record.target instanceof Element&&record.target.matches(selector))dirty.add(record.target);
        for(const node of record.addedNodes)if(node instanceof Element) {
          if(node.matches(selector))dirty.add(node);
          for(const list of node.querySelectorAll(selector))dirty.add(list);
        }
      }
      if(dirty.size||[...tracked.keys()].some(list=>!list.isConnected))update(dirty);
    });
    observer.observe(main,{childList:true,subtree:true});
    return ()=>{observer.disconnect();reset();};
  },[]);
}
