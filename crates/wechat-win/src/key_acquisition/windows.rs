use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::Path,
};
use werc_domain::connection::{AcquiredDatabaseKey, ClientInstallation};

fn open_database(path: &str) -> AppResult<File> {
    crate::data_sources::checked(Path::new(path), false)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // 取钥只检查第一页，允许现有写者，不请求写入或删除权限。
        options.share_mode(3).custom_flags(0x00200000);
    }
    let file = options
        .open(path)
        .map_err(|_| AppError::new("SOURCE_ACCESS_DENIED", "无法只读所选数据库第一页"))?;
    let metadata = file.metadata().map_err(|_| AppError::internal())?;
    if !metadata.is_file() || metadata.len() < 4096 {
        return Err(AppError::new(
            "DATABASE_FORMAT_UNSUPPORTED",
            "来源不足一页或不是普通文件",
        ));
    }
    Ok(file)
}
fn first_page(file: &mut File) -> AppResult<Zeroizing<Vec<u8>>> {
    let mut page = Zeroizing::new(vec![0; 4096]);
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut page))
        .map_err(|_| AppError::new("SOURCE_READ_FAILED", "无法完整读取数据库第一页"))?;
    if page.starts_with(b"SQLite format 3\0") {
        return Err(AppError::new(
            "DATABASE_NOT_ENCRYPTED",
            "明文来源不需要进程取钥",
        ));
    }
    Ok(page)
}
pub(super) fn database_salt(path: &str) -> AppResult<[u8; 16]> {
    let page = first_page(&mut open_database(path)?)?;
    page[..16].try_into().map_err(|_| AppError::internal())
}

