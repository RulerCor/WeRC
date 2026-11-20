export interface PosterBlock {heading:string;text:string}
export interface PosterOptions {title:string;subtitle:string;theme:'paper'|'night'|'mint';fontSize:number;showFooter:boolean}
const palettes={paper:{background:'#f5f1e9',card:'#ffffff',text:'#242722',muted:'#64675e',accent:'#345846'},night:{background:'#151c24',card:'#232e3b',text:'#f3f5f7',muted:'#b9c4d0',accent:'#bce3b1'},mint:{background:'#e7f2ea',card:'#ffffff',text:'#163f30',muted:'#526b5e',accent:'#287451'}};
/** Draw only selected text. Canvas has no remote images, HTML or screenshot access. */
export function renderPosters(blocks:PosterBlock[],options:PosterOptions):HTMLCanvasElement[]{
  if(!blocks.length||blocks.length>100)throw new Error('请选择1—100条内容。');
  if(blocks.reduce((n,b)=>n+b.text.length+b.heading.length,0)>80000)throw new Error('本次海报文字超过8万字符，请减少选择。');
  const width=900,height=1800,pad=56,contentWidth=width-pad*2,fontSize=Math.max(18,Math.min(34,options.fontSize));
  const palette=palettes[options.theme],pages:HTMLCanvasElement[]=[];
  let canvas!:HTMLCanvasElement,ctx!:CanvasRenderingContext2D,y=0;
  function wrap(value:string,font:string,maxWidth:number){
    ctx.font=font;const lines:string[]=[];
    for(const paragraph of value.replace(/\r\n?/g,'\n').split('\n')){
      let line='';for(const char of paragraph){if(line&&ctx.measureText(line+char).width>maxWidth){lines.push(line);line='';}line+=char;}lines.push(line);
    }
    return lines;
  }
  function start(){
    canvas=document.createElement('canvas');canvas.width=width;canvas.height=height;
    const context=canvas.getContext('2d');if(!context)throw new Error('当前系统无法绘制海报。');ctx=context;
    ctx.fillStyle=palette.background;ctx.fillRect(0,0,width,height);ctx.textBaseline='top';
    y=pad;ctx.fillStyle=palette.accent;ctx.font='bold 34px system-ui, sans-serif';
    // Long titles wrap; the subtitle remains separate from message content.
    for(const line of wrap(options.title.slice(0,160),'bold 34px system-ui, sans-serif',contentWidth)){ctx.fillText(line,pad,y);y+=44;}
    ctx.fillStyle=palette.muted;
    for(const line of wrap(options.subtitle.slice(0,240),'20px system-ui, sans-serif',contentWidth)){ctx.fillText(line,pad,y);y+=28;}
    y+=28;pages.push(canvas);
    if(pages.length>30)throw new Error('海报超过30页，请减少选择或字号。');
  }
  function finish(){
    if(options.showFooter){ctx.fillStyle=palette.muted;ctx.font='18px system-ui, sans-serif';ctx.fillText(`WeRC · 本地资料海报 · 第 ${pages.length} 页`,pad,height-42);}
  }
  start();
  for(const block of blocks){
    const heading=wrap(block.heading.slice(0,512),`bold ${fontSize-2}px system-ui, sans-serif`,contentWidth-40);
    const lines=wrap(block.text||'[未取得可读文字]',`${fontSize}px system-ui, sans-serif`,contentWidth-40);
    let offset=0;
    do{
      const headerHeight=heading.length*(fontSize+6)+16,lineHeight=fontSize+12;
      if(y+headerHeight+lineHeight+48>height-76){finish();start();}
      const count=Math.max(1,Math.floor((height-76-y-headerHeight-36)/lineHeight));
      const chunk=lines.slice(offset,offset+count),boxHeight=headerHeight+chunk.length*lineHeight+28;
      ctx.fillStyle=palette.card;ctx.fillRect(pad,y,contentWidth,boxHeight);let at=y+16;
      ctx.fillStyle=palette.muted;ctx.font=`bold ${fontSize-2}px system-ui, sans-serif`;
      for(const line of heading){ctx.fillText(line,pad+20,at);at+=fontSize+6;}
      at+=12;ctx.font=`${fontSize}px system-ui, sans-serif`;ctx.fillStyle=palette.text;
      for(const line of chunk){ctx.fillText(line,pad+20,at);at+=lineHeight;}
      offset+=chunk.length;y+=boxHeight+20;
      if(offset<lines.length){finish();start();}
    }while(offset<lines.length);
  }
  finish();return pages;
}
export async function posterPng(canvas:HTMLCanvasElement):Promise<Uint8Array>{
  const blob=await new Promise<Blob>((resolve,reject)=>canvas.toBlob(b=>b?resolve(b):reject(new Error('无法生成PNG图片。')),'image/png'));
  return new Uint8Array(await blob.arrayBuffer());
}
