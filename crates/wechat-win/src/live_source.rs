//! Windows file-object identity; independent of mutable page/WAL contents.
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::Path,
};
use werc_domain::{AppError, AppResult};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
};

pub struct LiveSourcePin {
    _file: File,
    fingerprint: String,
}
impl LiveSourcePin {
    pub fn open(path: &Path) -> AppResult<Self> {
        crate::data_sources::checked(path, false)?;
        let file = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x00200000)
            .open(path)
            .map_err(|_| AppError::new("LIVE_SOURCE_UNAVAILABLE", "无法只读固定当前来源文件"))?;
        let metadata = file.metadata().map_err(|_| AppError::internal())?;
        if !metadata.is_file()
            || !(4096..=1024 * 1024 * 1024).contains(&metadata.len())
            || metadata.file_attributes() & 0x400 != 0
        {
            return Err(AppError::new(
                "LIVE_SOURCE_INVALID",
                "当前来源不是受支持的普通数据库文件",
            ));
        }
        let mut info = FILE_ID_INFO::default();
        // FILE_ID_INFO is a fixed-size system output buffer; the read handle
        // remains alive through all consumers of this pin, with no delete share.
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        } == 0
            || info.FileId.Identifier.iter().all(|b| *b == 0)
        {
            return Err(AppError::new(
                "LIVE_SOURCE_ID_UNAVAILABLE",
                "无法验证当前来源文件身份",
            ));
        }
        let mut hash = Sha256::new();
        hash.update(b"WeRC/Windows-live-file/v1\0");
        hash.update(info.VolumeSerialNumber.to_le_bytes());
        hash.update(info.FileId.Identifier);
        hash.update(metadata.creation_time().to_le_bytes());
        Ok(Self {
            _file: file,
            fingerprint: hex::encode(hash.finalize()),
        })
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn require(&self, expected: &str) -> AppResult<()> {
        if self.fingerprint != expected {
            return Err(AppError::new(
                "LIVE_SOURCE_REPLACED",
                "当前来源文件已被替换，请重新读取",
            ));
        }
        Ok(())
    }
}
pub fn fingerprint(path: &Path) -> AppResult<String> {
    Ok(LiveSourcePin::open(path)?.fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};
    #[test]
    fn pin_allows_writer_but_blocks_replace_and_detects_identical_replacement() {
        let dir = std::env::temp_dir().join(format!("werc-source-pin-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("message_0.db");
        std::fs::write(&path, vec![0x35; 4096]).unwrap();
        let pin = LiveSourcePin::open(&path).unwrap();
        let old = pin.fingerprint().to_owned();
        let mut writer = OpenOptions::new().write(true).open(&path).unwrap();
        writer.seek(SeekFrom::Start(100)).unwrap();
        writer.write_all(&[0x36]).unwrap();
        writer.sync_all().unwrap();
        assert_eq!(fingerprint(&path).unwrap(), old);
        assert!(std::fs::rename(&path, dir.join("old.db")).is_err());
        assert!(std::fs::remove_file(&path).is_err());
        drop(writer);
        drop(pin);
        std::fs::rename(&path, dir.join("old.db")).unwrap();
        std::fs::copy(dir.join("old.db"), &path).unwrap();
        let replaced = LiveSourcePin::open(&path).unwrap();
        assert_ne!(replaced.fingerprint(), old);
        assert_eq!(
            replaced.require(&old).err().unwrap().code,
            "LIVE_SOURCE_REPLACED"
        );
        drop(replaced);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
