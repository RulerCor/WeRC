use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use werc_domain::{
    AppError, AppResult,
    connection::{ClientInstallation, DiscoveryReport},
};

pub(crate) fn architecture(path: &Path) -> AppResult<&'static str> {
    let mut file = File::open(path)
        .map_err(|_| AppError::new("CLIENT_ACCESS_DENIED", "无法读取所选程序文件"))?;
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos).map_err(|_| invalid())?;
    if &dos[..2] != b"MZ" {
        return Err(invalid());
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().map_err(|_| invalid())?) as u64;
    let length = file.metadata().map_err(|_| invalid())?.len();
    if offset < 64 || offset.checked_add(6).is_none_or(|end| end > length) {
        return Err(invalid());
    }
    file.seek(SeekFrom::Start(offset)).map_err(|_| invalid())?;
    let mut pe = [0u8; 6];
    file.read_exact(&mut pe).map_err(|_| invalid())?;
    if &pe[..4] != b"PE\0\0" {
        return Err(invalid());
    }
    match u16::from_le_bytes([pe[4], pe[5]]) {
        0x8664 => Ok("x86_64"),
        0x14c => Ok("x86"),
        0xaa64 => Ok("arm64"),
        _ => Err(invalid()),
    }
}
fn invalid() -> AppError {
    AppError::new("INVALID_EXECUTABLE", "文件不是受识别架构的 Windows PE 程序")
}

pub fn inspect(path: &str) -> AppResult<ClientInstallation> {
    if path.is_empty() || path.len() > 32760 || path.chars().any(char::is_control) {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "请选择本机微信安装程序的绝对路径",
        ));
    }
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "安装程序路径必须是绝对路径",
        ));
    }
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or_default();
    if !["weixin.exe", "wechat.exe"].contains(&name.to_ascii_lowercase().as_str()) {
        return Err(AppError::new(
            "INVALID_EXECUTABLE",
            "请选择 Weixin.exe 或 WeChat.exe，而不是安装包、DLL 或快捷方式",
        ));
    }
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor)
            .map_err(|_| AppError::new("CLIENT_ACCESS_DENIED", "安装路径不存在或无法访问"))?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::new(
                "UNSAFE_PATH",
                "安装路径不能经过符号链接或重解析点",
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(AppError::new("UNSAFE_PATH", "安装路径不能经过重解析点"));
            }
        }
    }
    let path = path
        .canonicalize()
        .map_err(|_| AppError::new("CLIENT_ACCESS_DENIED", "无法规范化安装路径"))?;
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        {
            return Err(AppError::new(
                "UNSAFE_PATH",
                "仅接受本机磁盘安装，不接受网络共享路径",
            ));
        }
    }
    let metadata = path.metadata().map_err(|_| invalid())?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 * 1024 {
        return Err(invalid());
    }
    let architecture = architecture(&path)?.to_string();
    let (version, product_name) = file_version(&path)?;
    let mut file = File::open(&path).map_err(|_| invalid())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 16384];
    let mut total = 0u64;
    loop {
        let length = file.read(&mut buffer).map_err(|_| invalid())?;
        if length == 0 {
            break;
        }
        total += length as u64;
        if total > 256 * 1024 * 1024 {
            return Err(invalid());
        }
        hash.update(&buffer[..length]);
    }
    let after = path.metadata().map_err(|_| invalid())?;
    if total != metadata.len()
        || after.len() != metadata.len()
        || after.modified().ok() != metadata.modified().ok()
    {
        return Err(AppError::new(
            "CLIENT_CHANGED",
            "检查期间程序文件发生变化，请重试",
        ));
    }
    let executable_path = path.to_str().ok_or_else(invalid)?.to_string();
    let id = hex::encode(Sha256::digest(executable_path.to_lowercase().as_bytes()));
    let (compatibility, reason) = crate::native_profiles::version_status(&version, &architecture);
    Ok(ClientInstallation {
        id, executable_path, version, architecture, product_name,
        sha256: hex::encode(hash.finalize()), process_ids: vec![], sources: vec!["manual".into()],
        compatibility: compatibility.into(),
        reason,
    })
}

