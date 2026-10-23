//! Modpkg header, string tables and table of contents, format version 1. Little-endian
//! throughout.
//!
//! Header: `_modpkg_`, version, signature size, chunk count (u32 each), signature. Then
//! the layer table (count; counted name and i32 priority each), the path table and the
//! WAD table (count; NUL-terminated names), zero padding to a multiple of 8, and one
//! 61-byte record per chunk: path hash, data offset (u64), compression (u8), compressed
//! and uncompressed size and XXH3 checksum (u64 each), path, layer and WAD index (u32).

use lol7z_api::{Error, ErrorFlags};
use lol7z_wad::PathHash;
use open_enum::open_enum;
use scroll::{LE, Pread};
use std::io::{self, BufRead, Read, Write};

pub const MAGIC: &[u8; 8] = b"_modpkg_";
pub const VERSION: u32 = 1;
pub const HEADER_SIZE: usize = 20;
pub const RECORD_SIZE: usize = 61;
/// The layer or WAD index of a chunk that has none.
pub const NONE: u32 = u32::MAX;
/// Longest name read from a table, in bytes: a guard against hostile counts only.
pub const MAX_NAME: usize = 1 << 16;

#[open_enum]
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum Compression {
    Raw = 0,
    Zstd = 1,
}

impl Compression {
    pub fn name(self) -> &'static str {
        match self {
            Compression::Raw => "Store",
            Compression::Zstd => "Zstd",
            _ => "Unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub path_hash: PathHash,
    pub data_offset: u64,
    pub compression: Compression,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub compressed_checksum: u64,
    pub uncompressed_checksum: u64,
    pub path_index: u32,
    pub layer_index: u32,
    pub wad_index: u32,
}

impl Record {
    pub fn parse(raw: &[u8]) -> Result<Record, TocError> {
        let o = &mut 0;
        Ok(Record {
            path_hash: PathHash(raw.gread_with(o, LE)?),
            data_offset: raw.gread_with(o, LE)?,
            compression: Compression(raw.gread(o)?),
            compressed_size: raw.gread_with(o, LE)?,
            uncompressed_size: raw.gread_with(o, LE)?,
            compressed_checksum: raw.gread_with(o, LE)?,
            uncompressed_checksum: raw.gread_with(o, LE)?,
            path_index: raw.gread_with(o, LE)?,
            layer_index: raw.gread_with(o, LE)?,
            wad_index: raw.gread_with(o, LE)?,
        })
    }

    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        let mut b = [0u8; RECORD_SIZE];
        b[0..8].copy_from_slice(&self.path_hash.0.to_le_bytes());
        b[8..16].copy_from_slice(&self.data_offset.to_le_bytes());
        b[16] = self.compression.0;
        b[17..25].copy_from_slice(&self.compressed_size.to_le_bytes());
        b[25..33].copy_from_slice(&self.uncompressed_size.to_le_bytes());
        b[33..41].copy_from_slice(&self.compressed_checksum.to_le_bytes());
        b[41..49].copy_from_slice(&self.uncompressed_checksum.to_le_bytes());
        b[49..53].copy_from_slice(&self.path_index.to_le_bytes());
        b[53..57].copy_from_slice(&self.layer_index.to_le_bytes());
        b[57..61].copy_from_slice(&self.wad_index.to_le_bytes());
        w.write_all(&b)
    }

    /// Belongs to no layer and no WAD.
    pub fn is_meta(&self) -> bool {
        self.layer_index == NONE && self.wad_index == NONE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TocError {
    NotModpkg,
    /// A package, but a format version this crate doesn't read.
    Unsupported,
    Truncated,
    OutOfMemory,
    /// A field that can't be right.
    Bad(&'static str),
}

impl From<scroll::Error> for TocError {
    fn from(_: scroll::Error) -> Self {
        TocError::Truncated
    }
}

impl From<std::collections::TryReserveError> for TocError {
    fn from(_: std::collections::TryReserveError) -> Self {
        TocError::OutOfMemory
    }
}

impl From<io::Error> for TocError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::UnexpectedEof => TocError::Truncated,
            io::ErrorKind::OutOfMemory => TocError::OutOfMemory,
            _ => TocError::Bad("read failed"),
        }
    }
}

