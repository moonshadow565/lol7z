//! Stored data out, streamed.

use crate::toc::{Entry, Method, OINK_VERSION, Preview, ZLIB_CMF, ZLIB_FLG, header_cut};
use flate2::read::{DeflateDecoder, ZlibDecoder};
use lol7z_api::{Error, ErrorFlags, InStream, OperationResult, Result};
use std::io::{self, Read, Write};

/// Writes `e`'s data. The result compares what came out against the record's size and,
/// in `OINK` packages, its Adler-32.
pub(crate) fn extract(
    stream: &InStream,
    version: i32,
    e: &Entry,
    out: &mut dyn Write,
) -> Result<OperationResult> {
    let mut out = Adler::new(out);
    match e.method {
        Method::Store => pump(&mut stream.slice(e.offset, e.size.into()), &mut out)?,
        Method::Zlib => {
            let cut = header_cut(version) as usize;
            let head = &[ZLIB_CMF, ZLIB_FLG][..cut];
            let src = stream.slice(e.offset, e.stored(version));
            pump(&mut ZlibDecoder::new(head.chain(src)), &mut out)?
        }
        Method::Deflate => {
            let src = stream.slice(e.offset, e.packed.into());
            pump(&mut DeflateDecoder::new(src), &mut out)?
        }
        _ => return Err(Error::Damaged(ErrorFlags::UnsupportedMethod)),
    };
    let (n, adler) = out.finish();
    Ok(if n < e.size.into() {
        OperationResult::UnexpectedEnd
    } else if n > e.size.into() {
        OperationResult::DataError
    } else if version >= OINK_VERSION && adler != e.adler {
        OperationResult::CRCError
    } else {
        OperationResult::OK
    })
}

/// Writes a preview's data: one raw deflate stream.
pub(crate) fn preview(stream: &InStream, p: &Preview, out: &mut dyn Write) -> Result<u64> {
    pump(
        &mut DeflateDecoder::new(stream.slice(p.offset, p.size.into())),
        out,
    )
}

/// The first bytes of a preview, as far as they inflate, for telling its type.
pub(crate) fn preview_head(stream: &InStream, p: &Preview) -> [u8; 16] {
    let mut head = [0u8; 16];
    let mut r = DeflateDecoder::new(stream.slice(p.offset, p.size.into()));
    let mut n = 0;
    while n < head.len() {
        match r.read(&mut head[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    head
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

/// A writer that counts what passes through and keeps its Adler-32.
struct Adler<'a> {
    w: &'a mut dyn Write,
    a: u32,
    b: u32,
    n: u64,
}

/// Largest prime below 2^16.
const MOD: u32 = 65521;
/// Bytes between reductions that keep `b` in a u32.
const RUN: usize = 5552;

impl<'a> Adler<'a> {
    fn new(w: &'a mut dyn Write) -> Self {
        Self {
            w,
            a: 1,
            b: 0,
            n: 0,
        }
    }

    /// `(size, adler32)`.
    fn finish(self) -> (u64, u32) {
        (self.n, self.b << 16 | self.a)
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
        self.n += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.w.flush()
    }
}