pub fn discover() -> AppResult<DiscoveryReport> {
    let mut report = DiscoveryReport::default();
    let mut candidates: Vec<(PathBuf, String, Option<u32>)> = Vec::new();
    #[cfg(windows)]
    native::candidates(&mut candidates, &mut report.warnings)?;
    #[cfg(not(windows))]
    return Err(AppError::new(
        "PLATFORM_UNSUPPORTED",
        "客户端发现仅支持 Windows",
    ));
    #[allow(unreachable_code)]
    {
        let mut found: BTreeMap<String, ClientInstallation> = BTreeMap::new();
        for (path, source, pid) in candidates.into_iter().take(256) {
            let Some(path) = path.to_str() else {
                continue;
            };
            match inspect(path) {
                Ok(mut item) => {
                    item.sources = vec![source.clone()];
                    let entry = found.entry(item.id.clone()).or_insert(item);
                    if !entry.sources.contains(&source) {
                        entry.sources.push(source);
                    }
                    if let Some(pid) = pid
                        && !entry.process_ids.contains(&pid)
                    {
                        entry.process_ids.push(pid);
                    }
                }
                Err(error) => {
                    if report.warnings.len() < 32 {
                        report
                            .warnings
                            .push(format!("候选检查未通过：{}", error.code));
                    }
                }
            }
        }
        report.installations = found.into_values().collect();
        Ok(report)
    }
}

