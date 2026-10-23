//! 7-Zip's output streams as `std::io` writers.

use crate::com_stream::{self, ComSeek, ComWrite};
use crate::error::check;
use lol7z_sys::i_stream::{IOutStream, ISequentialOutStream};
use std::io::{self, Seek, SeekFrom, Write};
use windows_core::Interface;

/// An output stream: an extracted item's destination, or the new archive in
/// [`Update`](crate::Update). Seekable when 7-Zip passed an `IOutStream`, which it does
/// for archive output to a file.
#[derive(Clone)]
pub struct OutStream {
    seq: ISequentialOutStream,
    seek: Option<IOutStream>,
}

impl OutStream {
    /// Serves `inner` as a 7-Zip stream. 7-Zip may use it from another thread, one at a
    /// time; see [`ItemStream`](crate::ItemStream).
    pub fn new<W: Write + 'static>(inner: W) -> Self {
        Self::from_com(com_stream::writer(inner))
    }

    pub fn seekable<W: Write + Seek + 'static>(inner: W) -> Self {
        let s = com_stream::seek_writer(inner);
        Self {
            seq: (*s).clone(),
            seek: Some(s),
        }
    }

    pub fn from_com(seq: ISequentialOutStream) -> Self {
        let seek = seq.cast::<IOutStream>().ok();
        Self { seq, seek }
    }

    pub fn as_com(&self) -> &ISequentialOutStream {
        &self.seq
    }

    pub fn is_seekable(&self) -> bool {
        self.seek.is_some()
    }

    fn seeker(&self) -> io::Result<&IOutStream> {
        self.seek
            .as_ref()
            .ok_or_else(|| io::ErrorKind::Unsupported.into())
    }

    /// Truncates or extends the stream. Needs [`is_seekable`](Self::is_seekable).
    pub fn set_len(&self, len: u64) -> io::Result<()> {
        Ok(check(unsafe { self.seeker()?.SetSize(len) })?)
    }
}

impl Write for &OutStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.seq.write_io(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for &OutStream {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.seeker()?.seek_io(pos)
    }
}

impl Write for OutStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for OutStream {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        (&*self).seek(pos)
    }
}