fn validate_acquired_page(before: &[u8], after: &[u8], key: &[u8]) -> AppResult<()> {
    // The open read handle denies deletion/replacement. A checkpoint may update
    // page 1 during a scan; accept only the same salt and a fresh valid page HMAC
    // under the independently discovered key. This proves a key, not a snapshot.
    if before.len() != 4096
        || after.len() != 4096
        || before[..16] != after[..16]
        || werc_db_crypto::verify_first_page(key, before).is_err()
        || werc_db_crypto::verify_first_page(key, after).is_err()
    {
        return Err(AppError::new(
            "SOURCE_CHANGED",
            "取钥期间来源salt已变化或第一页认证失败，请重新读取",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod continuity_tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha512;
    fn sign(page: &mut [u8; 4096], key: &[u8; 32]) {
        let salt: &[u8; 16] = page[..16].try_into().unwrap();
        let derived = werc_db_crypto::authentication_key(key, salt).unwrap();
        let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(derived.as_ref()).unwrap();
        mac.update(&page[16..4032]);
        mac.update(&1u32.to_le_bytes());
        page[4032..].copy_from_slice(&mac.finalize().into_bytes());
    }
    #[test]
    fn checkpoint_accepts_only_same_salt_and_fresh_authenticated_page() {
        let before = super::super::tests::page();
        let mut after = before;
        after[32] ^= 1;
        assert!(validate_acquired_page(&before, &after, &[0x5a; 32]).is_err());
        sign(&mut after, &[0x5a; 32]);
        validate_acquired_page(&before, &after, &[0x5a; 32]).unwrap();
        after[0] ^= 1;
        sign(&mut after, &[0x5a; 32]);
        assert!(validate_acquired_page(&before, &after, &[0x5a; 32]).is_err());
        assert!(validate_acquired_page(&before, &before[..4095], &[0x5a; 32]).is_err());
        assert!(validate_acquired_page(&before, &before, &[0x5b; 32]).is_err());
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        mem::{size_of, zeroed},
        ptr::null_mut,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{EqualSid, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser},
        System::{
            Diagnostics::Debug::ReadProcessMemory,
            Memory::{
                MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
                PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_READONLY, PAGE_READWRITE, PAGE_WRITECOPY,
                VirtualQueryEx,
            },
            Threading::{
                GetCurrentProcess, GetExitCodeProcess, IsWow64Process2, OpenProcess,
                OpenProcessToken, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
                QueryFullProcessImageNameW,
            },
        },
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn denied() -> AppError {
        AppError::new(
            "PROCESS_ACCESS_DENIED",
            "无法只读目标进程；不会提升权限、注入或附加调试器",
        )
    }
    fn owner(process: HANDLE) -> AppResult<Vec<usize>> {
        let mut token = null_mut();
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(denied());
        }
        let token = Handle(token);
        let mut size = 0;
        unsafe {
            GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut size);
        }
        if size < size_of::<TOKEN_USER>() as u32 || size > 65536 {
            return Err(denied());
        }
        let mut bytes = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                bytes.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } == 0
        {
            return Err(denied());
        }
        Ok(bytes)
    }
    fn normalized(path: &Path) -> AppResult<String> {
        let path = path.canonicalize().map_err(|_| denied())?;
        Ok(path
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_lowercase())
    }
    pub(super) struct Process(Handle);
    impl Process {
        pub(super) fn open(pid: u32, expected: &Path) -> AppResult<Self> {
            if pid == 0 || pid == std::process::id() {
                return Err(AppError::new(
                    "INVALID_PROCESS",
                    "必须选择另外一个明确的目标进程",
                ));
            }
            let handle =
                unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
            if handle.is_null() {
                return Err(denied());
            }
            let process = Self(Handle(handle));
            let own = owner(unsafe { GetCurrentProcess() })?;
            let target = owner(handle)?;
            let same = unsafe {
                EqualSid(
                    (own.as_ptr().cast::<TOKEN_USER>())
                        .read_unaligned()
                        .User
                        .Sid,
                    (target.as_ptr().cast::<TOKEN_USER>())
                        .read_unaligned()
                        .User
                        .Sid,
                )
            };
            if same == 0 {
                return Err(AppError::new(
                    "PROCESS_OWNER_MISMATCH",
                    "目标进程不属于当前Windows用户",
                ));
            }
            let mut machine = 0;
            let mut native = 0;
            if unsafe { IsWow64Process2(handle, &mut machine, &mut native) } == 0
                || machine != 0
                || native != 0x8664
            {
                return Err(AppError::new(
                    "PROCESS_ARCH_UNSUPPORTED",
                    "取钥目前只支持Windows x64原生进程",
                ));
            }
            let mut path = vec![0u16; 32768];
            let mut length = path.len() as u32;
            if unsafe { QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut length) } == 0
            {
                return Err(denied());
            }
            let actual = String::from_utf16(&path[..length as usize]).map_err(|_| denied())?;
            if normalized(Path::new(&actual))? != normalized(expected)? {
                return Err(AppError::new(
                    "PROCESS_SOURCE_MISMATCH",
                    "该PID不属于绑定的安装文件",
                ));
            }
            process.alive()?;
            Ok(process)
        }
        pub(super) fn alive(&self) -> AppResult<()> {
            let mut exit = 0;
            if unsafe { GetExitCodeProcess(self.0.0, &mut exit) } == 0 || exit != 259 {
                return Err(AppError::new("PROCESS_EXITED", "取钥期间目标进程已退出"));
            }
            Ok(())
        }
        fn regions(&self) -> AppResult<Vec<Region>> {
            self.regions_kind(false)
        }
        fn regions_kind(&self, private_only: bool) -> AppResult<Vec<Region>> {
            let mut address = 0x10000u64;
            let mut result = Vec::new();
            let mut queries = 0;
            let deadline = Instant::now() + Duration::from_secs(10);
            while address < MAX_ADDRESS {
                queries += 1;
                if queries > 65536 || Instant::now() >= deadline {
                    return Err(limit());
                }
                let mut info: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
                if unsafe {
                    VirtualQueryEx(
                        self.0.0,
                        address as *const _,
                        &mut info,
                        size_of::<MEMORY_BASIC_INFORMATION>(),
                    )
                } == 0
                {
                    break;
                }
                let base = info.BaseAddress as u64;
                let Some(end) = base.checked_add(info.RegionSize as u64) else {
                    return Err(limit());
                };
                if end <= address {
                    return Err(limit());
                }
                let readable = matches!(
                    info.Protect & 0xff,
                    PAGE_READONLY
                        | PAGE_READWRITE
                        | PAGE_WRITECOPY
                        | PAGE_EXECUTE_READ
                        | PAGE_EXECUTE_READWRITE
                        | PAGE_EXECUTE_WRITECOPY
                );
                if info.State == MEM_COMMIT
                    && (!private_only
                        || info.Type == windows_sys::Win32::System::Memory::MEM_PRIVATE)
                    && info.Protect & PAGE_GUARD == 0
                    && readable
                    && end <= MAX_ADDRESS
                {
                    result.push(Region {
                        base,
                        size: info.RegionSize,
                    });
                }
                address = end;
            }
            self.alive()?;
            Ok(result)
        }
        #[cfg(test)]
        pub(super) fn find(&mut self, page: &[u8]) -> AppResult<Zeroizing<[u8; 32]>> {
            let regions = self.regions()?;
            scan(self, &regions, page)
        }
    }
    impl Memory for Process {
        fn read(&mut self, address: u64, size: usize) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
            let mut bytes = Zeroizing::new(vec![0u8; size]);
            let mut read = 0;
            let ok = unsafe {
                ReadProcessMemory(
                    self.0.0,
                    address as *const _,
                    bytes.as_mut_ptr().cast(),
                    size,
                    &mut read,
                )
            };
            if ok == 0 || read != size {
                return Ok(None);
            }
            Ok(Some(bytes))
        }
    }
    pub(crate) fn read_emoticon(
        installation: &ClientInstallation,
        pid: u32,
        account: &str,
        cache: &[u8],
        md5: &str,
        allow_wxgf: bool,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        super::super::emoticon::check(cancel)?;
        let current = crate::discovery::inspect(&installation.executable_path)?;
        if current.sha256 != installation.sha256 || current.id != installation.id {
            return Err(AppError::new("CONNECTION_CHANGED", "安装文件身份已变化"));
        }
        if current.architecture != "x86_64" || current.version != "4.1.15.13" {
            return Err(AppError::new(
                "EMOTICON_LAYOUT_UNSUPPORTED",
                "本地表情缓存目前仅验证微信4.1.15.13/x64",
            ));
        }
        let mut process = Process::open(pid, Path::new(&current.executable_path))?;
        let regions = process.regions_kind(true)?;
        let bytes = super::super::emoticon::scan_cache(
            &mut process,
            &regions,
            account,
            cache,
            md5,
            allow_wxgf,
            cancel,
        )?;
        process.alive()?;
        if crate::discovery::inspect(&installation.executable_path)?.sha256 != current.sha256 {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "读取期间安装文件已变化",
            ));
        }
        super::super::emoticon::check(cancel)?;
        Ok(bytes)
    }
    pub(crate) fn acquire_image(installation:&ClientInstallation,pid:u32,sample:&[u8],cancel:&std::sync::atomic::AtomicBool)->AppResult<werc_domain::connection::ImageCacheKey>{
        let current=crate::discovery::inspect(&installation.executable_path)?;
        if current.sha256!=installation.sha256||current.id!=installation.id||current.version!="4.1.15.13"||current.architecture!="x86_64"{return Err(AppError::new("CONNECTION_CHANGED","当前微信构建与图片密钥适配不匹配"));}
        let mut process=Process::open(pid,Path::new(&current.executable_path))?;
        let regions=process.regions_kind(true)?;
        let result=super::super::image::scan_image(&mut process,&regions,sample,cancel)?;
        process.alive()?;crate::image_cache::check(cancel)?;Ok(result)
    }
    pub(crate) fn acquire(
        installation: &ClientInstallation,
        pid: u32,
        database: &str,
    ) -> AppResult<AcquiredDatabaseKey> {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let control = werc_application::ports::KeyScanControl { cancel: &cancel, deadline: Instant::now() + Duration::from_secs(30), progress: &|_| {} };
        acquire_controlled(installation, pid, database, &control)
    }
    pub(crate) fn acquire_controlled(
        installation: &ClientInstallation, pid: u32, database: &str,
        control: &werc_application::ports::KeyScanControl<'_>,
    ) -> AppResult<AcquiredDatabaseKey> {
        control.check()?;
        let current = crate::discovery::inspect(&installation.executable_path)?;
        if current.sha256 != installation.sha256 || current.id != installation.id {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "安装文件指纹已改变，请重新绑定",
            ));
        }
        if !super::super::supports_config_cipher(&current.version, &current.architecture) {
            return Err(AppError::new(
                "KEY_LAYOUT_UNSUPPORTED",
                "只读Config.Cipher路径接受显式选择的微信4.1.x或4.2.x/x64；候选必须通过所选数据库第一页认证",
            ));
        }
        let mut file = open_database(database)?;
        let page = first_page(&mut file)?;
        let mut process = Process::open(pid, Path::new(&current.executable_path))?;
        control.check()?;
        let regions = process.regions()?;
        let mut memory = ControlledMemory { memory: &mut process, control, bytes: 0 };
        let key = scan(&mut memory, &regions, &page)?;
        control.check()?;
        process.alive()?;
        let after_page = first_page(&mut file)?;
        validate_acquired_page(&page, &after_page, key.as_ref())?;
        let after = crate::discovery::inspect(&installation.executable_path)?;
        if after.sha256 != current.sha256 {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "取钥期间安装文件发生变化",
            ));
        }
        control.check()?;
        Ok(AcquiredDatabaseKey {
            key,
            salt: page[..16].try_into().map_err(|_| AppError::internal())?,
            first_page_sha256: Sha256::digest(after_page.as_slice()).into(),
        })
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::{BufRead, BufReader, Write};
        use std::process::{Command, Stdio};
        #[test]
        #[ignore = "需要显式构建自有内存目标，统一验证脚本装配"]
        fn reads_only_owned_synthetic_child_and_checks_source() {
            let executable = std::env::var("WERC_KEY_FIXTURE").expect("先构建自有fixture");
            let mut child = Command::new(&executable)
                .arg("--synthetic-memory-target")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut ready = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut ready)
                .unwrap();
            assert_eq!(ready.trim(), "SYNTHETIC_READY");
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                assert!(Process::open(child.id(), &std::env::current_exe().unwrap()).is_err());
                let mut process = Process::open(child.id(), Path::new(&executable)).unwrap();
                assert_eq!(
                    *process.find(&super::super::super::tests::page()).unwrap(),
                    [0x5a; 32]
                );
            }));
            child.stdin.as_mut().unwrap().write_all(b"exit\n").unwrap();
            assert!(child.wait().unwrap().success());
            if let Err(error) = result {
                std::panic::resume_unwind(error);
            }
        }
    }
}

#[cfg(windows)]
pub(super) use native::acquire;
#[cfg(windows)]
pub(super) use native::acquire_controlled;
#[cfg(not(windows))]
pub(super) fn acquire_controlled(_: &ClientInstallation, _: u32, _: &str, control: &werc_application::ports::KeyScanControl<'_>) -> AppResult<AcquiredDatabaseKey> {
    control.check()?;
    Err(AppError::new("PLATFORM_UNSUPPORTED", "只读取钥需要Windows"))
}
#[cfg(not(windows))]
pub(super) fn acquire(_: &ClientInstallation, _: u32, _: &str) -> AppResult<AcquiredDatabaseKey> {
    Err(AppError::new("PLATFORM_UNSUPPORTED", "只读取钥需要Windows"))
}

#[cfg(windows)]
pub(crate) use native::read_emoticon;
#[cfg(windows)]
pub(crate) use native::acquire_image;
