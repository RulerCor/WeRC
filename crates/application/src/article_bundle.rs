use crate::{Application,history::QueryLease};
use werc_domain::{AppError,AppResult,articles::Article};
use std::io::{Cursor,Write};
use zip::{ZipWriter,write::SimpleFileOptions};
fn escape(s:&str)->String{s.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;")}
fn file(zip:&mut ZipWriter<Cursor<Vec<u8>>>,path:&str,bytes:&[u8])->AppResult<()>{
    zip.start_file(path,SimpleFileOptions::default()).map_err(|_|AppError::internal())?;
    zip.write_all(bytes).map_err(|_|AppError::internal())
}
impl Application {
    pub(crate) fn article_bundle(&self,account:&str,rows:&[Article],lease:&QueryLease<'_>)->AppResult<(Vec<u8>,usize)>{
        let mut zip=ZipWriter::new(Cursor::new(Vec::new()));let mut total=0usize;let mut missing=0usize;let mut manifest=Vec::new();
        let mut index=String::from("<!doctype html><meta charset=utf-8><title>WeRC 文章目录</title><h1>文章目录</h1><ol>");
        for (i,a) in rows.iter().enumerate(){
            lease.check()?;let dir=format!("article-{:04}",i+1);
            let mut html=format!("<!doctype html><meta charset=utf-8><meta http-equiv=Content-Security-Policy content=\"default-src 'none'; img-src 'self'; style-src 'unsafe-inline'\"><title>{}</title><style>body{{max-width:70ch;margin:2em auto;padding:1em;font:18px/1.7 sans-serif}}pre{{white-space:pre-wrap;overflow-wrap:anywhere}}img{{max-width:100%;height:auto}}</style><h1>{}</h1><p>{} · {}</p><p>来源：{}</p><pre>{}</pre><h2>文章图片</h2>",escape(&a.title),escape(&a.title),escape(&a.author),escape(&a.published),escape(&a.url),escape(&a.text));
            let mut md=format!("# {}\n\n{} · {}\n\n来源：{}\n\n{}\n\n## 文章图片\n",a.title,a.author,a.published,a.url,a.text);
            for (j,url) in a.images.iter().enumerate(){
                lease.check()?;
                if let Some(m)=self.cached_article_image_local(account,url)?{
                    let (m,bytes)=self.article_image_contents_local(account,&m.id)?;total+=bytes.len();
                    if total>256*1024*1024{return Err(AppError::new("EXPORT_TOO_LARGE","离线图片超过256MiB，请分批导出"));}
                    let relative=format!("images/{:04}.{}",j+1,m.extension);file(&mut zip,&format!("{dir}/{relative}"),&bytes)?;
                    html+=&format!("<figure><img loading=lazy src=\"{relative}\" alt=\"图片 {}\"><figcaption>图片 {}</figcaption></figure>",j+1,j+1);
                    md+=&format!("\n![图片 {}]({relative})\n",j+1);
                    manifest.push(serde_json::json!({"article":i+1,"image":j+1,"path":format!("{dir}/{relative}"),"sha256":m.sha256,"size":m.size}));
                }else{missing+=1;html+=&format!("<p>图片 {} 未离线保存：{}</p>",j+1,escape(url));md+=&format!("\n图片 {} 未离线保存：{}\n",j+1,url);manifest.push(serde_json::json!({"article":i+1,"image":j+1,"missing":true,"url":url}));}
            }
            file(&mut zip,&format!("{dir}/article.html"),html.as_bytes())?;file(&mut zip,&format!("{dir}/article.md"),md.as_bytes())?;
            file(&mut zip,&format!("{dir}/article.json"),&serde_json::to_vec_pretty(a).map_err(|_|AppError::internal())?)?;
            index+=&format!("<li><a href=\"{dir}/article.html\">{}</a></li>",escape(&a.title));
        }
        index+="</ol>";file(&mut zip,"index.html",index.as_bytes())?;file(&mut zip,"images.json",&serde_json::to_vec_pretty(&manifest).map_err(|_|AppError::internal())?)?;
        lease.check()?;Ok((zip.finish().map_err(|_|AppError::internal())?.into_inner(),missing))
    }
}
