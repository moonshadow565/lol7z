//! Writing archives: the [`Update`] trait and the [`UpdateCallback`] it gets.

use crate::error::{Result, check, found};
use crate::format::Format;
use crate::in_stream::SeqInStream;
use crate::options::Options;
use crate::out_stream::OutStream;
use crate::progress::Progress;
use crate::prop::Prop;
use core::cell::OnceCell;
use core::sync::atomic::AtomicBool;
use lol7z_sys::i_archive::{
    EventIndexType, FileTimeType, IArchiveUpdateCallback, IArchiveUpdateCallback2,
    IArchiveUpdateCallbackFile, UpdateNotifyOp,
};
use lol7z_sys::i_password::ICryptoGetTextPassword2;
use lol7z_sys::i_stream::{ISequentialInStream, ISequentialOutStream};
use lol7z_sys::propid::PropId;
use windows_core::{BSTR, Interface};

/// A format that can create and modify archives.
pub trait Update: Format {
    /// The timestamp precision stored; 7-Zip rounds times to it when comparing files.
    const FILE_TIME_TYPE: FileTimeType = FileTimeType::Windows;

    /// Writes a complete new archive to `out`, with the items `cb` lists.
    ///
    /// `existing` is the archive open on this handler, if any: when adding to or
    /// deleting from an archive, 7-Zip opens it first and items with `old_index` refer
    /// to it. `out` is a different file, never the one `existing` reads.
    fn update(
        existing: Option<&Self::Archive>,
        out: OutStream,
        cb: &UpdateCallback<'_>,
        options: &Options,
    ) -> Result<()>;
}

/// What 7-Zip wants at one position of the new archive. See [`UpdateCallback::item`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdateItem {
    pub index: u32,
    /// Take the data from [`UpdateCallback::stream`]; otherwise copy it from `old_index`.
    pub new_data: bool,
    /// Take the properties from [`UpdateCallback`]; otherwise copy them from `old_index`.
    pub new_props: bool,
    /// The item in the archive being updated, if this item comes from there.
    pub old_index: Option<u32>,
}

/// Passed to [`Update::update`](crate::Update::update).
pub struct UpdateCallback<'a> {
    cb: &'a IArchiveUpdateCallback,
    len: u32,
    busy: AtomicBool,
    /// Looked up on first use.
    file: OnceCell<Option<IArchiveUpdateCallbackFile>>,
}

impl<'a> UpdateCallback<'a> {
    pub(crate) fn new(cb: &'a IArchiveUpdateCallback, len: u32) -> Self {
        Self {
            cb,
            len,
            busy: AtomicBool::new(false),
            file: OnceCell::new(),
        }
    }

    /// Number of items in the new archive.
    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
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

    pub fn item(&self, index: u32) -> Result<UpdateItem> {
        let (mut new_data, mut new_props, mut old) = (0, 0, 0);
        check(unsafe {
            self.cb
                .GetUpdateItemInfo(index, &mut new_data, &mut new_props, &mut old)
        })?;
        Ok(UpdateItem {
            index,
            new_data: new_data != 0,
            new_props: new_props != 0,
            old_index: (old != u32::MAX).then_some(old),
        })
    }

