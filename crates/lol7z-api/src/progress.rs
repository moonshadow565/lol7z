//! [`Progress`]: progress reports during extraction and updates.

use crate::error::{Result, check};
use core::sync::atomic::{AtomicBool, Ordering};
use lol7z_sys::i_progress::IProgress;
use std::io::{self, Read, Write};

/// Reports progress. 7-Zip allows this from other threads while the handler's main
/// thread is using the extract or update callback (IArchive.h:183), so a `Progress` can
/// be sent to worker threads.
#[derive(Clone, Copy)]
pub struct Progress<'a> {
    cb: &'a IProgress,
    busy: &'a AtomicBool,
}

// SAFETY: 7-Zip accepts IProgress calls from any thread, concurrently with the other
// callback methods. `busy` keeps two progress calls from overlapping, and `Progress`
// never AddRefs or Releases the object (7-Zip's reference counts aren't atomic).
unsafe impl Send for Progress<'_> {}
unsafe impl Sync for Progress<'_> {}

impl<'a> Progress<'a> {
    pub(crate) fn new(cb: &'a IProgress, busy: &'a AtomicBool) -> Self {
        Self { cb, busy }
    }

    fn call(&self, f: impl FnOnce(&IProgress) -> windows_core::HRESULT) -> Result<()> {
        while self.busy.swap(true, Ordering::Acquire) {
            std::thread::yield_now();
        }
        let hr = f(self.cb);
        self.busy.store(false, Ordering::Release);
        check(hr)
    }

    /// Total work, in the units later passed to [`set_completed`](Self::set_completed):
    /// unpacked bytes, for most formats.
    pub fn set_total(&self, total: u64) -> Result<()> {
        self.call(|cb| unsafe { cb.SetTotal(total) })
    }

    /// Fails with [`Error::ABORT`](crate::Error::ABORT) when the user has cancelled.
    pub fn set_completed(&self, done: u64) -> Result<()> {
        self.call(|cb| unsafe { cb.SetCompleted(Some(&done)) })
    }

    /// Reports `base` plus the bytes written every 4 MiB, so cancel works mid-item.
    pub fn writer<W: Write>(self, w: W, base: u64) -> ProgressWriter<'a, W> {
        ProgressWriter {
            inner: w,
            counter: Counter::new(self, base),
        }
    }

    /// [`writer`](Self::writer) for reading: counts the bytes read.
    pub fn reader<R: Read>(self, r: R, base: u64) -> ProgressReader<'a, R> {
        ProgressReader {
            inner: r,
            counter: Counter::new(self, base),
        }
    }
}

/// 7-Zip's `CCopyCoder` cadence.
const STEP: u64 = 1 << 22;

struct Counter<'a> {
    progress: Progress<'a>,
    base: u64,
    n: u64,
    next: u64,
}

impl<'a> Counter<'a> {
    fn new(progress: Progress<'a>, base: u64) -> Self {
        Self {
            progress,
            base,
            n: 0,
            next: STEP,
        }
    }

    fn add(&mut self, len: usize) -> io::Result<()> {
        self.n = self.n.saturating_add(len as u64);
        if self.n < self.next {
            return Ok(());
        }
        self.next = (self.n - self.n % STEP).saturating_add(STEP);
        Ok(self
            .progress
            .set_completed(self.base.saturating_add(self.n))?)
    }
}

/// See [`Progress::writer`].
pub struct ProgressWriter<'a, W> {
    inner: W,
    counter: Counter<'a>,
}

impl<W: Write> Write for ProgressWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.counter.add(n)?;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// See [`Progress::reader`].
pub struct ProgressReader<'a, R> {
    inner: R,
    counter: Counter<'a>,
}

impl<R: Read> Read for ProgressReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.counter.add(n)?;
        Ok(n)
    }
}
