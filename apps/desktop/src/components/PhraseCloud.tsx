export function PhraseCloud({phrases}:{phrases:[string,number][]}) {
  if(!phrases.length)return <p>所选范围没有重复短句。</p>;
  const maximum=Math.max(2,...phrases.map(([,count])=>count));
  return <ul aria-label="常用短句词云" style={{display:'flex',flexWrap:'wrap',alignItems:'baseline',gap:'12px 18px',listStyle:'none',padding:12}}>
    {phrases.map(([text,count])=><li key={text} title={`${count}次`} style={{fontSize:14+20*Math.log1p(count)/Math.log1p(maximum),overflowWrap:'anywhere',maxWidth:'100%',lineHeight:1.4}}><span>{text}</span><small style={{fontSize:12,marginInlineStart:4}}>{count}</small></li>)}
  </ul>;
}
