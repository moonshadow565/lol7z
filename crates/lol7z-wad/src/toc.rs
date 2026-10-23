//! WAD header and table of contents, versions 1 through 3.4. Little-endian throughout.
//!
//! Header: `RW`, major, minor, then by major version:
//! - 1: entry offset (u16), entry size (u16), count (u32)
//! - 2: signature (84), checksum (u64), entry offset, entry size, count
//! - 3: signature (256), checksum (u64), count; entries follow at 272, 32 bytes each
//!
//! Entry: hash (u64), offset, size, decompressed size (u32 each), type in the low nibble
//! of a byte whose high nibble is the subchunk count, then by version:
//! - 1: 3 bytes padding
//! - 2: duplicate flag (u8), subchunk index (u16), checksum (u64) when the entry is 32 bytes
//! - 3.0 to 3.3: duplicate flag, subchunk index (u16), checksum (u64)
//! - 3.4: subchunk index as 3 bytes in high, low, middle order, checksum (u64)

use crate::hash::PathHash;
use lol7z_api::{Error, ErrorFlags};
use open_enum::open_enum;
use scroll::{LE, Pread};
use std::io::{self, Write};
use xxhash_rust::xxh3::Xxh3;

pub const MAGIC: &[u8; 2] = b"RW";
pub const HEADER_V3_SIZE: usize = 272;
pub const ENTRY_V3_SIZE: usize = 32;

