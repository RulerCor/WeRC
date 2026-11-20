//! v3: one SQLite-owned page image, fixed-size page/WAL scratch, bounded row batches.
use crate::{
    codec::{self, ReadResult},
    schema, snapshot,
};
use rusqlite::{Connection, ffi, serialize::OwnedData};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    ptr::NonNull,
};
use werc_domain::history::{HistoryReadEvent, HistoryReadRequest};
use zeroize::{Zeroize, Zeroizing};

struct Image {
    pointer: NonNull<u8>,
    length: usize,
}
impl Image {
    fn new(length: usize) -> ReadResult<Self> {
        let pointer = NonNull::new(unsafe { ffi::sqlite3_malloc64(length as u64) } as *mut u8)
            .ok_or("DATABASE_LIMIT_EXCEEDED")?;
        unsafe {
            std::ptr::write_bytes(pointer.as_ptr(), 0, length);
        }
        Ok(Self { pointer, length })
    }
    fn bytes(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.length) }
    }
    fn transfer(self, length: usize) -> OwnedData {
        // Allocated only by sqlite3_malloc64: never transfer a Rust Vec allocator.
        let owned = unsafe { OwnedData::from_raw_nonnull(self.pointer, length) };
        std::mem::forget(self);
        owned
    }
}
impl Drop for Image {
    fn drop(&mut self) {
        self.bytes().zeroize();
        unsafe {
            ffi::sqlite3_free(self.pointer.as_ptr().cast());
        }
    }
}

pub fn read_database(
    request: &HistoryReadRequest,
    visitor: &mut dyn FnMut(HistoryReadEvent) -> ReadResult<()>,
) -> ReadResult<()> {
    let mut source = snapshot::Source::open(Path::new(&request.path))?;
    if source.database_len < codec::PAGE_SIZE as u64
        || !source.database_len.is_multiple_of(codec::PAGE_SIZE as u64)
    {
        return Err("DATABASE_FORMAT_UNSUPPORTED");
    }
    let source_sha256 = source.digest()?;
    source
        .database
        .seek(SeekFrom::Start(0))
        .map_err(|_| "SOURCE_READ_FAILED")?;
    let mut page = Zeroizing::new([0u8; codec::PAGE_SIZE]);
    source
        .database
        .read_exact(page.as_mut())
        .map_err(|_| "SOURCE_READ_FAILED")?;
    let plain = page.starts_with(b"SQLite format 3\0");
    if plain && (!request.allow_plaintext || request.key_hex.is_some()) {
        return Err("PLAINTEXT_NOT_AUTHORIZED");
    }
    let decoder = if plain {
        None
    } else {
        Some(codec::PageDecoder::new(
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            page.as_ref(),
        )?)
    };
    let wal = if let Some(file) = &mut source.wal {
        codec::scan_wal(file, source.wal_len, decoder.as_ref(), |_, _, _| Ok(()))?
    } else {
        codec::WalInfo::default()
    };
    let final_length = if wal.committed == 0 {
        source.database_len as usize
    } else {
        wal.pages * codec::PAGE_SIZE
    };
    let capacity = (source.database_len as usize).max(final_length);
    // Reserve 32 MiB identity maps, 64 MiB SQLite work/cache and 16 MiB for
    // decoded fields / serialization / in-flight frames. No cipher DB/WAL Vec.
    if capacity as u64 + 112 * 1024 * 1024 > snapshot::TOTAL_BUDGET_BYTES
        || final_length < codec::PAGE_SIZE
    {
        return Err("DATABASE_LIMIT_EXCEEDED");
    }
    let mut image = Image::new(capacity)?;
    source
        .database
        .seek(SeekFrom::Start(0))
        .map_err(|_| "SOURCE_READ_FAILED")?;
    for number in 1..=source.database_len as usize / codec::PAGE_SIZE {
        source
            .database
            .read_exact(page.as_mut())
            .map_err(|_| "SOURCE_READ_FAILED")?;
        let dest = &mut image.bytes()[(number - 1) * codec::PAGE_SIZE..number * codec::PAGE_SIZE];
        if let Some(d) = &decoder {
            d.page_into(number as u32, page.as_ref(), dest)?;
        } else {
            dest.copy_from_slice(page.as_ref());
        }
    }
    if let Some(file) = &mut source.wal {
        let replay = codec::scan_wal(
            file,
            source.wal_len,
            decoder.as_ref(),
            |index, number, bytes| {
                if index <= wal.committed && number as usize <= wal.pages {
                    let start = (number as usize - 1) * codec::PAGE_SIZE;
                    image.bytes()[start..start + codec::PAGE_SIZE].copy_from_slice(bytes);
                }
                Ok(())
            },
        )?;
        if replay != wal {
            return Err("SOURCE_CHANGED");
        }
    }
    let data = image.bytes();
    if &data[..16] != b"SQLite format 3\0"
        || data[16..18] != [0x10, 0]
        || !matches!(data[20], 0 | 80)
    {
        return Err("DATABASE_FORMAT_UNSUPPORTED");
    }
    data[18] = 1;
    data[19] = 1;
    let mut connection =
        Connection::open_in_memory().map_err(|e| codec::sqlite_error(e, "DATABASE_OPEN_FAILED"))?;
    connection
        .set_limit(
            rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
            2 * 1024 * 1024,
        )
        .map_err(|_| "LIMIT_CONFIGURATION_FAILED")?;
    connection
        .set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_COLUMN, 256)
        .map_err(|_| "LIMIT_CONFIGURATION_FAILED")?;
    connection
        .set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH, 16384)
        .map_err(|_| "LIMIT_CONFIGURATION_FAILED")?;
    connection
        .deserialize("main", image.transfer(final_length), true)
        .map_err(|e| codec::sqlite_error(e, "DATABASE_OPEN_FAILED"))?;
    // Worker is isolated; SQLite's allocation ceiling is process-global. Include
    // the SQLite-owned image in that limit, leaving headroom for Rust allocations.
    connection.execute_batch("PRAGMA hard_heap_limit=402653184; PRAGMA cache_size=-8192; PRAGMA trusted_schema=OFF; PRAGMA query_only=ON; PRAGMA temp_store=MEMORY; PRAGMA mmap_size=0;").map_err(|e| codec::sqlite_error(e, "READ_ONLY_REQUIRED"))?;
    schema::ensure_supported(&connection)?;
    codec::check_database(&connection)?;
    let meta = schema::StreamMetadata {
        source_sha256: source_sha256.clone(),
        encryption: if plain {
            "plaintext_explicit"
        } else {
            "sqlcipher4_rawkey_4096_sha512"
        }
        .into(),
        wal_committed_frames: wal.committed,
    };
    schema::read_stream(&connection, meta, &mut |event| {
        if matches!(event, HistoryReadEvent::End { .. }) && source.digest()? != source_sha256 {
            return Err("SOURCE_CHANGED");
        }
        visitor(event)
    })
}
