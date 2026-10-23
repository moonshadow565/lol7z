//! [`Writer`]: a package's chunk data laid out after room for its tables, then the
//! tables.

use crate::toc::{Compression, Header, NONE, Record, Tables, VERSION, toc_offset};
use lol7z_api::{Error, OutStream, Result};
use lol7z_wad::PathHash;
use lol7z_wad::encode::Counting;
use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom, Write};

/// What a record is besides its stored bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct At {
    pub hash: PathHash,
    pub path_index: u32,
    pub layer_index: u32,
    pub wad_index: u32,
}

/// Lays runs out one after another. One whose content hashes like an earlier one is
/// written over by the next, so equal content is kept once.
pub(crate) struct Writer {
    w: OutStream,
    /// Header, signature, tables and padding, as they go in front.
    head: Vec<u8>,
    /// Where the next run goes.
    pos: u64,
    /// How far the file has been written.
    end: u64,
    records: Vec<Record>,
    /// By `(uncompressed_checksum, uncompressed_size)`.
    written: BTreeMap<(u64, u64), Record>,
}

impl Writer {
    pub(crate) fn new(
        mut w: OutStream,
        tables: &Tables,
        signature: &[u8],
        count: usize,
    ) -> Result<Self> {
        let header = Header {
            version: VERSION,
            signature_size: u32::try_from(signature.len()).map_err(|_| Error::INVALID_ARG)?,
            chunk_count: u32::try_from(count).map_err(|_| Error::INVALID_ARG)?,
        };
        let mut head = Vec::new();
        head.try_reserve_exact(toc_offset(20 + signature.len() as u64 + tables.size()) as usize)?;
        header.write(&mut head)?;
        head.extend_from_slice(signature);
        tables.write(&mut head)?;
        head.resize(toc_offset(head.len() as u64) as usize, 0);
        let pos = head.len() as u64 + count as u64 * crate::toc::RECORD_SIZE as u64;
        w.seek(SeekFrom::Start(pos))?;
        let mut records = Vec::new();
        records.try_reserve_exact(count)?;
        Ok(Self {
            w,
            head,
            pos,
            end: pos,
            records,
            written: BTreeMap::new(),
        })
    }

    /// Streams one run in through `fill`, which returns its compression, content size
    /// and content checksum.
    pub(crate) fn add(
        &mut self,
        at: At,
        fill: impl FnOnce(&mut Counting<'_>) -> Result<(Compression, u64, u64)>,
    ) -> Result<Record> {
        let mut out = Counting::new(&mut self.w);
        let (compression, uncompressed_size, uncompressed_checksum) = fill(&mut out)?;
        let (compressed_size, compressed_checksum) = out.finish();
        self.end = self.end.max(self.pos + compressed_size);
        let key = (uncompressed_checksum, uncompressed_size);
        if let Some(&same) = self.written.get(&key) {
            self.w.seek(SeekFrom::Start(self.pos))?;
            return Ok(self.alias(at, same));
        }
        let r = Record {
            path_hash: at.hash,
            data_offset: self.pos,
            compression,
            compressed_size,
            uncompressed_size,
            compressed_checksum,
            uncompressed_checksum,
            path_index: at.path_index,
            layer_index: at.layer_index,
            wad_index: at.wad_index,
        };
        self.pos += compressed_size;
        self.written.insert(key, r);
        self.records.push(r);
        Ok(r)
    }

    /// Another record for a run already written.
    pub(crate) fn alias(&mut self, at: At, same: Record) -> Record {
        let r = Record {
            path_hash: at.hash,
            path_index: at.path_index,
            layer_index: at.layer_index,
            wad_index: at.wad_index,
            ..same
        };
        self.records.push(r);
        r
    }

    /// Cuts off what a final duplicate left behind (zeroes it on a stream that can't be
    /// cut), then writes the header, tables and records: meta chunks first, then by
    /// WAD, layer and path hash.
    pub(crate) fn finish(mut self) -> Result<()> {
        if self.end > self.pos
            && let Err(e) = self.w.set_len(self.pos)
        {
            let e = Error::from(e);
            if e != Error::NOT_IMPL {
                return Err(e);
            }
            self.w.seek(SeekFrom::Start(self.pos))?;
            io::copy(&mut io::repeat(0).take(self.end - self.pos), &mut self.w)?;
        }
        let first = |i: u32| if i == NONE { 0 } else { i + 1 };
        self.records
            .sort_unstable_by_key(|r| (first(r.wad_index), first(r.layer_index), r.path_hash));
        self.w.seek(SeekFrom::Start(0))?;
        self.w.write_all(&self.head)?;
        for r in &self.records {
            r.write(&mut self.w)?;
        }
        Ok(())
    }
}
