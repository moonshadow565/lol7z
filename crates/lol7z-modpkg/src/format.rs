//! [`Modpkg`]: the format 7-Zip lists, and how it opens.

use crate::archive::ModpkgArchive;
use crate::decode;
use crate::meta::{MAX_INFO, Metadata};
use crate::path::{self, BASE, INFO, META_PREFIX};
use crate::toc::{
    Compression, HEADER_SIZE, Header, MAGIC, NONE, RECORD_SIZE, Record, Tables, toc_offset,
};
use lol7z_api::{
    Error, ErrorFlags, Ext, Format, GUID, InStream, OpenCallback, OperationResult, Options, Result,
};
use std::io::BufReader;

/// Records read per table read, between progress calls.
const CHUNK: u32 = 4096;

pub struct Modpkg;

impl Format for Modpkg {
    const NAME: &'static str = "modpkg";
    const CLASS_ID: GUID = GUID::from_u128(0x6c746b37_7a00_4a1b_9c3d_6d6f64706b67);
    const EXTENSIONS: &'static [Ext] = &[Ext::new("modpkg")];
    const SIGNATURES: &'static [&'static [u8]] = &[MAGIC];

    type Archive = ModpkgArchive;

    fn open(stream: InStream, cb: &OpenCallback<'_>, _options: &Options) -> Result<ModpkgArchive> {
        let file_size = stream.size()?;
        let mut head = [0u8; HEADER_SIZE];
        let head = &mut head[..file_size.min(HEADER_SIZE as u64) as usize];
        stream.read_exact_at(0, head)?;
        let header = Header::parse(head)?;

        let sig_end = HEADER_SIZE as u64 + u64::from(header.signature_size);
        if sig_end > file_size {
            return Err(Error::UNEXPECTED_END);
        }
        let mut signature = Vec::new();
        signature.try_reserve_exact(header.signature_size as usize)?;
        signature.resize(header.signature_size as usize, 0);
        stream.read_exact_at(HEADER_SIZE as u64, &mut signature)?;
        let mut r = BufReader::new(stream.slice(sig_end, file_size - sig_end));
        let tables = Tables::read(&mut r, file_size - sig_end)?;
        drop(r);

        let toc = toc_offset(sig_end + tables.size());
        let fit = file_size.saturating_sub(toc) / RECORD_SIZE as u64;
        let count = u64::from(header.chunk_count).min(fit) as u32;
        if count < header.chunk_count {
            cb.error(ErrorFlags::UnexpectedEnd);
        }
        cb.set_total(Some(count.into()), Some(file_size))?;

        let mut records: Vec<Record> = Vec::new();
        records.try_reserve_exact(count as usize)?;
        let mut buf = Vec::new();
        buf.try_reserve_exact(count.min(CHUNK) as usize * RECORD_SIZE)?;
        buf.resize(count.min(CHUNK) as usize * RECORD_SIZE, 0);
        let mut pos = toc;
        let mut end = toc + u64::from(count) * RECORD_SIZE as u64;
        while records.len() < count as usize {
            let n = (count as usize - records.len()).min(CHUNK as usize);
            let raw = &mut buf[..n * RECORD_SIZE];
            stream.read_exact_at(pos, raw)?;
            pos += raw.len() as u64;
            for raw in raw.as_chunks::<RECORD_SIZE>().0 {
                let r = Record::parse(raw)?;
                if let Some(why) = refused(&tables, &r) {
                    cb.error_message(why);
                    return Err(Error::HEADERS_ERROR);
                }
                let data_end = r.data_offset.saturating_add(r.compressed_size);
                if data_end > file_size {
                    cb.error(ErrorFlags::UnexpectedEnd);
                }
                end = end.max(data_end);
                records.push(r);
            }
            cb.set_completed(Some(records.len() as u64), Some(pos))?;
        }

        let odd = odd(&tables, &records)?;
        if !odd.is_empty() {
            cb.warning(ErrorFlags::HeadersError);
            cb.warning_message(odd.join("; "));
        }

        let metadata = metadata(&stream, &tables, &records, file_size);
        ModpkgArchive::build(
            stream,
            header,
            signature,
            tables,
            records,
            metadata,
            end.min(file_size),
        )
    }
}

/// The metadata document, when the package has one that reads.
fn metadata(stream: &InStream, t: &Tables, records: &[Record], file_size: u64) -> Option<Metadata> {
    let r = records
        .iter()
        .find(|r| r.is_meta() && t.paths[r.path_index as usize] == INFO)?;
    if r.uncompressed_size > MAX_INFO || r.data_offset.saturating_add(r.compressed_size) > file_size
    {
        return None;
    }
    let mut buf = Vec::new();
    buf.try_reserve_exact(r.uncompressed_size as usize).ok()?;
    match decode::extract(stream, r, &mut buf) {
        Ok(OperationResult::OK) => Metadata::parse(&buf),
        _ => None,
    }
}

/// Why a record makes the package unreadable.
fn refused(t: &Tables, r: &Record) -> Option<&'static str> {
    let index = |i: u32, n: usize| i == NONE || (i as usize) < n;
    if !matches!(r.compression, Compression::Raw | Compression::Zstd) {
        Some("unknown compression")
    } else if r.path_index as usize >= t.paths.len() {
        Some("path index past the path table")
    } else if !index(r.layer_index, t.layers.len()) {
        Some("layer index past the layer table")
    } else if !index(r.wad_index, t.wads.len()) {
        Some("WAD index past the WAD table")
    } else {
        None
    }
}

/// What's wrong with a package that can still be listed.
fn odd(t: &Tables, records: &[Record]) -> Result<Vec<&'static str>> {
    let base = t.layers.iter().find(|l| l.name == BASE);
    let names = t
        .paths
        .iter()
        .chain(t.layers.iter().map(|l| &l.name))
        .chain(&t.wads);
    let meta = |r: &Record| t.paths[r.path_index as usize].starts_with(META_PREFIX);
    let mut keys: Vec<(u64, u32, u32, u64, u64)> = Vec::new();
    keys.try_reserve_exact(records.len())?;
    keys.extend(records.iter().map(|r| {
        (
            r.path_hash.0,
            r.layer_index,
            r.wad_index,
            r.uncompressed_checksum,
            r.uncompressed_size,
        )
    }));
    keys.sort_unstable();
    let (mut duplicate, mut differ) = (false, false);
    for w in keys.windows(2) {
        let (a, b) = (w[0], w[1]);
        if (a.0, a.1) != (b.0, b.1) {
            continue;
        }
        duplicate |= a.2 == b.2;
        differ |= (a.3, a.4) != (b.3, b.4);
    }
    let checks = [
        (base.is_none(), "no base layer"),
        (
            base.is_some_and(|l| l.priority != 0),
            "base layer priority not 0",
        ),
        (
            t.layers.iter().any(|l| !path::is_slug(&l.name)),
            "layer name not a slug",
        ),
        (
            names.clone().any(|s| path::escapes(s)),
            "name escapes the output directory",
        ),
        (
            records.iter().any(|r| meta(r) && !r.is_meta()),
            "chunk under _meta_ with a layer or WAD",
        ),
        (
            records.iter().any(|r| r.layer_index == NONE && !meta(r)),
            "chunk without a layer outside _meta_",
        ),
        (duplicate, "duplicate chunk records"),
        (differ, "chunks of one path and layer differ"),
    ];
    Ok(checks
        .into_iter()
        .filter_map(|(found, message)| found.then_some(message))
        .collect())
}
