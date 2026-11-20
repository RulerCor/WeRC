use std::{
    fs::{File, OpenOptions},
    path::Path,
};
use werc_domain::{AppError, AppResult};

pub fn offline_ready(path: &str) -> AppResult<bool> {
    let source = Path::new(path);
    crate::data_sources::checked(source, false)?;
    let wal = source.with_file_name(format!(
        "{}-wal",
        source
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(AppError::internal)?
    ));
    let mut held: Vec<File> = Vec::new();
    for (file, required) in [(source, true), (wal.as_path(), false)] {
        match std::fs::symlink_metadata(file) {
            Err(e) if !required && e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(AppError::new("SOURCE_NOT_FOUND", "来源或WAL无法访问")),
            Ok(_) => crate::data_sources::checked(file, false)?,
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000);
        }
        match options.open(file) {
            Ok(handle) => held.push(handle),
            Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => return Ok(false),
            Err(_) => {
                return Err(AppError::new(
                    "SOURCE_ACCESS_DENIED",
                    "无法只读来源；请检查权限",
                ));
            }
        }
    }
    Ok(true)
}

pub fn main_window_processes(pids: &[u32]) -> Vec<u32> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{HWND, LPARAM},
            UI::WindowsAndMessaging::{
                EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible,
            },
        };
        struct Context<'a> {
            candidates: &'a [u32],
            found: Vec<u32>,
        }
        unsafe extern "system" fn visit(window: HWND, param: LPARAM) -> i32 {
            let context = unsafe { &mut *(param as *mut Context<'_>) };
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(window, &mut pid);
            }
            if context.candidates.contains(&pid)
                && unsafe { IsWindowVisible(window) != 0 && GetWindow(window, GW_OWNER).is_null() }
                && !context.found.contains(&pid)
            {
                context.found.push(pid);
            }
            1
        }
        let mut context = Context {
            candidates: pids,
            found: Vec::new(),
        };
        unsafe {
            EnumWindows(Some(visit), &mut context as *mut _ as LPARAM);
        }
        context.found
    }
    #[cfg(not(windows))]
    {
        let _ = pids;
        Vec::new()
    }
}
