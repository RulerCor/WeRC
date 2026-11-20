// Presentation hint only; the backend independently validates the full descriptor.
export function isEmoticon(type:string|number|undefined,metadata?:string|null):boolean {
  let value:bigint;try{value=BigInt(type??0)&0xffffffffn;}catch{return false;}
  if(value===47n)return true;
  if(value!==49n||!metadata||metadata.length>65536||/<!DOCTYPE/i.test(metadata))return false;
  try{
    const xml=new DOMParser().parseFromString(metadata,'application/xml');
    if(xml.querySelector('parsererror')||xml.documentElement.tagName!=='msg')return false;
    const apps=Array.from(xml.documentElement.children).filter(n=>n.tagName==='appmsg');if(apps.length!==1)return false;
    const kinds=Array.from(apps[0].children).filter(n=>n.tagName==='type');
    return kinds.length===1&&kinds[0].children.length===0&&kinds[0].textContent?.trim()==='8';
  }catch{return false;}
}
export function emoticonMime(bytes:Uint8Array):'image/gif'|'image/png' {
  const png=[137,80,78,71,13,10,26,10];
  if(bytes.length>0&&bytes.length<=32*1024*1024&&png.every((v,i)=>bytes[i]===v))return 'image/png';
  if(bytes.length>0&&bytes.length<=1024*1024&&new TextDecoder().decode(bytes.subarray(0,6)).match(/^GIF8[79]a$/))return 'image/gif';
  throw new Error('表情响应格式或大小无效。');
}
