//! [`ExtractCallback`]: what 7-Zip passes to [`Archive::extract`](crate::Archive::extract).
//!
//! Calling into it can call back into the handler on the same thread: during
//! [`ExtractCallback::item`], 7-Zip reads the item's properties from the handler to name
//! the output file. Don't hold a `RefCell` borrow or a lock across these calls.

use crate::error::{Result, check};
use crate::open::password;
use crate::out_stream::OutStream;
use crate::progress::Progress;
use crate::prop::Prop;
use core::sync::atomic::AtomicBool;
use lol7z_sys::i_archive::{
    AskMode, EventIndexType, IArchiveExtractCallback, IArchiveExtractCallbackMessage2,
    IArchiveGetDiskProperty, IArchiveRequestMemoryUseCallback, OperationResult,
    RequestMemoryAnswerFlags, RequestMemoryUseFlags,
};
use lol7z_sys::i_stream::ISequentialOutStream;
use lol7z_sys::propid::PropId;
use std::io::{self, Write};
use windows_core::{Interface, PCWSTR};

/// Which items [`Archive::extract`](crate::Archive::extract) should process, in order.
/// Every index is below [`Archive::item_count`](crate::Archive::item_count).
#[derive(Clone, Copy, Debug)]
pub enum Indices<'a> {
    All(u32),
    List(&'a [u32]),
}

impl<'a> Indices<'a> {
    pub fn len(&self) -> usize {
        match *self {
            Indices::All(n) => n as usize,
            Indices::List(l) => l.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + 'a {
        let (all, list) = match *self {
            Indices::All(n) => (0..n, &[][..]),
            Indices::List(l) => (0..0, l),
        };
        all.chain(list.iter().copied())
    }
}

/// Passed to [`Archive::extract`](crate::Archive::extract).
pub struct ExtractCallback<'a> {
    cb: &'a IArchiveExtractCallback,
    test: bool,
    busy: AtomicBool,
}

impl<'a> ExtractCallback<'a> {
    pub(crate) fn new(cb: &'a IArchiveExtractCallback, test: bool) -> Self {
        Self {
            cb,
            test,
            busy: AtomicBool::new(false),
        }
    }

    /// Test mode: decode and check items, but there is nowhere to write them.
    /// [`ItemOut`] discards what you write.
    pub fn is_test(&self) -> bool {
        self.test
    }

