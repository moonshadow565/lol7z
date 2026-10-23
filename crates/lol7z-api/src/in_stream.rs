//! 7-Zip's input streams as `std::io` readers.
//!
//! A 7-Zip stream is a reference-counted COM object with its own position, so reading
//! and seeking work through `&self`, as with `&File`. Cloning shares the position.

use crate::com_stream::{self, ComRead, ComSeek};
use lol7z_sys::i_stream::{IInStream, ISequentialInStream, IStreamGetSize};
use std::io::{self, Read, Seek, SeekFrom};
use windows_core::Interface;

/// A seekable input stream: the archive file 7-Zip hands to [`Format::open`](crate::Format::open).
///
/// Keep a clone in your archive to read item data during extraction.
#[derive(Clone)]
pub struct InStream(IInStream);

impl InStream {
    /// Serves `inner` as a 7-Zip stream, e.g. a `Cursor` in tests. 7-Zip may use it from
    /// another thread, one at a time; see [`ItemStream`](crate::ItemStream).
    pub fn new<R: Read + Seek + 'static>(inner: R) -> Self {
        Self(com_stream::reader(inner))
    }

    pub fn from_com(stream: IInStream) -> Self {
        Self(stream)
    }

    pub fn as_com(&self) -> &IInStream {
        &self.0
    }

    /// Total length. Leaves the position where it was.
    pub fn size(&self) -> io::Result<u64> {
        if let Ok(s) = self.0.cast::<IStreamGetSize>() {
            let mut n = 0;
            if unsafe { s.GetSize(&mut n) }.is_ok() {
                return Ok(n);
            }
        }
        let mut s = self;
        let pos = s.stream_position()?;
        let end = s.seek(SeekFrom::End(0))?;
        s.seek(SeekFrom::Start(pos))?;
        Ok(end)
    }

    pub fn read_exact_at(&self, pos: u64, buf: &mut [u8]) -> io::Result<()> {
        let mut s = self;
        s.seek(SeekFrom::Start(pos))?;
        s.read_exact(buf)
    }

    /// `len` bytes from `offset`, as a stream with its own position.
    pub fn slice(&self, offset: u64, len: u64) -> Slice {
        Slice {
            stream: self.clone(),
            start: offset,
            len,
            pos: 0,
        }
    }
}

impl Read for &InStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read_io(buf)
    }
}

impl Seek for &InStream {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.seek_io(pos)
    }
}

impl Read for InStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self).read(buf)
    }
}

impl Seek for InStream {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        (&*self).seek(pos)
    }
}

/// A window into an [`InStream`] with its own position. See [`InStream::slice`].
///
/// Seeks the shared stream before every read, so slices of one stream can be read
/// in any order.
#[derive(Clone)]
pub struct Slice {
    stream: InStream,
    start: u64,
    len: u64,
    pos: u64,
}

impl Slice {
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Read for Slice {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.len.saturating_sub(self.pos);
        let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        if want == 0 {
            return Ok(0);
        }
        let at = self
            .start
            .checked_add(self.pos)
            .ok_or(io::ErrorKind::InvalidInput)?;
        let mut s = &self.stream;
        s.seek(SeekFrom::Start(at))?;
        let n = s.read(&mut buf[..want])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Slice {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::Current(n) => self.pos.checked_add_signed(n),
            SeekFrom::End(n) => self.len.checked_add_signed(n),
        };
        self.pos = new.ok_or(io::ErrorKind::InvalidInput)?;
        Ok(self.pos)
    }
}

/// A forward-only input stream: new item data during [`Update`](crate::Update).
pub struct SeqInStream(ISequentialInStream);

impl SeqInStream {
    pub fn from_com(stream: ISequentialInStream) -> Self {
        Self(stream)
    }
}

impl Read for SeqInStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read_io(buf)
    }
}
