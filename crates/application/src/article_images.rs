//! Encrypted public article images, independent of native login credentials.
use crate::Application;
use serde::{Serialize,Deserialize};
use sha2::{Digest,Sha256};
use werc_domain::{AppError,AppResult,articles::*};
use zeroize::Zeroizing;
const CHUNK:usize=1024*1024;
#[derive(Clone,Serialize,Deserialize)]
pub struct ArticleImage {pub id:String,pub url:String,pub blob_id:String,pub chunks:usize,pub size:usize,pub sha256:String,pub mime:String,pub extension:String}
fn invalid()->AppError{AppError::new("DOCUMENT_CORRUPTED","文章图片副本完整性校验失败")}
fn key(url:&str)->String{format!("articleimage-{}",hex::encode(Sha256::digest(url)))}
impl Application {
    pub fn cached_article_image_local(&self,account:&str,url:&str)->AppResult<Option<ArticleImage>>{
        self.ensure_account(account)?;
        match self.open_document::<ArticleImage>(account,&key(url),"article_image"){
            Ok((_,m))=>{if m.url!=url{return Err(invalid());}return Ok(Some(m));},Err(e) if e.code=="NOT_FOUND"=>{},Err(e)=>return Err(e),
        }
        // Imported libraries use new document IDs but retain the public URL.
        let mut before=None;
        loop{let page=self.repository.library_list(account,"article_image",before,100)?;
            for h in page.data {let (_,m)=self.open_document::<ArticleImage>(account,&h.id,"article_image")?;if m.url==url{return Ok(Some(m));}}
            before=page.next_cursor;if before.is_none(){return Ok(None);}
        }
    }
    pub fn cache_article_image_local(&self,account:&str,id:&str,index:usize,request:&str)->AppResult<ArticleImage>{
        let article=self.article_local(account,id)?.record.saved.ok_or_else(||AppError::new("ARTICLE_SAVE_REQUIRED","请先保存文章，再保存离线图片"))?;
        let url=article.images.get(index).ok_or_else(||AppError::new("NOT_FOUND","文章图片不存在"))?;
        if let Some(m)=self.cached_article_image_local(account,url)?{return Ok(m);}
        image_url(url)?;let lease=self.browse_lease(request)?;
        let image=self.provider.fetch_article_image(account,url,&lease.cancel)?;lease.check()?;
        if image.bytes.is_empty()||image.bytes.len()>werc_domain::image::MAX_IMAGE_BYTES||!matches!((image.mime.as_str(),image.extension.as_str()),("image/png","png")|("image/gif","gif")|("image/webp","webp")){return Err(invalid());}
        let m=ArticleImage{id:key(url),url:url.clone(),blob_id:format!("articleblob-{}",uuid::Uuid::new_v4()),chunks:image.bytes.len().div_ceil(CHUNK),size:image.bytes.len(),sha256:hex::encode(Sha256::digest(&*image.bytes)),mime:image.mime,extension:image.extension};
        let mut docs=image.bytes.chunks(CHUNK).enumerate().map(|(index,b)|{
            lease.check()?;self.seal_document_unchecked(account,&format!("{}:{index}",m.blob_id),"media_chunk",1,&crate::source_capture::media::MediaChunk{blob_id:m.blob_id.clone(),index:index as u64,hex:hex::encode(b)})
        }).chain(std::iter::once_with(||{lease.check()?;self.seal_document_unchecked(account,&m.id,"article_image",1,&m)}));
        self.repository.library_import(account,&mut docs)?;drop(docs);Ok(m)
    }
    pub fn article_image_contents_local(&self,account:&str,id:&str)->AppResult<(ArticleImage,Zeroizing<Vec<u8>>)> {
        let (_,m)=self.open_document::<ArticleImage>(account,id,"article_image")?;
        if m.id!=id||m.size==0||m.size>werc_domain::image::MAX_IMAGE_BYTES||m.chunks!=m.size.div_ceil(CHUNK)||!matches!((m.mime.as_str(),m.extension.as_str()),("image/png","png")|("image/gif","gif")|("image/webp","webp")){return Err(invalid());}
        let mut bytes=Zeroizing::new(Vec::with_capacity(m.size));
        for index in 0..m.chunks {let (_,chunk)=self.open_document::<crate::source_capture::media::MediaChunk>(account,&format!("{}:{index}",m.blob_id),"media_chunk")?;
            if chunk.blob_id!=m.blob_id||chunk.index!=index as u64{return Err(invalid());}
            let data=Zeroizing::new(hex::decode(&chunk.hex).map_err(|_|invalid())?);if data.len()!=CHUNK.min(m.size-bytes.len()){return Err(invalid());}bytes.extend_from_slice(&data);
        }
        if hex::encode(Sha256::digest(&*bytes))!=m.sha256{return Err(invalid());}Ok((m,bytes))
    }
}
