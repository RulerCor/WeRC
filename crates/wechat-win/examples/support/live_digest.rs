//! Diagnostic endpoint fingerprints, not a consistent live database snapshot.
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, os::windows::io::AsRawHandle, path::Path};
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::Memory::{
        CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY, UnmapViewOfFile,
    },
};

pub fn digest(path: &Path) -> Result<String, &'static str> {
    let mut file = File::open(path).map_err(|_| "SOURCE_UNAVAILABLE")?;
    let size = file.metadata().map_err(|_| "SOURCE_UNAVAILABLE")?.len();
    let mut hasher = Sha256::new();
    if path.to_string_lossy().ends_with("-shm") && size != 0 {
        if size > 16 * 1024 * 1024 {
            return Err("SOURCE_LIMIT");
        }
        // Windows ReadFile rejects byte-lock regions. Observe the existing SHM
        // through a strictly read-only mapping, like the SQLite reader itself.
        // Writer must be quiescent for equality to imply no reader-side changes.
        unsafe {
            let mapping = CreateFileMappingW(
                file.as_raw_handle(),
                std::ptr::null(),
                PAGE_READONLY,
                0,
                0,
                std::ptr::null(),
            );
            if mapping.is_null() {
                return Err("SHM_MAPPING_FAILED");
            }
            let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, size as usize);
            if view.Value.is_null() {
                CloseHandle(mapping);
                return Err("SHM_MAPPING_FAILED");
            }
            let mut buffer = [0_u8; 4096];
            let mut offset = 0_usize;
            while offset < size as usize {
                let n = buffer.len().min(size as usize - offset);
                for (i, byte) in buffer[..n].iter_mut().enumerate() {
                    *byte = std::ptr::read_volatile(view.Value.cast::<u8>().add(offset + i));
                }
                hasher.update(&buffer[..n]);
                offset += n;
            }
            UnmapViewOfFile(view);
            CloseHandle(mapping);
        }
    } else {
        if size > 1024 * 1024 * 1024 {
            return Err("SOURCE_LIMIT");
        }
        let mut total = 0_u64;
        let mut buffer = [0; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|_| "SOURCE_UNAVAILABLE")?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > 1024 * 1024 * 1024 {
                return Err("SOURCE_LIMIT");
            }
            hasher.update(&buffer[..n]);
        }
    }
    Ok(hex::encode(hasher.finalize()))
}
