//! Read only source-bound article responses in this client's browser descendants.
//! No renderer buffers, cookies or authenticated URLs are retained or logged.
use std::{collections::HashSet, mem::{size_of,zeroed}, sync::atomic::{AtomicBool,Ordering}, time::{Duration,Instant}};
use windows_sys::Win32::{Foundation::*,System::{Diagnostics::{Debug::ReadProcessMemory,ToolHelp::*},Memory::*,Threading::*}};
use werc_domain::{AppError,AppResult,articles::*};
use zeroize::Zeroizing;
struct Handle(HANDLE);
impl Drop for Handle {fn drop(&mut self){unsafe{CloseHandle(self.0);}}}
fn unavailable()->AppError{AppError::new("ARTICLE_BODY_NOT_LOADED","微信尚未加载这篇文章正文，请稍后重试")}
fn check(cancel:&AtomicBool)->AppResult<()>{if cancel.load(Ordering::SeqCst){Err(AppError::new("QUERY_CANCELED","文章读取已取消"))}else{Ok(())}}
fn read(h:HANDLE,address:usize,length:usize)->Option<Zeroizing<Vec<u8>>>{
    let mut bytes=Zeroizing::new(vec![0;length]);let mut got=0;
    if unsafe{ReadProcessMemory(h,address as _,bytes.as_mut_ptr() as _,length,&mut got)}==0||got!=length{None}else{Some(bytes)}
}
#[derive(Clone,PartialEq,Eq)]
struct Process{pid:u32,parent:u32,start:u64,path:String}
fn identity(pid:u32)->Option<(u64,String)>{unsafe{
    let h=Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,0,pid));if h.0.is_null(){return None;}
    let mut start:FILETIME=zeroed();let mut end=start;let mut kernel=start;let mut user=start;
    let mut name=vec![0u16;32768];let mut n=name.len() as u32;
    if GetProcessTimes(h.0,&mut start,&mut end,&mut kernel,&mut user)==0||QueryFullProcessImageNameW(h.0,0,name.as_mut_ptr(),&mut n)==0{return None;}
    Some(((u64::from(start.dwHighDateTime)<<32)|u64::from(start.dwLowDateTime),String::from_utf16(&name[..n as usize]).ok()?))
}}
#[link(name="ntdll")]
unsafe extern "system" {fn NtQueryInformationProcess(process:HANDLE,class:u32,output:*mut std::ffi::c_void,length:u32,returned:*mut u32)->i32;}
fn is_article_renderer(pid:u32)->bool{unsafe{
    let h=Handle(OpenProcess(PROCESS_QUERY_INFORMATION|PROCESS_VM_READ,0,pid));if h.0.is_null(){return false;}
    let mut basic=[0usize;6];let mut returned=0;
    if NtQueryInformationProcess(h.0,0,basic.as_mut_ptr() as _,size_of::<[usize;6]>() as u32,&mut returned)<0||returned as usize!=size_of::<[usize;6]>(){return false;}
    let Some(params)=read(h.0,basic[1]+0x20,8)else{return false;};let address=u64::from_le_bytes(params[..8].try_into().unwrap()) as usize;
    let Some(command)=read(h.0,address+0x70,16)else{return false;};let length=u16::from_le_bytes(command[..2].try_into().unwrap()) as usize;let address=u64::from_le_bytes(command[8..16].try_into().unwrap()) as usize;
    if length==0||length>65534||length%2!=0{return false;}
    let Some(raw)=read(h.0,address,length)else{return false;};let text=Zeroizing::new(String::from_utf16_lossy(&raw.chunks_exact(2).map(|c|u16::from_le_bytes([c[0],c[1]])).collect::<Vec<_>>()));
    text.split_whitespace().any(|s|s=="--type=renderer")&&text.split_whitespace().any(|s|s=="--wmpf-render-type=6")
}}
fn children(main:u32)->AppResult<Vec<Process>>{unsafe{
    let snap=Handle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS,0));if snap.0==INVALID_HANDLE_VALUE{return Err(unavailable());}
    let mut entry:PROCESSENTRY32W=zeroed();entry.dwSize=size_of::<PROCESSENTRY32W>() as u32;let mut rows=Vec::new();
    let mut ok=Process32FirstW(snap.0,&mut entry);
    while ok!=0{let n=entry.szExeFile.iter().position(|v|*v==0).unwrap_or(entry.szExeFile.len());
        if String::from_utf16_lossy(&entry.szExeFile[..n]).eq_ignore_ascii_case("WeChatAppEx.exe"){
            rows.push((entry.th32ProcessID,entry.th32ParentProcessID));
        }ok=Process32NextW(snap.0,&mut entry);
    }
    let browsers:Vec<_>=rows.iter().filter(|(_,p)|*p==main).collect();if browsers.len()!=1{return Err(unavailable());}
    let browser=browsers[0].0;let (bs,bp)=identity(browser).ok_or_else(unavailable)?;
    let mut out=vec![Process{pid:browser,parent:main,start:bs,path:bp.clone()}];
    for (pid,parent) in rows{if parent==browser&&is_article_renderer(pid){if let Some((start,path))=identity(pid){if path.eq_ignore_ascii_case(&bp){out.push(Process{pid,parent,start,path});}}}}
    out[1..].sort_by_key(|p|std::cmp::Reverse(p.start));out.truncate(7);Ok(out)
}}
fn escape(s:&str)->String{s.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;")}
fn project(v:&serde_json::Value,url:&str,depth:usize)->Option<Article>{
    if depth>6{return None;}
    if v["base_resp"]["ret"].as_i64()==Some(0)&&v["user_name"].as_str().is_some_and(werc_domain::article_history::publisher_valid){
        let link=["ContentUrl","link","Url","url"].iter().find_map(|k|v[*k].as_str())?;
        let link=link.replace("&amp;","&").replacen("http://mp.weixin.qq.com/","https://mp.weixin.qq.com/",1);
        if canonical_url(&link).ok().as_deref()!=Some(url){return None;}
        let body=v["content_noencode"].as_str()?;
        let title=v["Title"].as_str().or_else(||v["title"].as_str())?;
        if body.is_empty()||body.len()>MAX_ARTICLE_HTML||body.contains('\0')||title.len()>4096{return None;}
        let html=Zeroizing::new(format!("<h1 id=activity-name>{}</h1><div id=js_content>{body}</div>",escape(title)));
        let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
        let mut article=parse_article(url,&html,"native_authenticated",now).ok()?;
        article.author=v["nick_name"].as_str().or_else(||v["nickname"].as_str()).unwrap_or("").chars().take(1024).collect();
        article.published=["ori_create_time","send_time","CreateTime","create_time","pub_time","publish_time"].iter().find_map(|k|v[*k].as_i64().map(|n|n.to_string())).unwrap_or_default();
        return article.valid().then_some(article);
    }
    if let Some(array)=v.as_array(){return array.iter().take(512).find_map(|v|project(v,url,depth+1));}
    if let Some(s)=v.as_str(){if s.len()<=2*1024*1024 {if let Ok(v)=serde_json::from_str(s){return project(&v,url,depth+1);}}}
    ["AppMsgContextList","Content","content","data","result","resp_json","jsapi_resp","response"].iter().find_map(|k|v.get(*k).and_then(|n|project(n,url,depth+1)))
}
// Parse bounded JSON spans containing a body marker; compare the exact raw span
// a second time before accepting any public fields from a concurrently live heap.
fn candidate(h:HANDLE,base:usize,raw:&[u8],utf16:bool,url:&str,cancel:&AtomicBool,until:Instant)->AppResult<Option<Article>>{
    let decoded=Zeroizing::new(if utf16{String::from_utf16_lossy(&raw.chunks_exact(2).map(|c|u16::from_le_bytes([c[0],c[1]])).collect::<Vec<_>>())}else{String::new()});
    let input=if utf16{decoded.as_bytes()}else{raw};
    for marker in memchr::memmem::find_iter(input,b"content_noencode").take(32){
        let starts=memchr::memchr_iter(b'{',&input[..marker]).collect::<Vec<_>>();
        for at in starts.into_iter().rev().take(192){check(cancel)?;if Instant::now()>=until{return Ok(None);}
            let mut stream=serde_json::Deserializer::from_slice(&input[at..]).into_iter::<serde_json::Value>();
            let Some(Ok(value))=stream.next()else{continue;};let length=stream.byte_offset();
            if at+length<=marker{continue;}
            if let Some(article)=project(&value,url,0){
                let (offset,encoded)=if utf16{let prefix=&decoded[..at];let span=&decoded[at..at+length];(prefix.encode_utf16().count()*2,Zeroizing::new(span.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>()))}else{(at,Zeroizing::new(input[at..at+length].to_vec()))};
                if raw.get(offset..offset+encoded.len())!=Some(encoded.as_slice()){continue;}
                if read(h,base+offset,encoded.len()).is_some_and(|b|b.as_slice()==encoded.as_slice()){return Ok(Some(article));}
            }
        }
    }Ok(None)
}
pub fn loaded(main:u32,url:&str,cancel:&AtomicBool,seconds:u64)->AppResult<Article>{
    check(cancel)?;let processes=children(main)?;let until=Instant::now()+Duration::from_secs(seconds);let mut spent=0usize;
    let needles=[b"content_noencode".to_vec(),"content_noencode".encode_utf16().flat_map(u16::to_le_bytes).collect()];
    for process in processes.iter().skip(1){
        let handle=Handle(unsafe{OpenProcess(PROCESS_QUERY_INFORMATION|PROCESS_VM_READ,0,process.pid)});if handle.0.is_null(){continue;}
        if identity(process.pid)!=Some((process.start,process.path.clone())){return Err(unavailable());}
        let mut regions=Vec::new();let mut at=0usize;
        loop{check(cancel)?;if Instant::now()>=until{return Err(unavailable());}let mut info:MEMORY_BASIC_INFORMATION=unsafe{zeroed()};
            if unsafe{VirtualQueryEx(handle.0,at as _,&mut info,size_of::<MEMORY_BASIC_INFORMATION>())}==0{break;}
            let base=info.BaseAddress as usize;let Some(end)=base.checked_add(info.RegionSize)else{break;};if end<=at{break;}at=end;
            if info.State==MEM_COMMIT&&info.Type==MEM_PRIVATE&&info.Protect&PAGE_GUARD==0&&matches!(info.Protect&0xff,PAGE_READWRITE|PAGE_WRITECOPY){regions.push((base,info.RegionSize));}
        }
        regions.sort_by_key(|(base,size)|((*size).min(1024*1024),*base));let mut seen=HashSet::new();let mut per_process=0usize;
        let samples=regions.iter().map(|&(b,s)|(b,b,s.min(65536))).chain(regions.iter().flat_map(|&(b,s)|(65536..s).step_by(1024*1024).map(move|o|(b,b+o-64,(s-o).min(1024*1024)+64))));
        for (region,address,length) in samples{check(cancel)?;if Instant::now()>=until||spent+length>256*1024*1024{return Err(unavailable());}if per_process>128*1024*1024{break;}
            spent+=length;per_process+=length;let Some(block)=read(handle.0,address,length)else{continue;};
            for (encoding,needle) in needles.iter().enumerate(){for hit in memchr::memmem::find_iter(&block,needle).take(16){
                let location=address+hit;if !seen.insert(location){continue;}
                let mut start=location.saturating_sub(16384).max(region);if encoding==1{start+=(location-start)%2;}
                let size=regions.iter().find(|(b,_)|*b==region).unwrap().1;let n=(region+size-start).min(2*1024*1024);
                if spent+n>256*1024*1024{return Err(unavailable());}spent+=n;
                if let Some(window)=read(handle.0,start,n){if let Some(article)=candidate(handle.0,start,&window,encoding==1,url,cancel,until)?{
                    if identity(process.pid)!=Some((process.start,process.path.clone()))||children(main)?!=processes{return Err(unavailable());}return Ok(article);
                }}
            }}
        }
    }Err(unavailable())
}

#[cfg(test)]
mod tests {
 use super::*;
 #[test]fn article_body_requires_exact_url_and_success(){
   let url="https://mp.weixin.qq.com/s?__biz=owned&idx=1&mid=12&sn=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
   let mut value=serde_json::json!({"base_resp":{"ret":0},"user_name":"gh_owned","title":"自动化测试","link":url,"content_noencode":"<p>真实正文</p><script>private</script>","cookie":"secret"});
   let a=project(&value,url,0).unwrap();assert!(a.text.contains("真实正文"));assert!(!a.text.contains("private"));assert!(!serde_json::to_string(&a).unwrap().contains("secret"));
   assert!(project(&value,&url.replace("mid=12","mid=13"),0).is_none());
   value["base_resp"]["ret"]=serde_json::json!(2);assert!(project(&value,url,0).is_none());
 }
}
