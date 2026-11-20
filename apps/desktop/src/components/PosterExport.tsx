import {useEffect,useRef,useState} from 'react';
import {command} from '../lib/desktop';
import {renderPosters,posterPng,type PosterBlock,type PosterOptions} from '../lib/poster';
import {HelpTip} from './HelpTip';
export function PosterExport({accountId,title,blocks,label='生成图片',disabled=false}:{accountId:string;title:string;blocks:PosterBlock[];label?:string;disabled?:boolean}){
  const [theme,setTheme]=useState<PosterOptions['theme']>('paper'),[size,setSize]=useState(26),[heading,setHeading]=useState(title),[footer,setFooter]=useState(true),[hideNames,setHideNames]=useState(false);
  const [selected,setSelected]=useState<number[]>([]),[pages,setPages]=useState<string[]>([]),[busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const rendered=useRef<HTMLCanvasElement[]>([]),urls=useRef<string[]>([]),generation=useRef(0),working=useRef(false);
  function clear(){generation.current++;working.current=false;setBusy(false);for(const url of urls.current)URL.revokeObjectURL(url);urls.current=[];rendered.current=[];setPages([]);setNotice('');}
  useEffect(()=>{clear();setSelected([]);setHeading(title);return()=>{generation.current++;for(const url of urls.current)URL.revokeObjectURL(url);};},[accountId,title,blocks]);
  function change(){clear();setError('');}
  async function preview(){
    if(working.current)return;clear();working.current=true;setBusy(true);setError('');const expected=generation.current;
    const created:string[]=[];
    try{
      const chosen=blocks.filter((_,i)=>selected.includes(i)).map(b=>hideNames?{...b,heading:'聊天记录'}:b);
      const result=renderPosters(chosen,{title:heading||title,subtitle:`${chosen.length}项所选资料 · ${new Date().toLocaleDateString()}`,theme,fontSize:size,showFooter:footer});
      for(const page of result){const bytes=await posterPng(page);created.push(URL.createObjectURL(new Blob([bytes.slice().buffer],{type:'image/png'})));}
      if(expected!==generation.current){created.forEach(URL.revokeObjectURL);return;}rendered.current=result;urls.current=created;setPages(created);
    }catch(e){created.forEach(URL.revokeObjectURL);if(expected===generation.current)setError(String(e));}
    finally{if(expected===generation.current){working.current=false;setBusy(false);}}
  }
  async function save(){
    if(working.current||!rendered.current.length)return;
    working.current=true;setBusy(true);setError('');const expected=generation.current,canvases=[...rendered.current],stem=`WeRC-图片-${new Date().toISOString().replace(/[:.]/g,'-')}`;
    let saved=0;const paths:string[]=[];
    try{for(let i=0;i<canvases.length;i++){
      if(expected!==generation.current)break;
      const bytes=await posterPng(canvases[i]),path=`${stem}-${String(i+1).padStart(2,'0')}.png`;
      const output=await command<string>('save_visual_png',{accountId,path,bytes:Array.from(bytes)});paths.push(output);saved++;
    }if(expected===generation.current)setNotice(`已保存${saved}张PNG到系统下载目录。`);}
    catch(e){if(expected===generation.current)setError(`已保存${saved}张，其余未完成：${String(e)}`);}
    finally{if(expected===generation.current){working.current=false;setBusy(false);}}
  }
  return <details className="local-tools" onToggle={e=>{if(!e.currentTarget.open)clear();}}><summary>{label}</summary>
    <fieldset disabled={disabled||busy}><legend>选择内容</legend><div className="actions"><button type="button" className="button" onClick={()=>{change();setSelected(blocks.slice(0,100).map((_,i)=>i));}}>选择本页（最多100项）</button><button type="button" className="button" onClick={()=>{change();setSelected([]);}}>取消选择</button><span>{selected.length}项已选择</span><HelpTip label="图片内容说明">只绘制选中的可读文字和消息信息，不截取整个窗口，不包含附件或播放内容。长文字自动分成多张图片；隐藏名称只隐藏每条标题，正文中的姓名需自行核对。每页PNG为900×1800，保存到当前电脑的下载目录，同名不覆盖。图片不会自动发送。</HelpTip></div>
    <div style={{maxHeight:240,overflow:'auto'}}>{blocks.map((b,i)=><label className="permission-choice" key={i}><input type="checkbox" checked={selected.includes(i)} disabled={selected.length>=100&&!selected.includes(i)} onChange={e=>{change();setSelected(old=>e.target.checked?[...old,i]:old.filter(n=>n!==i));}}/><span style={{overflowWrap:'anywhere'}}>{b.heading} · {b.text.slice(0,90)||'[无可读文字]'}</span></label>)}</div></fieldset>
    <fieldset disabled={disabled||busy}><legend>图片样式</legend><label>标题<input value={heading} maxLength={160} onChange={e=>{change();setHeading(e.target.value);}}/></label><div className="actions"><label>主题<select value={theme} onChange={e=>{change();setTheme(e.target.value as PosterOptions['theme']);}}><option value="paper">纸白</option><option value="night">深夜</option><option value="mint">薄荷</option></select></label><label>字号<select value={size} onChange={e=>{change();setSize(Number(e.target.value));}}><option value={22}>紧凑</option><option value={26}>标准</option><option value={32}>大字</option></select></label><label><input type="checkbox" checked={hideNames} onChange={e=>{change();setHideNames(e.target.checked);}}/>隐藏逐条名称和时间</label><label><input type="checkbox" checked={footer} onChange={e=>{change();setFooter(e.target.checked);}}/>显示页码</label></div></fieldset>
    <div className="actions"><button className="button" disabled={disabled||busy||!selected.length} onClick={()=>void preview()}>{busy?'正在处理…':'预览图片'}</button>{pages.length>0&&<button className="button" disabled={disabled||busy} onClick={()=>void save()}>保存全部{pages.length}张PNG</button>}</div>
    {pages.length>0&&<div aria-label="图片导出预览" style={{display:'flex',gap:16,overflowX:'auto',maxHeight:480}}>{pages.map((url,i)=><img key={url} src={url} alt={`图片第${i+1}页`} style={{width:240,height:480,objectFit:'contain',flexShrink:0}}/>)}</div>}
    {notice&&<p role="status">{notice}</p>}{error&&<p role="alert" className="error-notice">{error}</p>}
  </details>;
}
