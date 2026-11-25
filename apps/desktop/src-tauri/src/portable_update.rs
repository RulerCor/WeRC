//! Keep the portable program directory separate from its data directory.
use super::*;
use std::{io::Cursor, os::windows::process::CommandExt, path::Path, process::Command};

pub(super) fn is_portable() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            let manifest = std::fs::read(exe.with_file_name("manifest.json")).ok()?;
            let value: serde_json::Value = serde_json::from_slice(&manifest).ok()?;
            Some(value["product"] == "WeRC" && value["platform"] == "windows-x86_64")
        })
        .unwrap_or(false)
}

pub(super) fn is_installed() -> bool {
    use windows_sys::Win32::System::Registry::*;
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WeRC\0"
        .encode_utf16()
        .collect();
    let entry: Vec<u16> = "InstallLocation\0".encode_utf16().collect();
    let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
    else {
        return false;
    };
    [0, RRF_SUBKEY_WOW6464KEY, RRF_SUBKEY_WOW6432KEY]
        .into_iter()
        .any(|view| {
            let mut value = [0u16; 4096];
            let mut len = std::mem::size_of_val(&value) as u32;
            let result = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    subkey.as_ptr(),
                    entry.as_ptr(),
                    RRF_RT_REG_SZ | view,
                    std::ptr::null_mut(),
                    value.as_mut_ptr().cast(),
                    &mut len,
                )
            };
            if result != 0 {
                return false;
            }
            let directory =
                String::from_utf16_lossy(&value[..(len as usize / 2).saturating_sub(1)]);
            Path::new(directory.trim_matches('"'))
                .join("werc-desktop.exe")
                .canonicalize()
                .is_ok_and(|p| p == exe)
        })
}

pub(super) struct Staged {
    directory: PathBuf,
    current: PathBuf,
    profile: PathBuf,
    launched: bool,
}
impl Drop for Staged {
    fn drop(&mut self) {
        if !self.launched {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}

pub(super) fn stage(app: &tauri::AppHandle, version: &str, bytes: &[u8]) -> AppResult<Staged> {
    let failed = || {
        error(
            "UPDATE_STAGE_FAILED",
            "无法解包更新，请检查程序目录和可用空间",
        )
    };
    let current = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .ok_or_else(failed)?;
    let info = std::fs::symlink_metadata(&current).map_err(|_| failed())?;
    if !super::super::storage::ordinary(&info) {
        return Err(failed());
    }
    let profile = profile_directory::selected(std::env::args_os().skip(1))
        .map_err(|_| failed())?
        .unwrap_or(app.path().app_local_data_dir().map_err(|_| failed())?);
    let resolved = profile.canonicalize().map_err(|_| failed())?;
    let program = current.canonicalize().map_err(|_| failed())?;
    let active = app.state::<storage_location::Locations>().data.canonicalize().map_err(|_| failed())?;
    if resolved.starts_with(&program) || active.starts_with(&program) {
        return Err(error(
            "UPDATE_DATA_IN_PROGRAM",
            "资料目录在程序目录内，请手动更新",
        ));
    }
    let parent = current.parent().ok_or_else(failed)?;
    let directory = parent.join(format!(".werc-update-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).map_err(|_| failed())?;
    let staged = Staged {
        directory,
        current,
        profile,
        launched: false,
    };
    let root = format!("WeRC-v{version}");
    let payload = staged.directory.join("payload");
    std::fs::create_dir(&payload).map_err(|_| failed())?;
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| failed())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|_| failed())?;
        let name = entry.enclosed_name().ok_or_else(failed)?;
        let relative = name.strip_prefix(&root).map_err(|_| failed())?;
        if relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(failed());
        }
        if entry.is_dir() {
            continue;
        }
        let output = payload.join(relative);
        std::fs::create_dir_all(output.parent().ok_or_else(failed)?).map_err(|_| failed())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|_| failed())?;
        std::io::copy(&mut entry, &mut file).map_err(|_| failed())?;
    }
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(payload.join("manifest.json")).map_err(|_| failed())?,
    )
    .map_err(|_| failed())?;
    if manifest["product"] != "WeRC"
        || manifest["version"] != version
        || !payload.join("werc-desktop.exe").is_file()
    {
        return Err(failed());
    }
    Ok(staged)
}
impl Staged {
    pub(super) fn launch(mut self) -> AppResult<()> {
        let failed = || error("UPDATE_INSTALL_FAILED", "无法启动便携版更新程序");
        let helper = self.directory.join("apply.ps1");
        std::fs::write(&helper, include_bytes!("apply_portable_update.ps1"))
            .map_err(|_| failed())?;
        let system = std::env::var_os("SystemRoot").ok_or_else(failed)?;
        Command::new(PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe"))
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(helper)
            .arg(std::process::id().to_string())
            .arg(&self.current)
            .arg(&self.profile)
            .current_dir(&self.directory)
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| failed())?;
        self.launched = true;
        Ok(())
    }
}
