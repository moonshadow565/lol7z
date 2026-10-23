//! `std::io` readers and writers served to 7-Zip as COM streams, and 7-Zip's COM
//! streams called as `std::io` operations.

use crate::error::{Error, check};
use crate::in_stream::Slice;
use core::cell::RefCell;
use core::ffi::c_void;
use lol7z_sys::i_stream::{
    IInStream, IInStream_Impl, IOutStream, IOutStream_Impl, ISequentialInStream,
    ISequentialInStream_Impl, ISequentialOutStream, ISequentialOutStream_Impl, STREAM_SEEK,
    STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET,
};
use std::io::{self, Read, Seek, SeekFrom, Write};
use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL};
use windows_core::{HRESULT, OutRef, implement};

/// Item data for 7-Zip to read directly, from
/// [`Archive::item_stream`](crate::Archive::item_stream). 7-Zip uses it to open archives
/// nested inside this one without extracting them first, and it needs a seekable stream
/// for most formats.
///
/// 7-Zip may read it from a different thread than the one that made it, one thread at a
/// time, so `inner` must not depend on thread-local state. It needs no `Send`: the COM
/// pointer that would carry the bound is itself `!Send`.
pub struct ItemStream(pub(crate) ISequentialInStream);

impl ItemStream {
    pub fn new<R: Read + 'static>(inner: R) -> Self {
        Self(
            ComSeqReader {
                inner: RefCell::new(inner),
            }
            .into(),
        )
    }

    pub fn seekable<R: Read + Seek + 'static>(inner: R) -> Self {
        let s = reader(inner);
        Self((*s).clone())
    }
}

impl From<Slice> for ItemStream {
    fn from(s: Slice) -> Self {
        Self::seekable(s)
    }
}

pub(crate) trait ComRead {
    fn read_io(&self, buf: &mut [u8]) -> io::Result<usize>;
}

pub(crate) trait ComWrite {
    fn write_io(&self, buf: &[u8]) -> io::Result<usize>;
}

pub(crate) trait ComSeek {
    fn seek_io(&self, pos: SeekFrom) -> io::Result<u64>;
}

impl ComRead for ISequentialInStream {
    fn read_io(&self, buf: &mut [u8]) -> io::Result<usize> {
        let size = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let mut n = 0u32;
        check(unsafe { self.Read(buf.as_mut_ptr().cast(), size, &mut n) })?;
        Ok(n as usize)
    }
}

impl ComWrite for ISequentialOutStream {
    fn write_io(&self, buf: &[u8]) -> io::Result<usize> {
        let size = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let mut n = 0u32;
        check(unsafe { self.Write(buf.as_ptr().cast(), size, &mut n) })?;
        Ok(n as usize)
    }
}

fn seek_args(pos: SeekFrom) -> io::Result<(i64, STREAM_SEEK)> {
    Ok(match pos {
        SeekFrom::Start(n) => (
            i64::try_from(n).map_err(|_| io::ErrorKind::InvalidInput)?,
            STREAM_SEEK_SET,
        ),
        SeekFrom::Current(n) => (n, STREAM_SEEK_CUR),
        SeekFrom::End(n) => (n, STREAM_SEEK_END),
    })
}

macro_rules! com_seek {
    ($t:ty) => {
        impl ComSeek for $t {
            fn seek_io(&self, pos: SeekFrom) -> io::Result<u64> {
                let (offset, origin) = seek_args(pos)?;
                let mut new = 0;
                check(unsafe { self.Seek(offset, origin, &mut new) })?;
                Ok(new)
            }
        }
    };
}

com_seek!(IInStream);
com_seek!(IOutStream);

pub(crate) fn reader<R: Read + Seek + 'static>(inner: R) -> IInStream {
    ComReader {
        inner: RefCell::new(inner),
    }
    .into()
}

pub(crate) fn writer<W: Write + 'static>(inner: W) -> ISequentialOutStream {
    ComWriter {
        inner: RefCell::new(inner),
    }
    .into()
}

pub(crate) fn seek_writer<W: Write + Seek + 'static>(inner: W) -> IOutStream {
    ComSeekWriter {
        inner: RefCell::new(inner),
    }
    .into()
}

/// # Safety
/// `data` must be valid for `size` bytes of writes, or `size` must be 0.
unsafe fn out_buf<'a>(data: *mut c_void, size: u32) -> &'a mut [u8] {
    if data.is_null() || size == 0 {
        return &mut [];
    }
    // The caller's buffer may be uninitialized; zero it so it can be a `&mut [u8]`.
    // FIXME: once `Read::read_buf` (feature `read_buf`) is stable, pass a `BorrowedBuf`
    // over `&mut [MaybeUninit<u8>]` instead and drop this memset.
    unsafe {
        core::ptr::write_bytes(data.cast::<u8>(), 0, size as usize);
        core::slice::from_raw_parts_mut(data.cast(), size as usize)
    }
}

