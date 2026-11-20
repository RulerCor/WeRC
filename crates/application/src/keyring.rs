use werc_domain::{AppError, AppResult, connection::AcquiredDatabaseKey};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"WRCKEY01";
const ENTRY: usize = 80;
const MAXIMUM: usize = 512;
fn invalid() -> AppError {
    AppError::new(
        "KEY_PROTECTION_FAILED",
        "受保护密钥集格式无效；未使用或输出其中的内容",
    )
}

pub(crate) fn verified_count(bytes: &[u8]) -> AppResult<usize> {
    if bytes.len() == 32 {
        return Ok(0);
    }
    if !bytes.starts_with(MAGIC)
        || bytes.len() <= MAGIC.len()
        || !(bytes.len() - MAGIC.len()).is_multiple_of(ENTRY)
        || (bytes.len() - MAGIC.len()) / ENTRY > MAXIMUM
    {
        return Err(invalid());
    }
    let entries = &bytes[MAGIC.len()..];
    for (index, record) in entries.as_chunks::<ENTRY>().0.iter().enumerate() {
        if entries[..index * ENTRY]
            .as_chunks::<ENTRY>()
            .0
            .iter()
            .any(|old| old[..16] == record[..16])
        {
            return Err(invalid());
        }
    }
    Ok(entries.len() / ENTRY)
}
pub(crate) fn merge(
    previous: Option<&[u8]>,
    acquired: &AcquiredDatabaseKey,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let mut output = Zeroizing::new(MAGIC.to_vec());
    if let Some(previous) = previous {
        let count = verified_count(previous)?;
        if count > 0 {
            for record in previous[MAGIC.len()..].as_chunks::<ENTRY>().0.iter() {
                if record[..16] != acquired.salt {
                    output.extend_from_slice(record);
                }
            }
        }
    }
    if (output.len() - MAGIC.len()) / ENTRY >= MAXIMUM {
        return Err(AppError::new(
            "QUOTA_EXCEEDED",
            "每个账号最多保护512个已验真数据库密钥；请先清理旧密钥集",
        ));
    }
    output.extend_from_slice(&acquired.salt);
    output.extend_from_slice(acquired.key.as_ref());
    output.extend_from_slice(&acquired.first_page_sha256);
    Ok(output)
}
pub(crate) fn key_for(bytes: &[u8], salt: &[u8; 16]) -> AppResult<Zeroizing<Vec<u8>>> {
    if bytes.len() == 32 {
        return Ok(Zeroizing::new(bytes.to_vec()));
    }
    verified_count(bytes)?;
    let record = bytes[MAGIC.len()..]
        .as_chunks::<ENTRY>()
        .0
        .iter()
        .find(|record| record[..16] == *salt)
        .ok_or_else(|| {
            AppError::new(
                "KEY_SOURCE_MISMATCH",
                "没有与所选数据库salt匹配的受保护密钥，请为此库单独获取或提供密钥",
            )
        })?;
    Ok(Zeroizing::new(record[16..48].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_keys_per_salt_replaces_duplicates_and_bounds_storage() {
        let mut item = AcquiredDatabaseKey {
            key: Zeroizing::new([0x5a; 32]),
            salt: [1; 16],
            first_page_sha256: [3; 32],
        };
        let first = merge(Some(&[4; 32]), &item).unwrap();
        assert_eq!(verified_count(&first).unwrap(), 1);
        assert_eq!(&*key_for(&first, &[1; 16]).unwrap(), &[0x5a; 32]);
        assert!(key_for(&first, &[2; 16]).is_err());
        let mut next = merge(Some(&first), &item).unwrap();
        assert_eq!(verified_count(&next).unwrap(), 1);
        for number in 2..=MAXIMUM {
            item.salt = [0; 16];
            item.salt[..2].copy_from_slice(&(number as u16).to_le_bytes());
            next = merge(Some(&next), &item).unwrap();
        }
        assert_eq!(verified_count(&next).unwrap(), MAXIMUM);
        item.salt = [0; 16];
        item.salt[..2].copy_from_slice(&((MAXIMUM + 1) as u16).to_le_bytes());
        assert!(merge(Some(&next), &item).is_err());
        assert!(verified_count(b"not a protected keyring").is_err());
        let mut duplicate = first.to_vec();
        duplicate.extend_from_slice(&first[8..]);
        assert!(verified_count(&duplicate).is_err());
    }
}