#[open_enum]
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Raw = 0,
    Gzip = 1,
    Link = 2,
    Zstd = 3,
    ZstdMulti = 4,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Raw => "Store",
            Kind::Gzip => "GZip",
            Kind::Link => "Link",
            Kind::Zstd => "Zstd",
            Kind::ZstdMulti => "ZstdMulti",
            _ => "Unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub hash: PathHash,
    pub offset: u32,
    pub size: u32,
    pub size_decompressed: u32,
    pub kind: Kind,
    pub subchunk_count: u8,
    pub subchunk_index: u32,
    /// SHA-256 of the stored bytes, first 8, in 2.x and 3.0; XXH3 of them from 3.1 on.
    pub checksum: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TocError {
    /// Not a WAD at all.
    NotWad,
    /// A WAD, but a version this crate doesn't read.
    Unsupported,
    /// Cut off inside the header or an entry.
    Truncated,
    /// A field that can't be right.
    Bad(&'static str),
}

impl From<scroll::Error> for TocError {
    fn from(_: scroll::Error) -> Self {
        TocError::Truncated
    }
}

impl From<TocError> for Error {
    fn from(e: TocError) -> Self {
        match e {
            TocError::NotWad => Error::NotArchive,
            TocError::Unsupported => Error::Damaged(ErrorFlags::Unsupported),
            TocError::Truncated => Error::UNEXPECTED_END,
            TocError::Bad(_) => Error::HEADERS_ERROR,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub major: u8,
    pub minor: u8,
    pub signature: [u8; 256],
    pub checksum: u64,
    pub entry_offset: u32,
    pub entry_size: u32,
    pub count: u32,
}

impl Header {
    /// Enough bytes for any version's header.
    pub const MAX_SIZE: usize = HEADER_V3_SIZE;

    pub fn parse(head: &[u8]) -> Result<Header, TocError> {
        if head.len() < 4 || &head[..2] != MAGIC {
            return Err(TocError::NotWad);
        }
        let (major, minor) = (head[2], head[3]);
        let o = &mut 4;
        let mut signature = [0; 256];
        let mut checksum = 0;
        let (entry_offset, entry_size, count) = match major {
            0 | 1 => (
                head.gread_with::<u16>(o, LE)?.into(),
                head.gread_with::<u16>(o, LE)?.into(),
                head.gread_with(o, LE)?,
            ),
            2 => {
                signature[..84].copy_from_slice(head.gread_with::<&[u8]>(o, 84)?);
                checksum = head.gread_with(o, LE)?;
                (
                    head.gread_with::<u16>(o, LE)?.into(),
                    head.gread_with::<u16>(o, LE)?.into(),
                    head.gread_with(o, LE)?,
                )
            }
            3 => {
                signature.copy_from_slice(head.gread_with::<&[u8]>(o, 256)?);
                checksum = head.gread_with(o, LE)?;
                (
                    HEADER_V3_SIZE as u32,
                    ENTRY_V3_SIZE as u32,
                    head.gread_with(o, LE)?,
                )
            }
            _ => return Err(TocError::Unsupported),
        };
        let min_entry = if major >= 3 { ENTRY_V3_SIZE } else { 24 };
        if (entry_size as usize) < min_entry {
            return Err(TocError::Bad("entry size"));
        }
        if (entry_offset as usize) < *o {
            return Err(TocError::Bad("entry offset"));
        }
        Ok(Header {
            major,
            minor,
            signature,
            checksum,
            entry_offset,
            entry_size,
            count,
        })
    }

    pub fn parse_entry(&self, raw: &[u8]) -> Result<Entry, TocError> {
        let o = &mut 0;
        let hash = PathHash(raw.gread_with(o, LE)?);
        let offset = raw.gread_with(o, LE)?;
        let size = raw.gread_with(o, LE)?;
        let size_decompressed = raw.gread_with(o, LE)?;
        let flags: u8 = raw.gread(o)?;
        let kind = Kind(flags & 0xF);
        let (subchunk_count, subchunk_index, checksum) = match (self.major, self.minor) {
            (0 | 1, _) => (0, 0, 0),
            (2, _) => {
                *o += 1;
                let index = raw.gread_with::<u16>(o, LE)?.into();
                let checksum = if raw.len() >= ENTRY_V3_SIZE {
                    raw.gread_with(o, LE)?
                } else {
                    0
                };
                (flags >> 4, index, checksum)
            }
            (3, 0..=3) => {
                *o += 1;
                let index = raw.gread_with::<u16>(o, LE)?.into();
                (flags >> 4, index, raw.gread_with(o, LE)?)
            }
            _ => {
                let b: &[u8] = raw.gread_with(o, 3)?;
                let index = u32::from(b[0]) << 16 | u32::from(b[2]) << 8 | u32::from(b[1]);
                (flags >> 4, index, raw.gread_with(o, LE)?)
            }
        };
        Ok(Entry {
            hash,
            offset,
            size,
            size_decompressed,
            kind,
            subchunk_count,
            subchunk_index,
            checksum,
        })
    }
}

/// A 3.4 header and table to write. `entries` must be sorted by hash.
pub struct Toc<'a> {
    pub signature: &'a [u8; 256],
    pub entries: &'a [Entry],
}

impl Toc<'_> {
    /// Bytes from the start of the file to the first entry's data.
    pub fn size(count: usize) -> usize {
        HEADER_V3_SIZE + ENTRY_V3_SIZE * count
    }

    /// The header checksum: XXH3 of the version bytes and each entry's hash and checksum,
    /// as cslol-manager writes it.
    pub fn checksum(&self) -> u64 {
        let mut h = Xxh3::new();
        h.update(&[MAGIC[0], MAGIC[1], 3, 4]);
        for e in self.entries {
            h.update(&e.hash.0.to_le_bytes());
            h.update(&e.checksum.to_le_bytes());
        }
        h.digest()
    }

    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(&[MAGIC[0], MAGIC[1], 3, 4])?;
        w.write_all(self.signature)?;
        w.write_all(&self.checksum().to_le_bytes())?;
        w.write_all(&(self.entries.len() as u32).to_le_bytes())?;
        for e in self.entries {
            let mut b = [0u8; ENTRY_V3_SIZE];
            b[0..8].copy_from_slice(&e.hash.0.to_le_bytes());
            b[8..12].copy_from_slice(&e.offset.to_le_bytes());
            b[12..16].copy_from_slice(&e.size.to_le_bytes());
            b[16..20].copy_from_slice(&e.size_decompressed.to_le_bytes());
            b[20] = e.subchunk_count << 4 | e.kind.0;
            b[21] = (e.subchunk_index >> 16) as u8;
            b[22] = e.subchunk_index as u8;
            b[23] = (e.subchunk_index >> 8) as u8;
            b[24..32].copy_from_slice(&e.checksum.to_le_bytes());
            w.write_all(&b)?;
        }
        Ok(())
    }
}