#[cfg(not(windows))]
fn file_version(_path: &Path) -> AppResult<(String, String)> {
    Err(AppError::new(
        "PLATFORM_UNSUPPORTED",
        "文件版本读取仅支持 Windows",
    ))
}
#[cfg(windows)]
pub(crate) fn file_version(path: &Path) -> AppResult<(String, String)> {
    native::file_version(path)
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::c_void,
        mem::{size_of, zeroed},
        os::windows::ffi::OsStrExt,
        ptr::null_mut,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
        },
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Registry::{
                HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
                RRF_SUBKEY_WOW6432KEY, RRF_SUBKEY_WOW6464KEY, RegGetValueW,
            },
            Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    };
    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    pub fn file_version(path: &Path) -> AppResult<(String, String)> {
        let path = wide(path.as_os_str());
        // 只解析本机文件的版本资源，不装载或执行该程序。
        unsafe {
            let length = GetFileVersionInfoSizeW(path.as_ptr(), null_mut());
            if length == 0 || length > 16 * 1024 * 1024 {
                return Err(AppError::new(
                    "VERSION_UNAVAILABLE",
                    "程序没有可读取的文件版本资源",
                ));
            }
            let mut bytes = vec![0u8; length as usize];
            if GetFileVersionInfoW(path.as_ptr(), 0, length, bytes.as_mut_ptr().cast()) == 0 {
                return Err(invalid());
            }
            let mut pointer: *mut c_void = null_mut();
            let mut size = 0;
            let root = [b'\\' as u16, 0];
            if VerQueryValueW(
                bytes.as_ptr().cast(),
                root.as_ptr(),
                &mut pointer,
                &mut size,
            ) == 0
                || pointer.is_null()
                || (size as usize) < size_of::<VS_FIXEDFILEINFO>()
            {
                return Err(invalid());
            }
            let info = pointer.cast::<VS_FIXEDFILEINFO>().read_unaligned();
            if info.dwSignature != 0xfeef04bd {
                return Err(invalid());
            }
            let version = format!(
                "{}.{}.{}.{}",
                info.dwFileVersionMS >> 16,
                info.dwFileVersionMS & 65535,
                info.dwFileVersionLS >> 16,
                info.dwFileVersionLS & 65535
            );
            let mut product = String::new();
            let translations = wide(std::ffi::OsStr::new("\\VarFileInfo\\Translation"));
            if VerQueryValueW(
                bytes.as_ptr().cast(),
                translations.as_ptr(),
                &mut pointer,
                &mut size,
            ) != 0
                && !pointer.is_null()
                && size >= 4
            {
                let language = pointer.cast::<u16>().read_unaligned();
                let page = pointer.cast::<u16>().add(1).read_unaligned();
                let query = wide(std::ffi::OsStr::new(&format!(
                    "\\StringFileInfo\\{language:04x}{page:04x}\\ProductName"
                )));
                if VerQueryValueW(
                    bytes.as_ptr().cast(),
                    query.as_ptr(),
                    &mut pointer,
                    &mut size,
                ) != 0
                    && !pointer.is_null()
                    && size < 1024
                {
                    let chars = std::slice::from_raw_parts(pointer.cast::<u16>(), size as usize);
                    product = String::from_utf16_lossy(chars)
                        .trim_end_matches('\0')
                        .to_string();
                }
            }
            Ok((version, product))
        }
    }
    fn registry(root: HKEY, key: &str, value: &str, view: u32) -> Option<PathBuf> {
        let key = wide(std::ffi::OsStr::new(key));
        let value = wide(std::ffi::OsStr::new(value));
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | view;
        unsafe {
            let mut size = 0u32;
            if RegGetValueW(
                root,
                key.as_ptr(),
                value.as_ptr(),
                flags,
                null_mut(),
                null_mut(),
                &mut size,
            ) != 0
                || size == 0
                || size > 65536
            {
                return None;
            }
            let mut chars = vec![0u16; (size as usize).div_ceil(2)];
            if RegGetValueW(
                root,
                key.as_ptr(),
                value.as_ptr(),
                flags,
                null_mut(),
                chars.as_mut_ptr().cast(),
                &mut size,
            ) != 0
            {
                return None;
            }
            let text = String::from_utf16_lossy(&chars)
                .trim_end_matches('\0')
                .trim_matches('"')
                .to_string();
            if text.is_empty() {
                None
            } else {
                Some(PathBuf::from(text))
            }
        }
    }
    fn installation_candidates(
        path: PathBuf,
        source: &str,
        output: &mut Vec<(PathBuf, String, Option<u32>)>,
    ) {
        if path.is_file() {
            output.push((path, source.into(), None));
            return;
        }
        for name in ["Weixin.exe", "WeChat.exe"] {
            let candidate = path.join(name);
            if candidate.is_file() {
                output.push((candidate, source.into(), None));
            }
        }
        // 仅一层已知安装根目录，兼容版本号子目录，不递归搜索用户文件。
        if let Ok(entries) = std::fs::read_dir(&path) {
            for entry in entries.take(64).flatten() {
                let name = entry.file_name();
                if !name
                    .to_string_lossy()
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.')
                {
                    continue;
                }
                for executable in ["Weixin.exe", "WeChat.exe"] {
                    let candidate = entry.path().join(executable);
                    if candidate.is_file() {
                        output.push((candidate, source.into(), None));
                    }
                }
            }
        }
    }
    pub fn candidates(
        output: &mut Vec<(PathBuf, String, Option<u32>)>,
        warnings: &mut Vec<String>,
    ) -> AppResult<()> {
        for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for view in [RRF_SUBKEY_WOW6432KEY, RRF_SUBKEY_WOW6464KEY] {
                for key in ["Software\\Tencent\\Weixin", "Software\\Tencent\\WeChat"] {
                    for value in ["InstallPath", "InstallDir"] {
                        if let Some(path) = registry(root, key, value, view) {
                            installation_candidates(path, "registry", output);
                        }
                    }
                }
                for executable in ["Weixin.exe", "WeChat.exe"] {
                    if let Some(path) = registry(
                        root,
                        &format!(
                            "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{executable}"
                        ),
                        "",
                        view,
                    ) {
                        installation_candidates(path, "registry", output);
                    }
                }
            }
        }
        for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(variable) {
                for suffix in [
                    "Tencent/Weixin",
                    "Tencent/WeChat",
                    "Programs/Tencent/Weixin",
                    "Programs/Tencent/WeChat",
                ] {
                    installation_candidates(
                        PathBuf::from(&base).join(suffix),
                        "installation_directory",
                        output,
                    );
                }
            }
        }
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                warnings.push("进程元数据暂时不可用；仍可手动选择安装文件。".into());
                return Ok(());
            }
            let mut entry: PROCESSENTRY32W = zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snapshot, &mut entry);
            while more != 0 {
                let length = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..length]).to_lowercase();
                if ["weixin.exe", "wechat.exe"].contains(&name.as_str()) {
                    let process =
                        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                    if !process.is_null() {
                        let mut path = vec![0u16; 32768];
                        let mut length = path.len() as u32;
                        if QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length)
                            != 0
                        {
                            output.push((
                                PathBuf::from(String::from_utf16_lossy(&path[..length as usize])),
                                "process_metadata".into(),
                                Some(entry.th32ProcessID),
                            ));
                        }
                        CloseHandle(process);
                    } else if warnings.len() < 32 {
                        warnings
                            .push("某个匹配进程的安装路径无权查询；未提权、未读取内存。".into());
                    }
                }
                more = Process32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_non_pe_and_invalid_offsets() {
        let path = std::env::temp_dir().join(format!("werc-pe-{}", std::process::id()));
        std::fs::write(&path, [0u8; 64]).unwrap();
        assert!(architecture(&path).is_err());
        let mut bytes = [0u8; 80];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(architecture(&path).is_err());
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(architecture(&path).unwrap(), "x86_64");
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn rejects_relative_and_non_client_names() {
        assert!(inspect("Weixin.exe").is_err());
        assert!(inspect("C:/Windows/not-a-client.exe").is_err());
    }
    #[test]
    #[cfg(windows)]
    fn reads_only_explicit_synthetic_fixture_when_provided() {
        let Ok(path) = std::env::var("WERC_TEST_CLIENT") else {
            return;
        };
        let item = inspect(&path).unwrap();
        assert_eq!(item.version, "0.0.0.1");
        assert_eq!(item.architecture, "x86_64");
        assert_eq!(item.compatibility, "unsupported");
        assert_eq!(item.sha256.len(), 64);
        assert!(item.product_name.contains("synthetic"));
    }
}
