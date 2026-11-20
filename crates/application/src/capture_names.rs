//! Optional encrypted directory snapshot for readable archive exports.
use super::*;
use std::collections::{BTreeMap,BTreeSet};
#[derive(Clone,Default,Serialize,Deserialize)]
pub(crate) struct CaptureNames {
    pub captured_at:Option<String>,
    pub contacts:BTreeMap<String,ContactName>,
    pub groups:BTreeMap<String,BTreeMap<String,String>>,
    #[serde(default)]
    pub unavailable_groups:usize,
}
#[derive(Clone,Default,Serialize,Deserialize)]
pub(crate) struct ContactName {pub nickname:Option<String>,pub remark:Option<String>,pub avatar_url:Option<String>}
#[derive(Serialize)]
pub struct CaptureNamesSummary {pub contacts:usize,pub group_members:usize,pub captured_at:Option<String>,pub unavailable_groups:usize}
impl CaptureNames {
    fn summary(&self)->CaptureNamesSummary{CaptureNamesSummary{contacts:self.contacts.len(),group_members:self.groups.values().map(BTreeMap::len).sum(),captured_at:self.captured_at.clone(),unavailable_groups:self.unavailable_groups}}
    pub fn display(&self,conversation:&str,username:&str,preference:&str)->String{
        let contact=self.contacts.get(username);
        let nickname=contact.and_then(|n|n.nickname.as_deref()).filter(|s|!s.is_empty());
        let remark=contact.and_then(|n|n.remark.as_deref()).filter(|s|!s.is_empty());
        let group=self.groups.get(conversation).and_then(|g|g.get(username)).map(String::as_str).filter(|s|!s.is_empty());
        match preference{"remark"=>remark.or(group).or(nickname),"group_nickname"=>group.or(remark).or(nickname),"nickname"=>nickname.or(remark).or(group),_=>None}.unwrap_or(username).to_owned()
    }
}
impl Application {
    pub(crate) fn capture_names_local(&self,account:&str,id:&str)->AppResult<CaptureNames>{let(_,capture):(_,Capture)=self.open_document(account,id,"source_capture")?;Ok(capture.names)}
    pub fn capture_names_status_local(&self,account:&str,id:&str)->AppResult<CaptureNamesSummary>{Ok(self.capture_names_local(account,id)?.summary())}
    pub fn refresh_capture_names_local(&self,account:&str,id:&str,view:&str,request:&str)->AppResult<CaptureNamesSummary>{
        let _guard=self.history_guard.try_lock().map_err(|_|AppError::new("BUSY","已有来源操作正在执行"))?;
        let lease=self.browse_lease(request)?;
        let(header,mut capture):(_,Capture)=self.open_document(account,id,"source_capture")?;
        let(context,revision)=self.checked_send_context(account,"filehelper")?;
        if capture.login!=context.sender_id{return Err(AppError::new("CONNECTION_CHANGED","归档与当前登录账号不同"));}
        let key_revision=self.repository.key_revision(account)?;
        let mut names=CaptureNames::default();let mut cursor=None;
        loop{
            let page=self.read_contacts_with_lease(account,view,cursor.as_deref(),None,&lease)?;
            for mut row in page.rows{
                names.contacts.insert(std::mem::take(&mut row.anchor.username),ContactName{nickname:row.nickname.take(),remark:row.remark.take(),avatar_url:row.avatar_url.take()});
                if names.contacts.len()>50_000{return Err(AppError::new("DIRECTORY_EXPORT_LIMIT","联系人快照超过5万项"));}
            }
            cursor=page.next;if cursor.is_none(){break;}
        }
        let mut groups=BTreeSet::new();
        for page in 0..capture.pages{
            lease.check()?;
            let(_,batch):(_,CaptureBatch)=self.open_document(account,&format!("{id}:{page}"),"capture_page")?;
            for row in batch.rows{if row.username.ends_with("@chatroom"){groups.insert(row.username);}}
        }
        for group in groups{
            let mut cursor=None;let mut members=BTreeMap::new();
            loop{
                let page=match self.read_groups_with_lease(account,view,&group,cursor.as_deref(),&lease){
                    Ok(page)=>page,
                    Err(error) if error.code=="LIVE_GROUP_NOT_IN_CACHE"=>{members.clear();names.unavailable_groups+=1;break;}
                    Err(error)=>return Err(error),
                };
                for mut member in page.rows{if let(Some(username),Some(nickname))=(member.anchor.username.take(),member.group_nickname.take()){members.insert(username,nickname);}}
                cursor=page.next;if cursor.is_none(){break;}
            }
            names.groups.insert(group,members);
            if serde_json::to_vec(&names).map_err(|_|AppError::internal())?.len()>3*1024*1024{return Err(AppError::new("DIRECTORY_EXPORT_LIMIT","联系人快照超过3MiB"));}
        }
        lease.check()?;
        if serde_json::to_vec(&names).map_err(|_|AppError::internal())?.len()>3*1024*1024{return Err(AppError::new("DIRECTORY_EXPORT_LIMIT","联系人快照超过3MiB"));}
        self.with_live_source_parent(account,view,|_,_,_,_|Ok(()))?;
        names.captured_at=Some(chrono::Utc::now().to_rfc3339());
        let summary=names.summary();capture.names=names;
        let doc=self.seal_document(account,id,"source_capture",header.revision+1,&capture)?;
        self.repository.library_save(&[doc],Some(&DocumentConnection{revision,key_revision}))?;
        Ok(summary)
    }
}