    pub fn progress(&self) -> Progress<'_> {
        Progress::new(self.cb, &self.busy)
    }

    pub fn set_total(&self, total: u64) -> Result<()> {
        self.progress().set_total(total)
    }

    pub fn set_completed(&self, done: u64) -> Result<()> {
        self.progress().set_completed(done)
    }

    pub fn password(&self) -> Result<Option<String>> {
        password(self.cb)
    }

    /// Starts item `index`: write its data to the returned [`ItemOut`], then
    /// [`finish`](ItemOut::finish) it. Check [`ItemOut::is_wanted`] first.
    pub fn item(&self, index: u32) -> Result<ItemOut<'_>> {
        self.item_as(index, self.mode())
    }

    /// The mode for items that were asked for.
    fn mode(&self) -> AskMode {
        if self.test {
            AskMode::Test
        } else {
            AskMode::Extract
        }
    }

    /// [`item`](Self::item) with an explicit mode. Solid formats pass `AskMode::Skip`
    /// for items they have to decode to reach a requested one.
    pub fn item_as(&self, index: u32, mode: AskMode) -> Result<ItemOut<'_>> {
        let mut out: Option<ISequentialOutStream> = None;
        check(unsafe { self.cb.GetStream(index, &mut out, mode) })?;
        // Extract mode without a stream: 7-Zip skipped the item (excluded, or "skip" at
        // an overwrite prompt), and expects neither PrepareOperation nor a result.
        let wanted = out.is_some() || mode != AskMode::Extract;
        if wanted {
            check(unsafe { self.cb.PrepareOperation(mode) })?;
        }
        Ok(ItemOut {
            cb: self.cb,
            out: out.map(OutStream::from_com),
            open: wanted,
        })
    }

    /// An item without data, such as a directory. 7-Zip creates it while handling the
    /// item, so this always reports a result.
    pub fn empty_item(&self, index: u32) -> Result<()> {
        let mode = self.mode();
        let mut out: Option<ISequentialOutStream> = None;
        check(unsafe { self.cb.GetStream(index, &mut out, mode) })?;
        drop(out);
        check(unsafe { self.cb.PrepareOperation(mode) })?;
        check(unsafe { self.cb.SetOperationResult(OperationResult::OK) })
    }

    /// Reports an error for an item outside the item's own [`ItemOut`], such as a CRC
    /// failure noticed after more data was decoded. Needs 7-Zip 23 or later; does
    /// nothing on older hosts.
    pub fn report(&self, index: u32, result: OperationResult) -> Result<()> {
        let Ok(m) = self.cb.cast::<IArchiveExtractCallbackMessage2>() else {
            return Ok(());
        };
        check(unsafe { m.ReportExtractResult(EventIndexType::InArcIndex, index, result) })
    }

    /// Asks before a large allocation whose size comes from the archive, such as a
    /// decoder window: `required` bytes, against the format's default limit `limit`.
    /// 7-Zip checks the user's `-smemx` setting and may ask the user. `index` is the item,
    /// or `None` for the whole archive.
    ///
    /// `false`: don't allocate. Stop with [`Error::OUT_OF_MEMORY`](crate::Error::OUT_OF_MEMORY)
    /// as 7-Zip's own formats do. Hosts without the callback allow anything within
    /// `limit`.
    pub fn request_memory(&self, index: Option<u32>, required: u64, limit: u64) -> Result<bool> {
        let over = required > limit;
        let Ok(cb) = self.cb.cast::<IArchiveRequestMemoryUseCallback>() else {
            return Ok(!over);
        };
        let flags = if over {
            RequestMemoryUseFlags::DefaultLimit_Exceeded
        } else {
            RequestMemoryUseFlags::empty()
        };
        let (kind, index) = match index {
            Some(i) => (EventIndexType::InArcIndex, i),
            None => (EventIndexType::NoIndex, 0),
        };
        // The default answer is "allow", as in 7-Zip's RAR5 handler: the host decides.
        let mut allowed = limit;
        let mut answer = RequestMemoryAnswerFlags::Allow;
        check(unsafe {
            cb.RequestMemoryUse(
                flags,
                kind,
                index,
                PCWSTR::null(),
                required,
                &mut allowed,
                &mut answer,
            )
        })?;
        let refused = RequestMemoryAnswerFlags::Stop | RequestMemoryAnswerFlags::SkipArc;
        Ok(answer.contains(RequestMemoryAnswerFlags::Allow) && !answer.intersects(refused))
    }

    /// A property of the file on disk that item `index` is being extracted over, such
    /// as its `Size`. Used by formats that check files rather than write them (hash
    /// lists). `None` when the host doesn't say.
    pub fn disk_prop(&self, index: u32, id: PropId) -> Result<Option<Prop<'static>>> {
        let Ok(cb) = self.cb.cast::<IArchiveGetDiskProperty>() else {
            return Ok(None);
        };
        Prop::read_from(|v| unsafe { cb.GetDiskProperty(index, id, v) })
    }
}

/// One item being extracted. See [`ExtractCallback::item`].
///
/// Writing is the same in every mode: in test mode, and for items 7-Zip asked to skip,
/// the data is discarded. Dropping it without [`finish`](Self::finish) reports
/// `DataError`.
pub struct ItemOut<'a> {
    cb: &'a IArchiveExtractCallback,
    out: Option<OutStream>,
    open: bool,
}

impl ItemOut<'_> {
    /// `false` when 7-Zip doesn't want this item after all. Write nothing and drop it.
    pub fn is_wanted(&self) -> bool {
        self.open
    }

    /// Closes the output and reports the item's result. Data errors go here, and
    /// extraction goes on with the next item.
    pub fn finish(mut self, result: OperationResult) -> Result<()> {
        self.close(result)
    }

    /// For errors that stop the whole operation: 7-Zip gets the HRESULT instead of an item result.
    pub fn abandon(mut self) {
        self.open = false;
        self.out = None;
    }

    fn close(&mut self, result: OperationResult) -> Result<()> {
        if !core::mem::take(&mut self.open) {
            return Ok(());
        }
        // 7-Zip closes the file and sets its attributes in SetOperationResult, so the
        // stream has to be released first.
        self.out = None;
        check(unsafe { self.cb.SetOperationResult(result) })
    }
}

impl Write for ItemOut<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.out {
            Some(out) => out.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for ItemOut<'_> {
    fn drop(&mut self) {
        let _ = self.close(OperationResult::DataError);
    }
}
