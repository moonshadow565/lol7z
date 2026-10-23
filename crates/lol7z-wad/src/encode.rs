//! New entry data in, streamed.

use crate::toc::Kind;
use lol7z_api::Result;
use std::io::{self, Read, Write};
use xxhash_rust::xxh3::Xxh3;

/// Stores `src` into `out`: zstd at `level`, or raw for `None` and for audio banks (`BKHD`,
/// `r3d2` wpk), which are already compressed. Returns the kind and the data's size.
pub fn encode(
    mut src: impl Read,
    out: &mut Counting<'_>,
    level: Option<i32>,
) -> Result<(Kind, u64)> {
    let mut head = [0u8; 5];
    let got = read_full(&mut src, &mut head)?;
    let head = &head[..got];
    let bank = head.starts_with(b"BKHD")
        || head.starts_with(b"r3d2") && head.get(4).is_some_and(|b| !b.is_ascii_alphabetic());
    // Nothing to compress: a frame would only add bytes.
    let Some(level) = level.filter(|_| !bank && got > 0) else {
        out.write_all(head)?;
        return Ok((Kind::Raw, got as u64 + io::copy(&mut src, out)?));
    };
    let mut enc = zstd::Encoder::new(out, level)?;
    enc.write_all(head)?;
    let n = got as u64 + io::copy(&mut src, &mut enc)?;
    enc.finish()?;
    Ok((Kind::Zstd, n))
}

/// A writer that counts and hashes what passes through: an entry's stored size and
/// checksum.
pub struct Counting<'a> {
    w: &'a mut dyn Write,
    hash: Xxh3,
    n: u64,
}

impl<'a> Counting<'a> {
    pub fn new(w: &'a mut dyn Write) -> Self {
        Self {
            w,
            hash: Xxh3::new(),
            n: 0,
        }
    }

    /// `(size, checksum)`.
    pub fn finish(self) -> (u64, u64) {
        (self.n, self.hash.digest())
    }
}

impl Write for Counting<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = self.w.write(b)?;
        self.hash.update(&b[..n]);
        self.n += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.w.flush()
    }
}

/// Fills `buf` as far as `r` goes.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}
