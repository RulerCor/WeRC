/** Decode one local frame for the editable poster. The backend validates MP4 independently. */
export async function videoCover(file: File): Promise<Uint8Array<ArrayBuffer>> {
  const url=URL.createObjectURL(file),video=document.createElement('video');
  video.muted=true;video.preload='auto';video.playsInline=true;
  try {
    await new Promise<void>((resolve,reject)=>{
      const timer=setTimeout(()=>finish(new Error('读取视频封面超时，请换一个H.264 MP4文件。')),15000);
      const finish=(error?:Error)=>{clearTimeout(timer);video.onloadeddata=null;video.onerror=null;error?reject(error):resolve();};
      video.onloadeddata=()=>finish();
      video.onerror=()=>finish(new Error('当前电脑无法读取此视频，请选择可播放的H.264 MP4文件。'));
      video.src=url;
    });
    if(!video.videoWidth||!video.videoHeight)throw new Error('视频缺少有效画面。');
    const scale=Math.min(1,1280/video.videoWidth,720/video.videoHeight);
    const canvas=document.createElement('canvas');
    canvas.width=Math.max(1,Math.round(video.videoWidth*scale));
    canvas.height=Math.max(1,Math.round(video.videoHeight*scale));
    const context=canvas.getContext('2d');if(!context)throw new Error('无法创建视频封面。');
    context.drawImage(video,0,0,canvas.width,canvas.height);
    const blob=await new Promise<Blob>((resolve,reject)=>canvas.toBlob(b=>b?resolve(b):reject(new Error('生成封面失败。')),'image/png'));
    return new Uint8Array(await blob.arrayBuffer());
  } finally {video.pause();video.removeAttribute('src');video.load();URL.revokeObjectURL(url);}
}
