use werc_domain::{AppError, AppResult};

#[cfg(windows)]
fn transform(context: &str, bytes: &[u8], encrypt: bool) -> AppResult<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    use zeroize::Zeroize;
    if context.is_empty() || context.len() > 512 || bytes.is_empty() || bytes.len() > 65536 {
        return Err(AppError::new("INVALID_ARGUMENT", "受保护密钥参数无效"));
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: context.len() as u32,
        pbData: context.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    // CurrentUser 模式；不使用 LOCAL_MACHINE，也不允许系统弹出无界交互窗口。
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                null(),
                &entropy,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                null_mut(),
                &entropy,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(AppError::new(
            "KEY_PROTECTION_FAILED",
            "无法保护或恢复密钥；可能用户、来源或保护内容已改变",
        ));
    }
    if output.pbData.is_null() || output.cbData == 0 || output.cbData > 65536 {
        if !output.pbData.is_null() {
            unsafe {
                LocalFree(output.pbData.cast());
            }
        }
        return Err(AppError::new(
            "KEY_PROTECTION_FAILED",
            "系统返回了无效的受保护内容",
        ));
    }
    let result = unsafe {
        let buffer = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let result = buffer.to_vec();
        buffer.zeroize();
        LocalFree(output.pbData.cast());
        result
    };
    Ok(result)
}

pub fn protect(context: &str, bytes: &[u8]) -> AppResult<Vec<u8>> {
    #[cfg(windows)]
    {
        transform(context, bytes, true)
    }
    #[cfg(not(windows))]
    {
        let _ = (context, bytes);
        Err(AppError::new(
            "PLATFORM_UNSUPPORTED",
            "受保护密钥存储需要 Windows DPAPI",
        ))
    }
}
pub fn unprotect(context: &str, bytes: &[u8]) -> AppResult<Vec<u8>> {
    #[cfg(windows)]
    {
        transform(context, bytes, false)
    }
    #[cfg(not(windows))]
    {
        let _ = (context, bytes);
        Err(AppError::new(
            "PLATFORM_UNSUPPORTED",
            "受保护密钥存储需要 Windows DPAPI",
        ))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use zeroize::Zeroizing;
    #[test]
    fn maximum_database_keyring_roundtrips_with_dpapi() {
        let ring = Zeroizing::new(vec![0x5a; 8 + 512 * 80]);
        let wrapped = protect("synthetic-512-keyring", &ring).unwrap();
        assert!(wrapped.len() < 65536);
        let restored = Zeroizing::new(unprotect("synthetic-512-keyring", &wrapped).unwrap());
        assert_eq!(&*restored, &*ring);
        assert!(protect("synthetic-512-keyring", &[0; 65537]).is_err());
    }
    #[test]
    fn synthetic_dpapi_roundtrip_and_binding() {
        let key = Zeroizing::new(vec![0x5a; 32]);
        let blob = protect("synthetic-account/fingerprint", &key).unwrap();
        assert_ne!(&blob, &*key);
        let restored = Zeroizing::new(unprotect("synthetic-account/fingerprint", &blob).unwrap());
        assert_eq!(&*restored, &*key);
        assert!(unprotect("other-account/fingerprint", &blob).is_err());
        let mut damaged = blob;
        let end = damaged.len() - 1;
        damaged[end] ^= 1;
        assert!(unprotect("synthetic-account/fingerprint", &damaged).is_err());
    }
}
