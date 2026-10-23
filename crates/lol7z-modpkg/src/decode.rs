//! Stored chunk data out, streamed and checked.

use crate::toc::{Compression, Record};
use lol7z_api::{Error, ErrorFlags, InStream, OperationResult, Result};
use lol7z_wad::encode::Counting;
use std::io::{self, Read, Write};

/// Writes `r`'s content bytes. The result compares what came out against the record's
/// uncompressed size and checksum.
pub(crate) fn extract(
    stream: &InStream,
    r: &Record,
    out: &mut dyn Write,
) -> Result<OperationResult> {
    let mut src = stream.slice(r.data_offset, r.compressed_size);
    let mut out = Counting::new(out);
    let n = match r.compression {
        Compression::Raw => pump(&mut src, &mut out)?,
        Compression::Zstd => {
            let dec = zstd::Decoder::new(src).map_err(decode_err)?;
            pump(&mut dec.single_frame(), &mut out)?
        }
        _ => return Err(Error::Damaged(ErrorFlags::UnsupportedMethod)),
    };
    let (_, checksum) = out.finish();
    Ok(if n < r.uncompressed_size {
        OperationResult::UnexpectedEnd
    } else if n > r.uncompressed_size {
        OperationResult::DataError
    } else if checksum != r.uncompressed_checksum {
        OperationResult::CRCError
    } else {
        OperationResult::OK
    })
}

/// Copies `r` to `out`. Read errors are the data's fault unless they carry an HRESULT,
/// which is 7-Zip's stream failing.
fn pump(r: &mut impl Read, out: &mut dyn Write) -> Result<u64> {
    let mut buf = vec![0u8; 1 << 16];
    let mut n = 0u64;
    loop {
        let k = match r.read(&mut buf) {
            Ok(0) => return Ok(n),
            Ok(k) => k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(decode_err(e)),
        };
        out.write_all(&buf[..k])?;
        n += k as u64;
    }
}

fn decode_err(e: io::Error) -> Error {
    if e.raw_os_error().is_some() {
        return e.into();
    }
    match e.kind() {
        io::ErrorKind::UnexpectedEof => Error::UNEXPECTED_END,
        _ => Error::DATA_ERROR,
    }
}
