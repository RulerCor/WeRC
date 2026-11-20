//! Explicit local-only image key management with complete sample validation.
use crate::Application;
use serde::{Serialize,Deserialize};
use werc_domain::{AppError,AppResult,connection::ImageCacheKey,library::DocumentConnection};
use zeroize::{Zeroize,Zeroizing};
use sha2::{Digest,Sha256};
#[derive(Serialize,Deserialize)]
struct Saved {sender:String,aes_hex:Option<String>,xor:Option<u8>,sample_sha256:String,verified_at:String}
impl Drop for Saved{fn drop(&mut self){self.aes_hex.zeroize();self.xor.zeroize();}}
#[derive(Serialize)]
pub struct ImageKeyStatus {pub saved:bool,pub has_aes:bool,pub has_xor:bool,pub sample_sha256:Option<String>,pub verified_at:Option<String>}
#[derive(Serialize)]
pub struct ImageKeyReveal {pub aes_hex:Option<String>,pub xor:Option<u8>}
impl Drop for ImageKeyReveal{fn drop(&mut self){self.aes_hex.zeroize();self.xor.zeroize();}}
#[derive(Serialize)]
pub struct ImageSample {pub png_hex:String,pub sha256:String,pub status:ImageKeyStatus}
fn material(aes:Option<String>,xor:Option<u8>)->AppResult<ImageCacheKey>{
    let aes=Zeroizing::new(aes);
    let aes=aes.as_deref().filter(|s|!s.trim().is_empty()).map(|s|{
        if s.len()!=32||!s.bytes().all(|b|b.is_ascii_hexdigit()){return Err(AppError::new("INVALID_ARGUMENT","图片AES密钥需要32位十六进制字符"));}
        let bytes=Zeroizing::new(hex::decode(s).map_err(|_|AppError::internal())?);
        Ok(Zeroizing::new(bytes.as_slice().try_into().map_err(|_|AppError::internal())?))
    }).transpose()?;
    Ok(ImageCacheKey{aes,xor})
}
impl Saved {fn status(&self)->ImageKeyStatus{ImageKeyStatus{saved:true,has_aes:self.aes_hex.is_some(),has_xor:self.xor.is_some(),sample_sha256:Some(self.sample_sha256.clone()),verified_at:Some(self.verified_at.clone())}}}
impl Application{
    pub fn image_key_status_local(&self,account:&str)->AppResult<ImageKeyStatus>{
        self.ensure_account(account)?;
        match self.open_document::<Saved>(account,"local-image-keys","image_cache_keys"){
            Ok((_,saved))=>Ok(saved.status()),Err(e)if e.code=="NOT_FOUND"=>Ok(ImageKeyStatus{saved:false,has_aes:false,has_xor:false,sample_sha256:None,verified_at:None}),Err(e)=>Err(e)
        }
    }
    pub fn reveal_image_keys_local(&self,account:&str)->AppResult<ImageKeyReveal>{let(_,saved):(_,Saved)=self.open_document(account,"local-image-keys","image_cache_keys")?;Ok(ImageKeyReveal{aes_hex:saved.aes_hex.clone(),xor:saved.xor})}
    pub fn remove_image_keys_local(&self,account:&str)->AppResult<()>{
        self.ensure_account(account)?;
        match self.open_document::<Saved>(account,"local-image-keys","image_cache_keys"){
            Ok((h,_))=>self.repository.library_delete(account,&h.id,h.revision),Err(e)if e.code=="NOT_FOUND"=>Ok(()),Err(e)=>Err(e)
        }
    }
    pub fn verify_image_keys_local(&self,account:&str,sample:Vec<u8>,aes:Option<String>,xor:Option<u8>,automatic:bool,request:&str)->AppResult<ImageSample>{
        let sample=Zeroizing::new(sample);
        if sample.len()<4||sample.len()>8*1024*1024||sample.starts_with(b"\x89PNG")||sample.starts_with(b"\xff\xd8\xff")||sample.starts_with(b"GIF8")||sample.starts_with(b"wxgf")||sample.starts_with(b"RIFF"){return Err(AppError::new("IMAGE_SAMPLE_INVALID","请选择微信加密DAT缓存样本，不能用普通图片验证密钥"));}
        let lease=self.browse_lease(request)?;
        let(context,revision)=self.checked_send_context(account,"filehelper")?;
        let key_revision=self.repository.key_revision(account)?;
        let keys=if automatic{self.provider.recover_image_cache_key(account,&sample,&lease.cancel)?}else{material(aes,xor)?};
        let png=self.provider.decode_image_cache(&sample,&keys,&lease.cancel)?;
        let(info,_)=werc_domain::image::canonical_png(&png)?;let _=info;
        if !automatic&&keys.aes.is_some()&&!sample.starts_with(b"\x07\x08V2\x08\x07"){return Err(AppError::new("IMAGE_SAMPLE_INVALID","验证AES密钥需要V2缓存样本；旧版样本只验证XOR字节"));}
        // Recover the actual XOR byte from the provider's validated result.
        // Automatic providers return this explicitly; manual input must specify
        // XOR for a legacy sample instead of persisting a guessed setting.
        if !automatic&&keys.aes.is_none()&&keys.xor.is_none(){return Err(AppError::new("INVALID_ARGUMENT","请填写AES或XOR密钥，或使用自动恢复"));}
        lease.check()?;if self.checked_send_context(account,"filehelper")?.0!=context{return Err(AppError::new("CONNECTION_CHANGED","验证期间微信身份已变化"));}
        let previous=match self.open_document::<Saved>(account,"local-image-keys","image_cache_keys"){Ok((h,_))=>h.revision,Err(e)if e.code=="NOT_FOUND"=>0,Err(e)=>return Err(e)};
        let saved=Saved{sender:context.sender_id.clone(),aes_hex:keys.aes.as_ref().map(|k|hex::encode(&**k)),xor:keys.xor,sample_sha256:hex::encode(Sha256::digest(&*sample)),verified_at:chrono::Utc::now().to_rfc3339()};
        let status=saved.status();let doc=self.seal_document(account,"local-image-keys","image_cache_keys",previous+1,&saved)?;
        self.repository.library_save(&[doc],Some(&DocumentConnection{revision,key_revision}))?;
        Ok(ImageSample{png_hex:hex::encode(&*png),sha256:hex::encode(Sha256::digest(&*png)),status})
    }
    pub fn decode_cached_image_local(&self,account:&str,sample:Vec<u8>,request:&str)->AppResult<ImageSample>{
        let sample=Zeroizing::new(sample);let lease=self.browse_lease(request)?;
        let(context,_)=self.checked_send_context(account,"filehelper")?;
        let(_,saved):(_,Saved)=self.open_document(account,"local-image-keys","image_cache_keys")?;
        if saved.sender!=context.sender_id{return Err(AppError::new("CONNECTION_CHANGED","图片密钥属于其他登录身份，请重新验证"));}
        let png=self.provider.decode_image_cache(&sample,&material(saved.aes_hex.clone(),saved.xor)?,&lease.cancel)?;
        lease.check()?;if self.checked_send_context(account,"filehelper")?.0!=context{return Err(AppError::new("CONNECTION_CHANGED","解码期间微信身份已变化"));}
        Ok(ImageSample{png_hex:hex::encode(&*png),sha256:hex::encode(Sha256::digest(&*png)),status:saved.status()})
    }
}
