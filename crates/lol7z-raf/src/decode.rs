//! Entry data out of the data file, streamed.

use crate::archive::Method;
use crate::toc::Entry;
use flate2::read::DeflateDecoder;
use lol7z_api::{Error, InStream, OperationResult, Result};
use std::io::{self, Read, Write};

/// A zlib stream's header and Adler-32 trailer, around the deflate data.
pub const ZLIB_HEADER: u64 = 2;
pub const ZLIB_TRAILER: u64 = 4;

/// Whether these two bytes start a zlib stream: deflate, a window of 32 KiB or less, no
/// preset dictionary, and the check bits right. What the index leaves unsaid.
pub fn is_zlib(head: [u8; 2]) -> bool {
    let [cmf, flg] = head;
    cmf & 0x0F == 8
        && cmf >> 4 <= 7
        && flg & 0x20 == 0
        && (u16::from(cmf) << 8 | u16::from(flg)) % 31 == 0
}

/// Writes `e`'s data. For a zlib entry the result compares what came out against the
/// stream's Adler-32.
pub(crate) fn extract(
    data: &InStream,
    e: &Entry,
    method: Method,
    out: &mut dyn Write,
) -> Result<OperationResult> {
    let length = u64::from(e.length);
    let offset = u64::from(e.offset);
    match method {
        Method::Store => {
            let n = pump(&mut data.slice(offset, length), out)?;
            Ok(if n < length {
                OperationResult::UnexpectedEnd
            } else {
                OperationResult::OK
            })
        }
        Method::Zlib => {
            let packed = length - ZLIB_HEADER - ZLIB_TRAILER;
            let mut out = Adler::new(out);
            let src = data.slice(offset + ZLIB_HEADER, packed);
            let mut dec = DeflateDecoder::new(src);
            pump(&mut dec, &mut out)?;
            // The trailer follows the deflate stream, wherever that stopped.
            let used = dec.total_in();
            let adler = out.finish();
            let mut trailer = [0u8; 4];
            data.read_exact_at(offset + ZLIB_HEADER + used, &mut trailer)?;
            Ok(if adler != u32::from_be_bytes(trailer) {
                OperationResult::CRCError
            } else {
                OperationResult::OK
            })
        }
        Method::Unknown => Ok(OperationResult::Unavailable),
    }
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

/// A writer that keeps the Adler-32 of what passes through.
struct Adler<'a> {
    w: &'a mut dyn Write,
    a: u32,
    b: u32,
}

/// Largest prime below 2^16.
const MOD: u32 = 65521;
/// Bytes between reductions that keep `b` in a u32.
const RUN: usize = 5552;

impl<'a> Adler<'a> {
    fn new(w: &'a mut dyn Write) -> Self {
        Self { w, a: 1, b: 0 }
    }

    fn finish(self) -> u32 {
        self.b << 16 | self.a
    }
}

impl Write for Adler<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.w.write(buf)?;
        for run in buf[..n].chunks(RUN) {
            for &x in run {
                self.a += u32::from(x);
                self.b += self.a;
            }
            self.a %= MOD;
            self.b %= MOD;
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.w.flush()
    }
}