    pub fn items(&self) -> impl Iterator<Item = Result<UpdateItem>> + '_ {
        (0..self.len).map(|i| self.item(i))
    }

    /// A property of a new item. `None` when 7-Zip has no value for it.
    pub fn prop(&self, index: u32, id: PropId) -> Result<Option<Prop<'static>>> {
        Prop::read_from(|v| unsafe { self.cb.GetProperty(index, id, v) })
    }

    /// Path inside the archive, with `/` or `\` separators as the host gives it.
    pub fn path(&self, index: u32) -> Result<Option<String>> {
        Ok(self.prop(index, PropId::Path)?.and_then(Prop::into_string))
    }

    pub fn is_dir(&self, index: u32) -> Result<bool> {
        self.flag(index, PropId::IsDir)
    }

    /// A delete marker, used by formats that store deletions (7z's anti-items).
    pub fn is_anti(&self, index: u32) -> Result<bool> {
        self.flag(index, PropId::IsAnti)
    }

    /// A yes/no property; no value counts as no.
    fn flag(&self, index: u32, id: PropId) -> Result<bool> {
        Ok(self.prop(index, id)?.and_then(|p| p.as_bool()) == Some(true))
    }

    /// Expected data size. The stream can still turn out shorter or longer.
    pub fn size(&self, index: u32) -> Result<Option<u64>> {
        Ok(self.prop(index, PropId::Size)?.and_then(|p| p.as_u64()))
    }

    /// The new item's data. `None` when the file couldn't be opened; 7-Zip has
    /// already told the user, so leave the item out.
    pub fn stream(&self, index: u32) -> Result<Option<SeqInStream>> {
        let mut s: Option<ISequentialInStream> = None;
        let hr = unsafe { self.cb.GetStream(index, &mut s) };
        Ok(found(hr, s)?.map(SeqInStream::from_com))
    }

    /// [`stream`](Self::stream), saying what the data is for. Plain `stream` already
    /// shows as "add" or "update" depending on whether the item existed; use this for
    /// `Analyze` (reading ahead to sort or group files) and the like.
    pub fn stream_as(&self, index: u32, op: UpdateNotifyOp) -> Result<Option<SeqInStream>> {
        let Some(file) = self.file() else {
            return self.stream(index);
        };
        let mut s: Option<ISequentialInStream> = None;
        let hr = unsafe { file.GetStream2(index, &mut s, op) };
        Ok(found(hr, s)?.map(SeqInStream::from_com))
    }

    /// Tells 7-Zip what's happening, for its log and progress display. `index` is
    /// interpreted by `kind`: an old item for `InArcIndex`, a new one for `OutArcIndex`.
    /// Does nothing on hosts without `IArchiveUpdateCallbackFile`.
    pub fn report(&self, kind: EventIndexType, index: u32, op: UpdateNotifyOp) -> Result<()> {
        let Some(file) = self.file() else {
            return Ok(());
        };
        check(unsafe { file.ReportOperation(kind, index, op) })
    }

    /// Reports an item copied unchanged from the old archive; 7-Zip lists it as `=`.
    pub fn copied(&self, old_index: u32) -> Result<()> {
        self.report(
            EventIndexType::InArcIndex,
            old_index,
            UpdateNotifyOp::Replicate,
        )
    }

    fn file(&self) -> Option<&IArchiveUpdateCallbackFile> {
        self.file.get_or_init(|| self.cb.cast().ok()).as_ref()
    }

    /// The size the user asked for volume `index` (`-v`), for formats that split
    /// themselves. `None` when the host doesn't say. Formats that don't split can ignore
    /// it: 7-Zip splits their output itself.
    pub fn volume_size(&self, index: u32) -> Result<Option<u64>> {
        let Ok(cb) = self.cb.cast::<IArchiveUpdateCallback2>() else {
            return Ok(None);
        };
        let mut size = 0;
        check(unsafe { cb.GetVolumeSize(index, &mut size) })?;
        Ok(Some(size))
    }

    /// The output for volume `index`, for formats that split themselves.
    pub fn volume_stream(&self, index: u32) -> Result<Option<OutStream>> {
        let Ok(cb) = self.cb.cast::<IArchiveUpdateCallback2>() else {
            return Ok(None);
        };
        let mut s: Option<ISequentialOutStream> = None;
        check(unsafe { cb.GetVolumeStream(index, &mut s) })?;
        Ok(s.map(OutStream::from_com))
    }

    /// Call after writing a new item's data, so 7-Zip can close its source file.
    pub fn item_done(&self) -> Result<()> {
        check(unsafe { self.cb.SetOperationResult(0) })
    }

    /// The password to encrypt with, if the user gave one.
    pub fn password(&self) -> Result<Option<String>> {
        let Ok(p) = self.cb.cast::<ICryptoGetTextPassword2>() else {
            return Ok(None);
        };
        let (mut defined, mut s) = (0, BSTR::new());
        check(unsafe { p.CryptoGetTextPassword2(&mut defined, &mut s) })?;
        Ok((defined != 0).then(|| s.to_string()))
    }
}
