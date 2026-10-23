//! [`OpenCallback`]: what 7-Zip passes to [`Format::open`](crate::Format::open).

use crate::error::{Result, check, found};
use crate::in_stream::InStream;
use crate::prop::Prop;
use core::cell::RefCell;
use lol7z_sys::i_archive::{IArchiveOpenCallback, IArchiveOpenVolumeCallback};
use lol7z_sys::i_password::ICryptoGetTextPassword;
use lol7z_sys::i_stream::IInStream;
use lol7z_sys::propid::{ErrorFlags, PropId};
use windows_core::{BSTR, HSTRING, Interface, PCWSTR};

pub(crate) fn password(cb: &impl Interface) -> Result<Option<String>> {
    let Ok(p) = cb.cast::<ICryptoGetTextPassword>() else {
        return Ok(None);
    };
    let mut s = BSTR::new();
    check(unsafe { p.CryptoGetTextPassword(&mut s) })?;
    Ok(Some(s.to_string()))
}

/// What `open` found wrong, answered as the `ErrorFlags`, `WarningFlags`, `Error` and
/// `Warning` archive properties. 7-Zip reads them after every `Open`, failed or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenReport {
    pub errors: ErrorFlags,
    pub warnings: ErrorFlags,
    pub error: Option<String>,
    pub warning: Option<String>,
}

impl Default for OpenReport {
    fn default() -> Self {
        Self {
            errors: ErrorFlags::empty(),
            warnings: ErrorFlags::empty(),
            error: None,
            warning: None,
        }
    }
}

impl OpenReport {
    pub(crate) fn prop(&self, id: PropId) -> Option<Prop<'_>> {
        let flags = |f: ErrorFlags| (!f.is_empty()).then(|| f.bits().into());
        match id {
            PropId::ErrorFlags => flags(self.errors),
            PropId::WarningFlags => flags(self.warnings),
            PropId::Error => self.error.as_deref().map(Prop::from),
            PropId::Warning => self.warning.as_deref().map(Prop::from),
            _ => None,
        }
    }
}

/// Passed to [`Format::open`](crate::Format::open).
///
/// Damage found while parsing goes here too. If the rest of the archive is still usable,
/// record it with [`error`](Self::error) and return what you read: 7-Zip lists the items
/// and shows the error. If nothing is usable, return [`Error::Damaged`](crate::Error::Damaged)
/// instead.
pub struct OpenCallback<'a> {
    cb: Option<&'a IArchiveOpenCallback>,
    report: RefCell<OpenReport>,
}

impl<'a> OpenCallback<'a> {
    pub(crate) fn new(cb: Option<&'a IArchiveOpenCallback>) -> Self {
        Self {
            cb,
            report: RefCell::default(),
        }
    }

    /// A callback that reports nowhere, for calling `open` outside 7-Zip.
    pub fn none() -> OpenCallback<'static> {
        OpenCallback::new(None)
    }

    pub fn set_total(&self, files: Option<u64>, bytes: Option<u64>) -> Result<()> {
        let Some(cb) = self.cb else { return Ok(()) };
        check(unsafe { cb.SetTotal(files.as_ref(), bytes.as_ref()) })
    }

    /// Fails with [`Error::ABORT`](crate::Error::ABORT) when the user has cancelled.
    pub fn set_completed(&self, files: Option<u64>, bytes: Option<u64>) -> Result<()> {
        let Some(cb) = self.cb else { return Ok(()) };
        check(unsafe { cb.SetCompleted(files.as_ref(), bytes.as_ref()) })
    }

    /// Records damage, such as `ErrorFlags::UnexpectedEnd` for a truncated file.
    pub fn error(&self, flags: ErrorFlags) {
        self.report.borrow_mut().errors |= flags;
    }

    /// Records something odd that doesn't stop extraction.
    pub fn warning(&self, flags: ErrorFlags) {
        self.report.borrow_mut().warnings |= flags;
    }

    /// An error message, shown along with the flags.
    pub fn error_message(&self, message: impl Into<String>) {
        self.report.borrow_mut().error = Some(message.into());
    }

    pub fn warning_message(&self, message: impl Into<String>) {
        self.report.borrow_mut().warning = Some(message.into());
    }

    /// What has been recorded so far.
    pub fn report(&self) -> OpenReport {
        self.report.borrow().clone()
    }

    pub(crate) fn into_report(self) -> OpenReport {
        self.report.into_inner()
    }

    /// Asks the user for a password. `None` when the host can't ask.
    pub fn password(&self) -> Result<Option<String>> {
        self.cb.map_or(Ok(None), password)
    }

    /// Access to the archive's file name and sibling files, for multi-volume formats.
    pub fn volumes(&self) -> Option<Volumes> {
        self.cb?
            .cast::<IArchiveOpenVolumeCallback>()
            .ok()
            .map(Volumes)
    }
}

/// See [`OpenCallback::volumes`].
pub struct Volumes(IArchiveOpenVolumeCallback);

impl Volumes {
    /// File name of the archive being opened.
    pub fn name(&self) -> Result<Option<String>> {
        let name = Prop::read_from(|v| unsafe { self.0.GetProperty(PropId::Name, v) })?;
        Ok(name.and_then(Prop::into_string))
    }

    /// Opens a file next to the archive. `None` when it doesn't exist.
    pub fn open(&self, name: &str) -> Result<Option<InStream>> {
        let name = HSTRING::from(name);
        let mut s: Option<IInStream> = None;
        let hr = unsafe { self.0.GetStream(PCWSTR(name.as_ptr()), &mut s) };
        Ok(found(hr, s)?.map(InStream::from_com))
    }
}
