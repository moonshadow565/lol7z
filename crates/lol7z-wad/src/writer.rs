//! [`Writer`]: a 3.4 WAD's entries laid out, then its table.

use crate::encode::Counting;
use crate::hash::PathHash;
use crate::toc::{Entry, Kind, Toc};
use lol7z_api::{Error, OutStream, Result};
use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom};

/// Lays entries out one after another. One whose stored bytes hash like an earlier one
/// of the same kind and sizes is written over by the next, so equal data is kept once.
pub(crate) struct Writer {
    w: OutStream,
    /// Where the next entry goes.
    pos: u64,
    /// How far the file has been written.
    end: u64,
    entries: Vec<Entry>,
    // Inserts and lookups interleave, which a sorted Vec would make quadratic.
    written: BTreeMap<u64, Entry>,
}

impl Writer {
    pub(crate) fn new(mut w: OutStream, count: usize) -> Result<Self> {
        let pos = Toc::size(count) as u64;
        w.seek(SeekFrom::Start(pos))?;
        let mut entries = Vec::new();
        entries.try_reserve_exact(count)?;
        Ok(Self {
            w,
            pos,
            end: pos,
            entries,
            written: BTreeMap::new(),
        })
    }

    /// Streams one entry's stored bytes in through `fill`, which returns its kind,
    /// decompressed size, subchunk count and subchunk index.
    pub(crate) fn add(
        &mut self,
        hash: PathHash,
        fill: impl FnOnce(&mut Counting<'_>) -> Result<(Kind, u64, u8, u32)>,
    ) -> Result<Entry> {
        let mut out = Counting::new(&mut self.w);
        let (kind, size_decompressed, subchunk_count, subchunk_index) = fill(&mut out)?;
        let (size, checksum) = out.finish();
        self.end = self.end.max(self.pos + size);
        let shape = (
            kind,
            size,
            size_decompressed,
            subchunk_count,
            subchunk_index,
        );
        if let Some(&same) = self.written.get(&checksum)
            && shape == shape_of(&same)
        {
            self.w.seek(SeekFrom::Start(self.pos))?;
            return Ok(self.alias(hash, same));
        }
        let fits = (
            u32::try_from(self.pos),
            u32::try_from(size),
            u32::try_from(size_decompressed),
        );
        let (Ok(offset), Ok(size), Ok(size_decompressed)) = fits else {
            return Err(Error::INVALID_ARG);
        };
        let e = Entry {
            hash,
            offset,
            size,
            size_decompressed,
            kind,
            subchunk_count,
            subchunk_index,
            checksum,
        };
        self.pos += u64::from(size);
        self.written.entry(checksum).or_insert(e);
        self.entries.push(e);
        Ok(e)
    }

    /// Another row for data already written.
    pub(crate) fn alias(&mut self, hash: PathHash, same: Entry) -> Entry {
        let e = Entry { hash, ..same };
        self.entries.push(e);
        e
    }

    /// Cuts off what a final duplicate left behind (zeroes it on a stream that can't be
    /// cut), then writes the header and table.
    pub(crate) fn finish(mut self, signature: &[u8; 256]) -> Result<()> {
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
        self.entries.sort_unstable_by_key(|e| e.hash);
        self.w.seek(SeekFrom::Start(0))?;
        let toc = Toc {
            signature,
            entries: &self.entries,
        };
        Ok(toc.write(&mut self.w)?)
    }
}

fn shape_of(e: &Entry) -> (Kind, u64, u64, u8, u32) {
    (
        e.kind,
        e.size.into(),
        e.size_decompressed.into(),
        e.subchunk_count,
        e.subchunk_index,
    )
}
