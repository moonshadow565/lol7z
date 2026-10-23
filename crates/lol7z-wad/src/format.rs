//! [`Wad`]: the format 7-Zip lists, and how it opens.

use crate::archive::{Item, WadArchive};
use crate::names::Names;
use crate::toc::Header;
use lol7z_api::{Error, ErrorFlags, Ext, Format, GUID, InStream, OpenCallback, Options, Result};

/// Entries read per table read, between progress calls.
const CHUNK: u32 = 4096;

pub struct Wad;

impl Format for Wad {
    const NAME: &'static str = "WAD";
    const CLASS_ID: GUID = GUID::from_u128(0x6c746b37_7a00_4a1b_9c3d_776164000001);
    const EXTENSIONS: &'static [Ext] = &[Ext::new("wad"), Ext::new("client")];
    const SIGNATURES: &'static [&'static [u8]] = &[b"RW\x01", b"RW\x02", b"RW\x03"];

    type Archive = WadArchive;

    fn open(stream: InStream, cb: &OpenCallback<'_>, _options: &Options) -> Result<WadArchive> {
        let file_size = stream.size()?;
        let mut head = [0u8; Header::MAX_SIZE];
        let head = &mut head[..file_size.min(Header::MAX_SIZE as u64) as usize];
        stream.read_exact_at(0, head)?;
        let header = Header::parse(head)?;

        let entry_size = header.entry_size as usize;
        let fit =
            file_size.saturating_sub(header.entry_offset.into()) / u64::from(header.entry_size);
        let count = u64::from(header.count).min(fit) as u32;
        if count < header.count {
            cb.error(ErrorFlags::UnexpectedEnd);
        }
        cb.set_total(Some(count.into()), Some(file_size))?;

        let mut items: Vec<Item> = Vec::new();
        items.try_reserve_exact(count as usize)?;
        let chunk_bytes = (count.min(CHUNK) as usize)
            .checked_mul(entry_size)
            .ok_or(Error::OUT_OF_MEMORY)?;
        let mut buf = Vec::new();
        buf.try_reserve_exact(chunk_bytes)?;
        buf.resize(chunk_bytes, 0);

        let mut pos = u64::from(header.entry_offset);
        let mut end = pos + u64::from(count) * u64::from(header.entry_size);
        let (mut duplicate, mut unsorted) = (false, false);
        while items.len() < count as usize {
            let n = (count as usize - items.len()).min(CHUNK as usize);
            let toc = &mut buf[..n * entry_size];
            stream.read_exact_at(pos, toc)?;
            pos += toc.len() as u64;
            for raw in toc.chunks_exact(entry_size) {
                let entry = header.parse_entry(raw)?;
                let data_end = u64::from(entry.offset) + u64::from(entry.size);
                if data_end > file_size {
                    cb.error(ErrorFlags::UnexpectedEnd);
                }
                end = end.max(data_end);
                if let Some(prev) = items.last() {
                    duplicate |= prev.entry.hash == entry.hash;
                    unsorted |= prev.entry.hash > entry.hash;
                }
                items.push(Item { entry });
            }
            cb.set_completed(Some(items.len() as u64), Some(pos))?;
        }
        let odd: Vec<&str> = [
            (duplicate, "duplicate entry hashes"),
            (unsorted, "entries not sorted by hash"),
        ]
        .into_iter()
        .filter_map(|(found, message)| found.then_some(message))
        .collect();
        if !odd.is_empty() {
            cb.warning(ErrorFlags::HeadersError);
            cb.warning_message(odd.join("; "));
        }
        let names = Names::new(items.iter().map(|i| i.entry.hash))?;
        Ok(WadArchive::new(
            stream,
            header,
            items,
            names,
            end.min(file_size),
        ))
    }
}
