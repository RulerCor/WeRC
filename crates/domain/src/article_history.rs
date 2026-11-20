//! Public metadata projected from one attributed native publisher history reply.
use crate::{AppError,AppResult,articles::canonical_url};
use serde::{Deserialize,Serialize};
use serde_json::Value;
#[derive(Clone,Serialize,Deserialize)]
pub struct ArticleSummary {pub url:String,pub title:String,pub published_at:Option<i64>}
#[derive(Clone,Serialize,Deserialize)]
pub struct ArticleHistoryPage {
    pub publisher:String,pub data:Vec<ArticleSummary>,pub next_cursor:Option<String>,
    pub end_reached:bool,pub skipped:usize,pub source_mode:String,
}
pub fn publisher_valid(s:&str)->bool{s.starts_with("gh_")&&s.len()>3&&s.len()<128&&s.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-'|b'.'))}
pub fn project(raw:&[u8],publisher:&str)->AppResult<(Vec<ArticleSummary>,Option<String>,usize)> {
    let invalid=||AppError::new("ARTICLE_HISTORY_INVALID","微信文章列表格式或来源不符，已丢弃本页");
    if raw.len()>2*1024*1024||!publisher_valid(publisher){return Err(invalid());}
    let outer:Value=serde_json::from_slice(raw).map_err(|_|invalid())?;
    if !outer["err_msg"].as_str().is_some_and(|s|s.eq_ignore_ascii_case("H5ExtTransfer:ok")){return Err(AppError::new("ARTICLE_HISTORY_FAILED","微信未能读取公众号历史"));}
    let body=match &outer["jsapi_resp"]["resp_json"]{Value::String(s)=>serde_json::from_str::<Value>(s).map_err(|_|invalid())?,v=>v.clone()};
    if body["BaseResponse"]["Ret"].as_i64()!=Some(0){return Err(AppError::new("ARTICLE_HISTORY_SERVER_ERROR",&format!("公众号历史服务未返回成功（状态 {:?}）",body["BaseResponse"]["Ret"].as_i64())));}
    if body["AccountInfo"]["UserName"].as_str()!=Some(publisher){return Err(AppError::new("ARTICLE_HISTORY_PUBLISHER_MISMATCH",&format!("公众号来源未通过核对（账号字段 {:?}）",body["AccountInfo"].as_object().map(|m|m.keys().collect::<Vec<_>>()))));}
    let messages=body["MsgList"]["Msg"].as_array().filter(|v|v.len()<=128).ok_or_else(invalid)?;
    let paging=&body["MsgList"]["PagingInfo"];
    let offset=match paging["IsEnd"].as_u64(){
        Some(1)=>None,Some(0)=>Some(paging["Offset"].as_str().filter(|s|!s.is_empty()&&s.len()<=8192&&!s.contains('\0')).ok_or_else(invalid)?.into()),_=>return Err(invalid()),
    };
    let mut rows=Vec::new();let mut seen=std::collections::HashSet::new();let mut skipped=0;
    for message in messages {
        if message.get("AppMsg").is_none(){skipped+=1;continue;}
        let app=&message["AppMsg"];
        let details=app["DetailInfo"].as_array().filter(|v|v.len()<=64).ok_or_else(invalid)?;
        for item in details {
            let field=|name:&str|item.get(name).or_else(||app["BaseInfo"].get(name));
            let Some(value)=field("ContentUrl").and_then(Value::as_str) else{skipped+=1;continue;};
            let decoded=value.replace("&amp;","&");
            let https=decoded.strip_prefix("http://mp.weixin.qq.com/").map(|s|format!("https://mp.weixin.qq.com/{s}")).unwrap_or(decoded);
            let Ok(url)=canonical_url(&https) else{skipped+=1;continue;};
            let Some(title)=field("Title").and_then(Value::as_str).filter(|s|!s.trim().is_empty()&&s.len()<=4096) else{skipped+=1;continue;};
            let published_at=field("CreateTime").or_else(||message["BaseInfo"].get("CreateTime")).and_then(Value::as_i64).filter(|t|*t>0);
            if seen.insert(url.clone()){rows.push(ArticleSummary{url,title:title.into(),published_at});}
        }
    }
    Ok((rows,offset,skipped))
}
#[cfg(test)]
mod tests {
 use super::*;
 #[test]fn native_history_projects_only_public_fields(){
  let url=format!("http://mp.weixin.qq.com/s?__biz=Owned&mid=123&idx=1&sn={}&key=private", "a".repeat(32));
  let body=serde_json::json!({"BaseResponse":{"Ret":0},"AccountInfo":{"UserName":"gh_owned","Secret":"private"},"MsgList":{"Msg":[{"AppMsg":{"DetailInfo":[{"Title":"自动化测试","ContentUrl":url,"CreateTime":1}]}}],"PagingInfo":{"IsEnd":0,"Offset":"owned-next"}}});
  let bytes=serde_json::to_vec(&serde_json::json!({"err_msg":"H5ExtTransfer:ok","jsapi_resp":{"resp_json":body.to_string()}})).unwrap();
  let (rows,next,skipped)=project(&bytes,"gh_owned").unwrap();
  assert_eq!(rows.len(),1);assert_eq!(next.as_deref(),Some("owned-next"));assert_eq!(skipped,0);
  assert!(!serde_json::to_string(&rows).unwrap().contains("private"));
  assert_eq!(project(&bytes,"gh_other").err().unwrap().code,"ARTICLE_HISTORY_PUBLISHER_MISMATCH");
 }
}