impl From<TocError> for Error {
    fn from(e: TocError) -> Self {
        match e {
            TocError::NotModpkg => Error::NotArchive,
            TocError::Unsupported => Error::Damaged(ErrorFlags::Unsupported),
            TocError::Truncated => Error::UNEXPECTED_END,
            TocError::OutOfMemory => Error::OUT_OF_MEMORY,
            TocError::Bad(_) => Error::HEADERS_ERROR,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    pub signature_size: u32,
    pub chunk_count: u32,
}

impl Header {
    pub fn parse(head: &[u8]) -> Result<Header, TocError> {
        if head.len() < MAGIC.len() || &head[..MAGIC.len()] != MAGIC {
            return Err(TocError::NotModpkg);
        }
        let o = &mut MAGIC.len();
        let version = head.gread_with(o, LE)?;
        if version != VERSION {
            return Err(TocError::Unsupported);
        }
        Ok(Header {
            version,
            signature_size: head.gread_with(o, LE)?,
            chunk_count: head.gread_with(o, LE)?,
        })
    }

    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(MAGIC)?;
        w.write_all(&self.version.to_le_bytes())?;
        w.write_all(&self.signature_size.to_le_bytes())?;
        w.write_all(&self.chunk_count.to_le_bytes())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub name: String,
    pub priority: i32,
}

/// The three string tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tables {
    pub layers: Vec<Layer>,
    pub paths: Vec<String>,
    pub wads: Vec<String>,
}

impl Tables {
    /// Reads the tables from `r`, at the first byte after the signature, with `budget`
    /// bytes left in the file.
    pub fn read(r: &mut impl BufRead, budget: u64) -> Result<Tables, TocError> {
        let mut buf = Vec::new();
        let mut t = Tables::default();
        // A layer entry is at least 8 bytes, a path or WAD at least 1.
        let layers = count(r, budget / 8)?;
        t.layers.try_reserve_exact(layers)?;
        for _ in 0..layers {
            let len = count(r, MAX_NAME as u64)?;
            buf.clear();
            buf.resize(len, 0);
            r.read_exact(&mut buf)?;
            let name = string(&buf)?;
            let mut p = [0; 4];
            r.read_exact(&mut p)?;
            t.layers.push(Layer {
                name,
                priority: i32::from_le_bytes(p),
            });
        }
        for table in [&mut t.paths, &mut t.wads] {
            let n = count(r, budget)?;
            table.try_reserve_exact(n)?;
            for _ in 0..n {
                buf.clear();
                r.take(MAX_NAME as u64 + 1).read_until(0, &mut buf)?;
                let Some(bytes) = buf.strip_suffix(&[0]) else {
                    if buf.len() > MAX_NAME {
                        return Err(TocError::Bad("name too long"));
                    }
                    return Err(TocError::Truncated);
                };
                table.push(string(bytes)?);
            }
        }
        Ok(t)
    }

    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(&(self.layers.len() as u32).to_le_bytes())?;
        for l in &self.layers {
            w.write_all(&(l.name.len() as u32).to_le_bytes())?;
            w.write_all(l.name.as_bytes())?;
            w.write_all(&l.priority.to_le_bytes())?;
        }
        for table in [&self.paths, &self.wads] {
            w.write_all(&(table.len() as u32).to_le_bytes())?;
            for s in table {
                w.write_all(s.as_bytes())?;
                w.write_all(&[0])?;
            }
        }
        Ok(())
    }

    /// Bytes the tables take written.
    pub fn size(&self) -> u64 {
        let layers: u64 = self.layers.iter().map(|l| 8 + l.name.len() as u64).sum();
        let names: u64 = self
            .paths
            .iter()
            .chain(&self.wads)
            .map(|s| s.len() as u64 + 1)
            .sum();
        12 + layers + names
    }
}

/// Where the records start in a file whose header and tables take `before` bytes.
pub fn toc_offset(before: u64) -> u64 {
    before.div_ceil(8) * 8
}

fn count(r: &mut impl Read, max: u64) -> Result<usize, TocError> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    let n = u64::from(u32::from_le_bytes(b));
    // More entries than the file has bytes for.
    if n > max {
        return Err(TocError::Truncated);
    }
    Ok(n as usize)
}

fn string(bytes: &[u8]) -> Result<String, TocError> {
    let s = str::from_utf8(bytes).map_err(|_| TocError::Bad("name not UTF-8"))?;
    let mut out = String::new();
    out.try_reserve_exact(s.len())?;
    out.push_str(s);
    Ok(out)
}
