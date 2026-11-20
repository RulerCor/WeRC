//! WeChat local DAT decoding. Never changes the source file.
use aes::{Aes128,cipher::{BlockDecrypt,KeyInit}};
use werc_domain::{AppError,AppResult,connection::ImageCacheKey};
use zeroize::Zeroizing;
use std::sync::atomic::{AtomicBool,Ordering};
const V1:&[u8]=b"\x07\x08V1\x08\x07";
const V2:&[u8]=b"\x07\x08V2\x08\x07";
fn invalid()->AppError{AppError::new("IMAGE_SAMPLE_INVALID","图片样本、密钥或缓存布局无效；未保存密钥")}
pub(crate) fn check(cancel:&AtomicBool)->AppResult<()>{if cancel.load(Ordering::SeqCst){Err(AppError::new("QUERY_CANCELED","图片密钥操作已取消"))}else{Ok(())}}
fn header(bytes:&[u8])->bool{bytes.starts_with(b"\xff\xd8\xff")||bytes.starts_with(b"\x89PNG\r\n\x1a\n")||bytes.starts_with(b"GIF8")||bytes.starts_with(b"wxgf")||bytes.starts_with(b"RIFF")}
fn layout(bytes:&[u8])->AppResult<(usize,usize,usize)>{
    if bytes.len()<31||bytes.len()>8*1024*1024{return Err(invalid());}
    let aes=u32::from_le_bytes(bytes[6..10].try_into().map_err(|_|invalid())?)as usize;
    let xor=u32::from_le_bytes(bytes[10..14].try_into().map_err(|_|invalid())?)as usize;
    // A complete PKCS#7 block is present even when the prefix is aligned.
    let aligned=aes.checked_div(16).and_then(|v|v.checked_add(1)).and_then(|v|v.checked_mul(16)).ok_or_else(invalid)?;
    if aes==0||aligned>bytes.len()-15||xor>bytes.len()-15-aligned{return Err(invalid());}Ok((aes,aligned,xor))
}
pub(crate) fn first_block_matches(bytes:&[u8],key:&[u8;16])->bool{
    if layout(bytes).is_err(){return false;}
    let cipher=Aes128::new(key.into());let mut block=aes::cipher::Block::<Aes128>::clone_from_slice(&bytes[15..31]);cipher.decrypt_block(&mut block);header(&block)
}
pub(crate) fn decode_bytes(bytes:&[u8],key:&ImageCacheKey)->AppResult<(Zeroizing<Vec<u8>>,Option<u8>)>{
    if bytes.len()>8*1024*1024||bytes.len()<4{return Err(invalid());}
    if header(bytes){return Ok((Zeroizing::new(bytes.to_vec()),None));}
    if bytes.starts_with(V1)||bytes.starts_with(V2){
        let(aes_len,aligned,xor_len)=layout(bytes)?;
        let fixed=*b"cfcd208495d565ef";
        let secret=if bytes.starts_with(V1){&fixed}else{key.aes.as_deref().ok_or_else(||AppError::new("IMAGE_KEY_REQUIRED","此V2图片需要AES密钥"))?};
        let cipher=Aes128::new(secret.into());let mut out=Zeroizing::new(bytes[15..15+aligned].to_vec());
        for chunk in out.chunks_exact_mut(16){cipher.decrypt_block(aes::cipher::Block::<Aes128>::from_mut_slice(chunk));}
        let padding=aligned-aes_len;
        if padding==0||padding>16||out[aes_len..].iter().any(|b|*b as usize!=padding)||!header(&out){return Err(invalid());}
        out.truncate(aes_len);let middle_end=bytes.len()-xor_len;out.extend_from_slice(&bytes[15+aligned..middle_end]);
        let xor=if xor_len==0{None}else if let Some(value)=key.xor{Some(value)}else if out.starts_with(b"\xff\xd8\xff")&&xor_len>=2{
            let value=bytes[bytes.len()-1]^0xd9;if bytes[bytes.len()-2]^value!=0xff{return Err(invalid());}Some(value)
        }else if out.starts_with(b"\x89PNG")&&xor_len>=12{
            let footer=b"\0\0\0\0IEND\xaeB`\x82";let tail=&bytes[bytes.len()-12..];let value=tail[0]^footer[0];if !tail.iter().zip(footer).all(|(a,b)|a^value==*b){return Err(invalid());}Some(value)
        }else{return Err(AppError::new("IMAGE_XOR_REQUIRED","此图片需要手工填写XOR字节或选择完整JPEG/PNG样本"));};
        out.extend(bytes[middle_end..].iter().map(|b|b^xor.unwrap_or(0)));return Ok((out,xor));
    }
    if let Some(xor)=key.xor{let out=Zeroizing::new(bytes.iter().map(|b|b^xor).collect::<Vec<_>>());if header(&out){return Ok((out,Some(xor)));}return Err(invalid());}
    for xor in 0..=255u8{let prefix=bytes.iter().take(12).map(|b|b^xor).collect::<Vec<_>>();if header(&prefix){return Ok((Zeroizing::new(bytes.iter().map(|b|b^xor).collect()),Some(xor)));}}
    Err(invalid())
}
pub fn decode(sample:&[u8],key:&ImageCacheKey,cancel:&AtomicBool)->AppResult<Zeroizing<Vec<u8>>>{
    check(cancel)?;let(bytes,_)=decode_bytes(sample,key)?;
    let output=if bytes.starts_with(b"wxgf")||bytes.starts_with(b"GIF8"){
        #[cfg(windows)]{crate::emoticon_codec::render(&bytes,cancel)?}
        #[cfg(not(windows))]{return Err(AppError::new("PLATFORM_UNSUPPORTED","此缓存格式需要Windows图像组件"));}
    }else{werc_domain::image::canonical_received_png(&bytes)?.1};
    check(cancel)?;Ok(output)
}
#[cfg(test)]
pub(crate) mod tests{
    use super::*;
    use aes::cipher::BlockEncrypt;
    pub(crate) const KEY:[u8;16]=*b"qZ7mN0pL2xT5vB9r";
    pub(crate) fn encrypted_fixture()->Vec<u8>{
        let png=include_bytes!("../../../tests/fixtures/owned-image.png");let prefix=32usize;let suffix=12usize;let mut data=V2.to_vec();data.extend_from_slice(&(prefix as u32).to_le_bytes());data.extend_from_slice(&(suffix as u32).to_le_bytes());data.push(0);
        let cipher=Aes128::new((&KEY).into());let mut first=png[..prefix].to_vec();first.extend([16;16]);for block in first.chunks_exact_mut(16){cipher.encrypt_block(aes::cipher::Block::<Aes128>::from_mut_slice(block));}data.extend(first);data.extend_from_slice(&png[prefix..png.len()-suffix]);data.extend(png[png.len()-suffix..].iter().map(|b|b^0xa5));data
    }
    #[test]fn dat_v2_checks_padding_full_image_and_tail_key(){
        let sample=encrypted_fixture();let key=ImageCacheKey{aes:Some(Zeroizing::new(KEY)),xor:None};let(raw,xor)=decode_bytes(&sample,&key).unwrap();assert_eq!(xor,Some(0xa5));assert_eq!(raw.as_slice(),include_bytes!("../../../tests/fixtures/owned-image.png"));let png=decode(&sample,&key,&AtomicBool::new(false)).unwrap();assert!(png.starts_with(b"\x89PNG"));
        assert!(decode(&sample,&ImageCacheKey{aes:Some(Zeroizing::new([3;16])),xor:None},&AtomicBool::new(false)).is_err());
        assert!(decode(&sample,&ImageCacheKey{aes:Some(Zeroizing::new(KEY)),xor:Some(0xa4)},&AtomicBool::new(false)).is_err());
        assert_eq!(decode(&sample,&key,&AtomicBool::new(true)).err().unwrap().code,"QUERY_CANCELED");
        let mut broken=sample.clone();broken[10..14].copy_from_slice(&u32::MAX.to_le_bytes());assert!(decode_bytes(&broken,&key).is_err());
        let mut broken=sample.clone();broken[15+47]^=1;assert!(decode_bytes(&broken,&key).is_err());
        assert!(decode(&sample[..sample.len()-1],&key,&AtomicBool::new(false)).is_err());
    }
    #[test]fn legacy_xor_is_validated_with_complete_image_decode(){
        let png=include_bytes!("../../../tests/fixtures/owned-image.png");let encrypted=png.iter().map(|b|b^0x42).collect::<Vec<_>>();let key=ImageCacheKey{aes:None,xor:None};let(_,xor)=decode_bytes(&encrypted,&key).unwrap();assert_eq!(xor,Some(0x42));decode(&encrypted,&key,&AtomicBool::new(false)).unwrap();assert!(decode(&encrypted,&ImageCacheKey{aes:None,xor:Some(0x43)},&AtomicBool::new(false)).is_err());assert!(decode(&encrypted[..15],&key,&AtomicBool::new(false)).is_err());
    }
}
