use super::*;
use std::sync::atomic::{AtomicBool,Ordering};
use werc_domain::article_history::*;
#[derive(serde::Serialize,serde::Deserialize)]
struct Cursor {offset:String,expires:u64}
fn canceled(cancel:&AtomicBool)->AppResult<()>{if cancel.load(Ordering::SeqCst){Err(AppError::new("QUERY_CANCELED","公众号读取已取消"))}else{Ok(())}}
impl NativeFileHelperProvider {
    pub fn article_history(&self,account:&str,publisher:&str,cursor:Option<&str>,cancel:&AtomicBool)->AppResult<ArticleHistoryPage>{
        self.require_feature(NativeFeature::ArticleHistory)?;
        if account!=self.account_id||!publisher_valid(publisher){return Err(AppError::new("INVALID_ARGUMENT","请选择当前账号的公众号"));}
        canceled(cancel)?;self.require_open()?;
        let now=SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_|AppError::internal())?.as_secs();
        let entropy=format!("WeRC/article-cursor/v1/{}/{}/{publisher}",self.account_id,self.context.epoch);
        let offset=if let Some(token)=cursor {
            if token.len()>40000{return Err(error("ARTICLE_CURSOR_INVALID"));}
            let bytes=hex::decode(token).map_err(|_|error("ARTICLE_CURSOR_INVALID"))?;
            let plain=Zeroizing::new(crate::WindowsWechatProvider.unprotect_key(&entropy,&bytes)?);
            let c:Cursor=serde_json::from_slice(&plain).map_err(|_|error("ARTICLE_CURSOR_INVALID"))?;
            if c.expires<=now||c.offset.is_empty()||c.offset.len()>8192||c.offset.contains('\0'){return Err(error("ARTICLE_CURSOR_EXPIRED"));}c.offset
        }else{String::new()};
        let mut random=[0u8;20];getrandom::fill(&mut random).map_err(|_|AppError::internal())?;
        let mut packet={let channel=self.transport.lock().map_err(|_|error("NATIVE_DISCONNECTED"))?;channel_binding(&channel,63)};
        packet.text(TARGET,128,publisher)?;packet.text(ATTEMPT,64,&format!("article-{}",hex::encode(&random[..16])))?;
        packet.text(DIGEST,65,&hex::encode(Sha256::digest(offset.as_bytes())))?;
        packet.put64(40,now+40);packet.put64(1520,offset.len() as u64);
        packet.put64(1512,u32::from_le_bytes(random[16..20].try_into().unwrap()).max(1) as u64);
        let exchange=|p:Packet|->AppResult<Packet>{let mut channel=self.transport.lock().map_err(|_|error("NATIVE_DISCONNECTED"))?;channel.check_identity()?;let op=p.u32(8);channel.exchange(p).map_err(|e|AppError::new(&e.code,&format!("文章读取步骤 {op}：{}",e.message)))};
        exchange(packet.copy(63))?;
        let result=(||{
            for (i,chunk) in offset.as_bytes().chunks(1024).enumerate(){canceled(cancel)?;let mut p=packet.copy(64);p.put64(1512,(i*1024) as u64);p.put32(1496,chunk.len() as u32);p.0[TEXT..TEXT+chunk.len()].copy_from_slice(chunk);exchange(p)?;}
            canceled(cancel)?;exchange(packet.copy(65))?;
            let until=Instant::now()+Duration::from_secs(32);
            let size=loop{canceled(cancel)?;let p=exchange(packet.copy(66))?;
                if p.u32(1492)==4 {if p.u32(1500)!=0{return Err(error("ARTICLE_NATIVE_RESULT_INVALID"));}break p.u64(1520) as usize;}
                if Instant::now()>=until{return Err(AppError::new("ARTICLE_HISTORY_TIMEOUT",&format!("公众号历史请求超时（阶段 {}，帧 {}，参数 {}，匹配 {}）",p.u32(1492),p.u32(1496),p.u64(1512)>>32,p.u64(1512)&0xffffffff)));}
                std::thread::sleep(Duration::from_millis(80));
            };
            if size==0||size>2*1024*1024{return Err(error("ARTICLE_NATIVE_RESULT_INVALID"));}
            let mut bytes=Zeroizing::new(Vec::with_capacity(size));
            while bytes.len()<size {canceled(cancel)?;let mut p=packet.copy(67);p.put64(1512,bytes.len() as u64);let p=exchange(p)?;let n=p.u32(1496) as usize;
                if n==0||n>1024||n>size-bytes.len()||p.u64(1520)!=size as u64{return Err(error("ARTICLE_NATIVE_RESULT_INVALID"));}bytes.extend_from_slice(&p.0[TEXT..TEXT+n]);}
            canceled(cancel)?;let (data,next,skipped)=project(&bytes,publisher)?;
            if next.as_deref()==Some(offset.as_str()){return Err(AppError::new("ARTICLE_CURSOR_STALLED","微信文章分页没有前进，请重新读取"));}
            let next_cursor=if let Some(next)=next {let plain=Zeroizing::new(serde_json::to_vec(&Cursor{offset:next,expires:now+1800}).map_err(|_|AppError::internal())?);Some(hex::encode(crate::WindowsWechatProvider.protect_key(&entropy,&plain)?))}else{None};
            Ok(ArticleHistoryPage{publisher:publisher.into(),data,end_reached:next_cursor.is_none(),next_cursor,skipped,source_mode:"native_authenticated".into()})
        })();
        let release=exchange(packet.copy(68));
        match result{Err(e)=>Err(e),Ok(page)=>{release?;Ok(page)}}
    }
}

impl NativeFileHelperProvider {
    pub fn article_body(&self,account:&str,value:&str,cancel:&AtomicBool)->AppResult<werc_domain::articles::Article>{
        if account!=self.account_id{return Err(error("ACCOUNT_MISMATCH"));}
        let url=werc_domain::articles::canonical_url(value)?;
        if !url.starts_with("https://mp.weixin.qq.com/s?"){return Err(AppError::new("ARTICLE_FULL_URL_REQUIRED","请从公众号历史列表选择文章，以取得完整公开链接"));}
        canceled(cancel)?;self.require_open()?;
        let pid={let mut c=self.transport.lock().map_err(|_|error("NATIVE_DISCONNECTED"))?;c.check_identity()?;c.binding.u32(16)};
        let body=crate::article_renderer::loaded(pid,&url,cancel,3).map_err(|e| {
            if e.code=="ARTICLE_BODY_NOT_LOADED" {AppError::new("ARTICLE_BODY_NOT_LOADED","当前文章正文未加载；后台模式不会自动打开文章窗口")}else{e}
        })?;
        self.transport.lock().map_err(|_|error("NATIVE_DISCONNECTED"))?.check_identity()?;
        Ok(body)
    }
}
