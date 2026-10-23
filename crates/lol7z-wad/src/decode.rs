//! Stored entry data out, streamed.

use crate::toc::{Entry, Kind};
use flate2::read::GzDecoder;
use lol7z_api::{Error, ErrorFlags, InStream, OperationResult, Result};
use std::io::{self, BufRead, Read, Write};

pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// An entry's stored bytes, read from the archive.
pub struct Stored<'a> {
    stream: &'a InStream,
    entry: &'a Entry,
}

impl<'a> Stored<'a> {
    pub fn new(stream: &'a InStream, entry: &'a Entry) -> Self {
        Self { stream, entry }
    }

    /// Writes the data decoded. `Link` data (length-prefixed target path) goes out as
    /// stored. The result compares what came out against the entry's decompressed size.
    pub fn decode(&self, out: &mut dyn Write) -> Result<OperationResult> {
        let e = self.entry;
        let mut src = self.stream.slice(e.offset.into(), e.size.into());
        let n = match e.kind {
            Kind::Raw | Kind::Link => pump(&mut src, out)?,
            Kind::Gzip => pump(&mut GzDecoder::new(src), out)?,
            // One or more frames, back to back.
            Kind::Zstd => pump(&mut zstd::Decoder::new(src).map_err(decode_err)?, out)?,
            Kind::ZstdMulti => decode_multi(src, out)?,
            _ => return Err(Error::Damaged(ErrorFlags::UnsupportedMethod)),
        };
        let want = u64::from(e.size_decompressed);
        Ok(if n == want {
            OperationResult::OK
        } else if n < want {
            OperationResult::UnexpectedEnd
        } else {
            OperationResult::DataError
        })
    }

    /// The target of a `Link` entry.
    pub fn link_target(&self) -> Result<String> {
        let e = self.entry;
        if e.size < 4 {
            return Err(Error::DATA_ERROR);
        }
        let mut len = [0; 4];
        self.stream.read_exact_at(e.offset.into(), &mut len)?;
        let len = u32::from_le_bytes(len);
        if len > e.size - 4 {
            return Err(Error::DATA_ERROR);
        }
        let mut target = Vec::new();
        target.try_reserve_exact(len as usize)?;
        target.resize(len as usize, 0);
        self.stream
            .read_exact_at(u64::from(e.offset) + 4, &mut target)?;
        Ok(String::from_utf8_lossy(&target).into_owned())
    }
}

/// Subchunks laid end to end, each one zstd frame or stored raw, in any order. The
/// archive's subchunk table says which; without it, a raw run is whatever sits between
/// frames.
fn decode_multi(src: impl Read, out: &mut dyn Write) -> Result<u64> {
    let mut p = Peek::new(src);
    let mut n = 0u64;
    loop {
        let (buf, eof) = p.peek().map_err(decode_err)?;
        if buf.is_empty() {
            return Ok(n);
        }
        if buf.starts_with(&ZSTD_MAGIC) {
            let dec = zstd::Decoder::with_buffer(&mut p).map_err(decode_err)?;
            n += pump(&mut dec.single_frame(), out)?;
        } else {
            // Keep the last 3 bytes back: a magic may straddle the refill.
            let whole = if eof { buf.len() } else { buf.len() - 3 };
            let gap = find(&buf[1..], &ZSTD_MAGIC).map_or(whole, |i| i + 1);
            out.write_all(&buf[..gap])?;
            n += gap as u64;
            p.consume(gap);
        }
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

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// A `BufRead` whose buffer holds at least 4 bytes until the source ends, so a frame
/// magic can always be checked at the front.
struct Peek<R> {
    r: R,
    buf: Box<[u8]>,
    pos: usize,
    end: usize,
    eof: bool,
}

impl<R: Read> Peek<R> {
    fn new(r: R) -> Self {
        Self {
            r,
            buf: vec![0; 1 << 16].into_boxed_slice(),
            pos: 0,
            end: 0,
            eof: false,
        }
    }

    /// The buffer and whether the source has ended.
    fn peek(&mut self) -> io::Result<(&[u8], bool)> {
        self.refill()?;
        Ok((&self.buf[self.pos..self.end], self.eof))
    }

    fn refill(&mut self) -> io::Result<()> {
        if self.end - self.pos >= 4 || self.eof {
            return Ok(());
        }
        self.buf.copy_within(self.pos..self.end, 0);
        self.end -= self.pos;
        self.pos = 0;
        while self.end < self.buf.len() {
            match self.r.read(&mut self.buf[self.end..]) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(n) => self.end += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

impl<R: Read> Read for Peek<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let buf = self.fill_buf()?;
        let n = buf.len().min(out.len());
        out[..n].copy_from_slice(&buf[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl<R: Read> BufRead for Peek<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Ok(self.peek()?.0)
    }

    fn consume(&mut self, n: usize) {
        self.pos += n;
    }
}
