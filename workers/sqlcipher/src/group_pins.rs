//! ChatroomDetailInfoExtData(1) -> TopMsgInfo(1 repeated) -> TopMsgInfoItem(3).
//! Only message identity is projected. Missing/unknown cache is not an empty list.
use crate::group_protocol::LiveGroupPin;

enum Value<'a> { Integer(u64), Bytes(&'a [u8]), Other }
fn varint(input: &[u8], at: &mut usize) -> Option<u64> {
    let mut value=0u64;
    for shift in (0..70).step_by(7) {
        let byte=*input.get(*at)?; *at+=1;
        if shift==63 && byte>1 {return None;}
        value|=u64::from(byte&127)<<shift;
        if byte&128==0 {return Some(value);}
    }
    None
}
fn fields(input: &[u8]) -> Option<Vec<(u32,Value<'_>)>> {
    let mut at=0; let mut out=Vec::new();
    while at<input.len() {
        if out.len()>=4096 {return None;}
        let tag=varint(input,&mut at)?;
        let field=u32::try_from(tag>>3).ok()?;
        if field==0 || field>0x1fffffff {return None;}
        let value=match tag&7 {
            0=>Value::Integer(varint(input,&mut at)?),
            1|5=>{at=at.checked_add(if tag&7==1 {8}else{4})?;Value::Other},
            2=>{let len=usize::try_from(varint(input,&mut at)?).ok()?;let end=at.checked_add(len)?;let bytes=input.get(at..end)?;at=end;Value::Bytes(bytes)},
            _=>return None,
        };
        if at>input.len() {return None;}
        out.push((field,value));
    }
    Some(out)
}
pub(super) fn decode(input: &[u8]) -> Option<Vec<LiveGroupPin>> {
    if input.len()>2*1024*1024 {return None;}
    let mut top=None;
    for (field,value) in fields(input)? {
        if field==1 {
            let Value::Bytes(bytes)=value else {return None;};
            if top.replace(bytes).is_some() {return None;}
        }
    }
    let mut result=Vec::new();let mut ids=std::collections::BTreeSet::new();
    for (field,value) in fields(top?)? {
        if field!=1 {continue;}
        if result.len()>=64 {return None;}
        let Value::Bytes(bytes)=value else {return None;};
        let mut id=None;
        for (field,value) in fields(bytes)? {
            if field==3 {
                let Value::Integer(number)=value else {return None;};
                if number==0 || id.replace(number).is_some() {return None;}
            }
        }
        let id=id?;
        if !ids.insert(id) {return None;}
        result.push(LiveGroupPin{server_id:id.to_string()});
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture(ids: &[u8]) -> Vec<u8> {
        let mut top=Vec::new();
        for id in ids {top.extend([10,2,24,*id]);}
        let mut out=vec![10,top.len() as u8];out.extend(top);out
    }
    #[test]
    fn distinguishes_missing_from_explicit_empty_and_retains_exact_unsigned_id() {
        assert!(decode(&[]).is_none());
        assert!(decode(&[16,1]).is_none());
        assert!(decode(&[10,0]).unwrap().is_empty());
        let pins=decode(&fixture(&[41,42])).unwrap();
        assert_eq!(pins.iter().map(|p|p.server_id.as_str()).collect::<Vec<_>>(),["41","42"]);
        // The maximum u64 varint occupies ten bytes.
        let mut big=vec![10,13,10,11,24];big.extend([255;9]);big.push(1);
        assert_eq!(decode(&big).unwrap()[0].server_id,u64::MAX.to_string());
    }
    #[test]
    fn rejects_malformed_and_ambiguous_cache_without_claiming_no_pins() {
        for data in [vec![10,1],vec![10,0,10,0],fixture(&[0]),fixture(&[1,1]),vec![10,2,10,0],vec![10,4,10,2,26,0],vec![15,0],vec![10,10,10,8,24,128,128,128,128,128,128,128]] {
            assert!(decode(&data).is_none());
        }
        let mut data=fixture(&[1]);data.extend([16,1]);assert!(decode(&data).is_some());
    }
}
