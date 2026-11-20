import type {ReactNode} from 'react';

export function SearchHighlight({text,needle}:{text:string;needle?:string}) {
  if(!needle)return <>{text}</>;
  const expression=new RegExp(needle.replace(/[.*+?^${}()|[\]\\]/g,'\\$&'),'giu');
  const parts:ReactNode[]=[];let position=0;
  for(const match of text.matchAll(expression)) {
    const index=match.index;
    parts.push(text.slice(position,index),<mark key={index}>{match[0]}</mark>);
    position=index+match[0].length;
  }
  parts.push(text.slice(position));
  return <>{parts}</>;
}