/// # Safety
/// `data` must be valid for `size` bytes of reads, or `size` must be 0.
unsafe fn in_buf<'a>(data: *const c_void, size: u32) -> &'a [u8] {
    if data.is_null() || size == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(data.cast(), size as usize) }
    }
}

/// Fills `buf` until it is full or the reader is at its end, so the result is right
/// whether or not the caller passed `processed`.
fn fill(r: &mut impl Read, buf: &mut [u8], processed: OutRef<u32>) -> HRESULT {
    let mut done = 0;
    let res = loop {
        if done == buf.len() {
            break Ok(());
        }
        match r.read(&mut buf[done..]) {
            Ok(0) => break Ok(()),
            Ok(n) => done += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => break Err(e),
        }
    };
    let _ = processed.write(done as u32);
    res.map_or_else(|e| Error::from(e).code(), |()| HRESULT(0))
}

fn drain(w: &mut impl Write, buf: &[u8], processed: OutRef<u32>) -> HRESULT {
    let mut done = 0;
    let res = loop {
        if done == buf.len() {
            break Ok(());
        }
        match w.write(&buf[done..]) {
            Ok(0) => break Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => done += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => break Err(e),
        }
    };
    let _ = processed.write(done as u32);
    res.map_or_else(|e: io::Error| Error::from(e).code(), |()| HRESULT(0))
}

fn seek_com(
    s: &mut impl Seek,
    offset: i64,
    origin: STREAM_SEEK,
    new_position: OutRef<u64>,
) -> HRESULT {
    let pos = match origin {
        STREAM_SEEK_SET => match u64::try_from(offset) {
            Ok(n) => SeekFrom::Start(n),
            Err(_) => return E_INVALIDARG,
        },
        STREAM_SEEK_CUR => SeekFrom::Current(offset),
        STREAM_SEEK_END => SeekFrom::End(offset),
        _ => return E_INVALIDARG,
    };
    match s.seek(pos) {
        Ok(n) => {
            let _ = new_position.write(n);
            HRESULT(0)
        }
        Err(e) => Error::from(e).code(),
    }
}

#[implement(ISequentialInStream)]
struct ComSeqReader<R>
where
    R: Read + 'static,
{
    inner: RefCell<R>,
}

impl<R> ISequentialInStream_Impl for ComSeqReader_Impl<R>
where
    R: Read + 'static,
{
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: OutRef<u32>) -> HRESULT {
        fill(
            &mut *self.inner.borrow_mut(),
            unsafe { out_buf(data, size) },
            processed,
        )
    }
}

#[implement(IInStream)]
struct ComReader<R>
where
    R: Read + Seek + 'static,
{
    inner: RefCell<R>,
}

impl<R> ISequentialInStream_Impl for ComReader_Impl<R>
where
    R: Read + Seek + 'static,
{
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: OutRef<u32>) -> HRESULT {
        fill(
            &mut *self.inner.borrow_mut(),
            unsafe { out_buf(data, size) },
            processed,
        )
    }
}

impl<R> IInStream_Impl for ComReader_Impl<R>
where
    R: Read + Seek + 'static,
{
    unsafe fn Seek(&self, offset: i64, origin: STREAM_SEEK, new_position: OutRef<u64>) -> HRESULT {
        seek_com(&mut *self.inner.borrow_mut(), offset, origin, new_position)
    }
}

#[implement(ISequentialOutStream)]
struct ComWriter<W>
where
    W: Write + 'static,
{
    inner: RefCell<W>,
}

impl<W> ISequentialOutStream_Impl for ComWriter_Impl<W>
where
    W: Write + 'static,
{
    unsafe fn Write(&self, data: *const c_void, size: u32, processed: OutRef<u32>) -> HRESULT {
        drain(
            &mut *self.inner.borrow_mut(),
            unsafe { in_buf(data, size) },
            processed,
        )
    }
}

#[implement(IOutStream)]
struct ComSeekWriter<W>
where
    W: Write + Seek + 'static,
{
    inner: RefCell<W>,
}

impl<W> ISequentialOutStream_Impl for ComSeekWriter_Impl<W>
where
    W: Write + Seek + 'static,
{
    unsafe fn Write(&self, data: *const c_void, size: u32, processed: OutRef<u32>) -> HRESULT {
        drain(
            &mut *self.inner.borrow_mut(),
            unsafe { in_buf(data, size) },
            processed,
        )
    }
}

impl<W> IOutStream_Impl for ComSeekWriter_Impl<W>
where
    W: Write + Seek + 'static,
{
    unsafe fn Seek(&self, offset: i64, origin: STREAM_SEEK, new_position: OutRef<u64>) -> HRESULT {
        seek_com(&mut *self.inner.borrow_mut(), offset, origin, new_position)
    }

    /// `std::io` has no generic truncate.
    unsafe fn SetSize(&self, _new_size: u64) -> HRESULT {
        E_NOTIMPL
    }
}
