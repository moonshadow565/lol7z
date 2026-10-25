//! [`Wxy`]: the format 7-Zip lists, and how it opens.

use crate::archive::WxyArchive;
use crate::toc::{self, MAGIC};
use lol7z_api::{Error, ErrorFlags, Ext, Format, GUID, InStream, OpenCallback, Options, Result};
use std::io::BufReader;

pub struct Wxy;

impl Format for Wxy {
    const NAME: &'static str = "wxy";
    const CLASS_ID: GUID = GUID::from_u128(0x6c746b37_7a00_4a1b_9c3d_777879000001);
    const EXTENSIONS: &'static [Ext] = &[Ext::new("wxy")];
    const SIGNATURES: &'static [&'static [u8]] = &[MAGIC];

    type Archive = WxyArchive;

    fn open(stream: InStream, cb: &OpenCallback<'_>, _options: &Options) -> Result<WxyArchive> {
        let file_size = stream.size()?;
        let mut head = [0u8; 4];
        if file_size < 4 {
            return Err(Error::NotArchive);
        }
        stream.read_exact_at(0, &mut head)?;
        if &head != MAGIC {
            return Err(Error::NotArchive);
        }
        cb.set_total(None, Some(file_size))?;
        let mut r = BufReader::new(stream.slice(0, file_size));
        let toc = toc::read(&mut r, file_size, |n, pos| {
            cb.set_completed(Some(n), Some(pos))
        })?;
        drop(r);

        let mut end = toc.data_start;
        for e in &toc.entries {
            let data_end = e.offset.saturating_add(e.stored(toc.version));
            if data_end > file_size {
                cb.error(ErrorFlags::UnexpectedEnd);
            }
            end = end.max(data_end);
        }
        for p in &toc.previews {
            end = end.max(p.offset.saturating_add(p.size.into()));
        }
        if toc.entries.iter().any(|e| {
            !matches!(
                e.method,
                toc::Method::Store | toc::Method::Deflate | toc::Method::Zlib
            )
        }) {
            cb.warning(ErrorFlags::UnsupportedMethod);
        }
        WxyArchive::build(stream, toc, end.min(file_size))
    }
}
