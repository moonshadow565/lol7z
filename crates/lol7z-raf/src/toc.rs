//! The index file, as Fantome.Libraries.RADS (`RAF.cs`) reads it. Little-endian.
//!
//! Header: u32 magic `0x18BE0EF0`, i32 version (only 1 exists), i32 manager index, u32
//! file list offset, u32 path list offset. File list: i32 count, then per file u32 path
//! hash, u32 offset into the data file, u32 length there, u32 path list index. Path
//! list: u32 byte size, u32 count, then per path u32 offset from the list's start and
//! u32 length counting the terminating zero, followed by the strings. The file list is
//! sorted by hash, a 4-bit shift-xor over the lowercased path.

use lol7z_api::{Error, Result};

pub const MAGIC: [u8; 4] = 0x18BE_0EF0u32.to_le_bytes();
pub const HEADER_SIZE: usize = 20;
pub const ENTRY_SIZE: usize = 16;
pub const PATH_SIZE: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub hash: u32,
    /// Into the data file.
    pub offset: u32,
    pub length: u32,
    /// As stored, `/`-separated.
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toc {
    pub version: i32,
    pub manager_index: i32,
    pub entries: Vec<Entry>,
}

/// Parses the whole index. `NotArchive` without the magic; a read past the end is
/// `UnexpectedEnd`, and anything that contradicts itself is `HeadersError`.
pub fn read(buf: &[u8]) -> Result<Toc> {
    if buf.len() < MAGIC.len() || buf[..MAGIC.len()] != MAGIC {
        return Err(Error::NotArchive);
    }
    let ix = Index(buf);
    let version = ix.u32(4)? as i32;
    let manager_index = ix.u32(8)? as i32;
    let files_at = ix.u32(12)? as usize;
    let paths_at = ix.u32(16)? as usize;

    let count = ix.count(files_at, ENTRY_SIZE)?;
    let path_count = ix.count(
        paths_at.checked_add(4).ok_or(Error::UNEXPECTED_END)?,
        PATH_SIZE,
    )?;
    let table_at = paths_at + PATH_SIZE;

    // Paths can share bytes, so their total is bounded by the index, not by the count:
    // a file list that claims more string than the whole index holds is lying.
    let mut budget = buf.len();
    let mut entries = Vec::new();
    entries.try_reserve_exact(count)?;
    for i in 0..count {
        let at = files_at + 4 + i * ENTRY_SIZE;
        let hash = ix.u32(at)?;
        let offset = ix.u32(at + 4)?;
        let length = ix.u32(at + 8)?;
        let path_index = ix.u32(at + 12)? as usize;
        if path_index >= path_count {
            return Err(Error::HEADERS_ERROR);
        }
        let p = table_at + path_index * PATH_SIZE;
        let path_offset = ix.u32(p)? as usize;
        let path_len = ix.u32(p + 4)? as usize;
        if path_len == 0 {
            return Err(Error::HEADERS_ERROR);
        }
        let start = paths_at
            .checked_add(path_offset)
            .ok_or(Error::UNEXPECTED_END)?;
        let bytes = &ix.bytes(start, path_len)?[..path_len - 1];
        budget = budget
            .checked_sub(bytes.len())
            .ok_or(Error::HEADERS_ERROR)?;
        let mut path = String::new();
        path.try_reserve_exact(bytes.len())?;
        path.push_str(&String::from_utf8_lossy(bytes));
        entries.push(Entry {
            hash,
            offset,
            length,
            path,
        });
    }
    Ok(Toc {
        version,
        manager_index,
        entries,
    })
}

/// The hash the file list is keyed and sorted by.
pub fn path_hash(path: &str) -> u32 {
    let mut hash = 0u32;
    let mut utf16 = [0u16; 2];
    for c in path.chars().flat_map(char::to_lowercase) {
        for &c in c.encode_utf16(&mut utf16).iter() {
            hash = (hash << 4).wrapping_add(u32::from(c));
            let top = hash & 0xF000_0000;
            if top != 0 {
                hash ^= top >> 24;
                hash ^= top;
            }
        }
    }
    hash
}

struct Index<'a>(&'a [u8]);

impl Index<'_> {
    fn bytes(&self, at: usize, len: usize) -> Result<&[u8]> {
        at.checked_add(len)
            .and_then(|end| self.0.get(at..end))
            .ok_or(Error::UNEXPECTED_END)
    }

    fn u32(&self, at: usize) -> Result<u32> {
        let b = self.bytes(at, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A count at `at` of records `each` bytes long that follow it: more than fit is a
    /// cut file.
    fn count(&self, at: usize, each: usize) -> Result<usize> {
        let n = self.u32(at)? as usize;
        let room = self.0.len().saturating_sub(at + 4) / each;
        if n > room {
            return Err(Error::UNEXPECTED_END);
        }
        Ok(n)
    }
}
