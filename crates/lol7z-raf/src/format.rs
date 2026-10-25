//! [`Raf`]: the format 7-Zip lists, and how it opens.

use crate::archive::{Method, RafArchive};
use crate::decode::{self, ZLIB_HEADER, ZLIB_TRAILER};
use crate::toc::{self, MAGIC};
use lol7z_api::{Error, ErrorFlags, Ext, Format, GUID, InStream, OpenCallback, Options, Result};

pub struct Raf;

/// The data file's suffix after the index's name: `Archive_1.raf` → `Archive_1.raf.dat`.
pub const DATA_SUFFIX: &str = ".dat";
/// Entries between progress reports while the data file is sniffed.
const PROGRESS_EVERY: usize = 256;

impl Format for Raf {
    const NAME: &'static str = "raf";
    const CLASS_ID: GUID = GUID::from_u128(0x6c746b37_7a00_4a1b_9c3d_726166000001);
    const EXTENSIONS: &'static [Ext] = &[Ext::new("raf")];
    const SIGNATURES: &'static [&'static [u8]] = &[&MAGIC];

    type Archive = RafArchive;

    fn open(stream: InStream, cb: &OpenCallback<'_>, _options: &Options) -> Result<RafArchive> {
        let index_size = stream.size()?;
        let mut head = [0u8; 4];
        if index_size < 4 {
            return Err(Error::NotArchive);
        }
        stream.read_exact_at(0, &mut head)?;
        if head != MAGIC {
            return Err(Error::NotArchive);
        }

        // The index is a table a few hundred KB long next to its data file: read whole.
        let len = usize::try_from(index_size).map_err(|_| Error::OUT_OF_MEMORY)?;
        let mut buf: Vec<u8> = Vec::new();
        buf.try_reserve_exact(len)?;
        buf.resize(len, 0);
        stream.read_exact_at(0, &mut buf)?;
        let toc = toc::read(&buf)?;
        drop(buf);

        if toc
            .entries
            .iter()
            .any(|e| toc::path_hash(&e.path) != e.hash)
        {
            cb.warning(ErrorFlags::HeadersError);
            cb.warning_message("path hashes don't match their paths");
        }

        // The data is a second file next to the index.
        let volumes = cb.volumes();
        let name = volumes.as_ref().and_then(|v| v.name().ok().flatten());
        let data_name = name.as_deref().map_or_else(
            || format!("<archive>{DATA_SUFFIX}"),
            |n| format!("{n}{DATA_SUFFIX}"),
        );
        let data = match &volumes {
            // A data file that exists but won't open (locked, denied) lists like a missing one.
            Some(v) if name.is_some() => v.open(&data_name).unwrap_or(None),
            _ => None,
        };
        let Some(data) = data else {
            cb.error(ErrorFlags::UnexpectedEnd);
            cb.error_message(format!("can't open data file {data_name}"));
            let mut methods = Vec::new();
            methods.try_reserve_exact(toc.entries.len())?;
            methods.resize(toc.entries.len(), Method::Unknown);
            return RafArchive::build(None, data_name, 0, toc, methods, index_size);
        };

        // The index doesn't say which entries are compressed: look at each one's first
        // bytes, in file order.
        let data_size = data.size()?;
        let count = toc.entries.len();
        cb.set_total(Some(count as u64), Some(data_size))?;
        let mut order: Vec<u32> = Vec::new();
        order.try_reserve_exact(count)?;
        order.extend(0..count as u32);
        order.sort_unstable_by_key(|&i| toc.entries[i as usize].offset);
        let mut methods = Vec::new();
        methods.try_reserve_exact(count)?;
        methods.resize(count, Method::Store);
        let mut cut = false;
        for (n, &i) in order.iter().enumerate() {
            if n % PROGRESS_EVERY == 0 {
                let e = &toc.entries[i as usize];
                cb.set_completed(Some(n as u64), Some(e.offset.into()))?;
            }
            let e = &toc.entries[i as usize];
            let (offset, length) = (u64::from(e.offset), u64::from(e.length));
            if offset.saturating_add(length) > data_size {
                cut = true;
            }
            if length < ZLIB_HEADER + ZLIB_TRAILER || offset + ZLIB_HEADER > data_size {
                continue;
            }
            let mut head = [0u8; 2];
            data.read_exact_at(offset, &mut head)?;
            if decode::is_zlib(head) {
                methods[i as usize] = Method::Zlib;
            }
        }
        if cut {
            cb.error(ErrorFlags::UnexpectedEnd);
            cb.error_message(format!("{data_name} is shorter than the index says"));
        }
        cb.set_completed(Some(count as u64), Some(data_size))?;
        RafArchive::build(Some(data), data_name, data_size, toc, methods, index_size)
    }
}
