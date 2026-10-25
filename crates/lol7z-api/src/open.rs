//! [`OpenCallback`]: what 7-Zip passes to [`Format::open`](crate::Format::open).

use crate::error::{Error, Result, check, found, to_hresult};
use crate::in_stream::InStream;
use crate::prop::Prop;
use core::cell::RefCell;
use core::marker::PhantomData;
use lol7z_sys::i_archive::{
    IArchiveOpenCallback, IArchiveOpenCallback_Impl, IArchiveOpenVolumeCallback,
    IArchiveOpenVolumeCallback_Impl,
};
use lol7z_sys::i_password::ICryptoGetTextPassword;
use lol7z_sys::i_stream::IInStream;
use lol7z_sys::propid::{ErrorFlags, PropId};
use lol7z_sys::propvar::PropVariant;
use windows::Win32::Foundation::S_FALSE;
use windows_core::{BSTR, HRESULT, HSTRING, Interface, OutRef, PCWSTR, implement};

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
    cb: Option<IArchiveOpenCallback>,
    report: RefCell<OpenReport>,
    _borrow: PhantomData<&'a ()>,
}

impl<'a> OpenCallback<'a> {
    pub(crate) fn new(cb: Option<&'a IArchiveOpenCallback>) -> Self {
        Self {
            cb: cb.cloned(),
            report: RefCell::default(),
            _borrow: PhantomData,
        }
    }

    /// A callback that reports nowhere, for calling `open` outside 7-Zip.
    pub fn none() -> OpenCallback<'static> {
        OpenCallback::new(None)
    }

    /// A callback that knows the archive's file name and its sibling files, for calling
    /// a multi-volume format's `open` outside 7-Zip. `open` gets a sibling's file name
    /// and returns its stream, or `None` when there is no such file.
    pub fn with_volumes(
        name: &str,
        open: impl Fn(&str) -> Option<InStream> + 'static,
    ) -> OpenCallback<'static> {
        let cb: IArchiveOpenCallback = TestVolumes {
            name: name.to_owned(),
            open: Box::new(open),
        }
        .into();
        OpenCallback {
            cb: Some(cb),
            report: RefCell::default(),
            _borrow: PhantomData,
        }
    }

    pub fn set_total(&self, files: Option<u64>, bytes: Option<u64>) -> Result<()> {
        let Some(cb) = &self.cb else { return Ok(()) };
        check(unsafe { cb.SetTotal(files.as_ref(), bytes.as_ref()) })
    }

    /// Fails with [`Error::ABORT`](crate::Error::ABORT) when the user has cancelled.
    pub fn set_completed(&self, files: Option<u64>, bytes: Option<u64>) -> Result<()> {
        let Some(cb) = &self.cb else { return Ok(()) };
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
        self.cb.as_ref().map_or(Ok(None), password)
    }

    /// Access to the archive's file name and sibling files, for multi-volume formats.
    /// `None` when the host can't give them: the archive isn't a file on disk.
    pub fn volumes(&self) -> Option<Volumes> {
        self.cb
            .as_ref()?
            .cast::<IArchiveOpenVolumeCallback>()
            .ok()
            .map(Volumes)
    }
}

/// See [`OpenCallback::volumes`].
pub struct Volumes(IArchiveOpenVolumeCallback);

impl Volumes {
    /// File name of the archive being opened, without its directory.
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

type Opener = Box<dyn Fn(&str) -> Option<InStream>>;

/// The host side of [`OpenCallback::with_volumes`].
#[implement(IArchiveOpenCallback, IArchiveOpenVolumeCallback)]
struct TestVolumes {
    name: String,
    open: Opener,
}

impl IArchiveOpenCallback_Impl for TestVolumes_Impl {
    unsafe fn SetTotal(&self, _files: Option<&u64>, _bytes: Option<&u64>) -> HRESULT {
        HRESULT(0)
    }

    unsafe fn SetCompleted(&self, _files: Option<&u64>, _bytes: Option<&u64>) -> HRESULT {
        HRESULT(0)
    }
}

impl IArchiveOpenVolumeCallback_Impl for TestVolumes_Impl {
    unsafe fn GetProperty(&self, prop_id: PropId, value: OutRef<'_, PropVariant>) -> HRESULT {
        let prop = (prop_id == PropId::Name).then(|| Prop::from(self.name.as_str()));
        to_hresult(Prop::write_to(value, prop))
    }

    unsafe fn GetStream(&self, name: PCWSTR, in_stream: OutRef<'_, IInStream>) -> HRESULT {
        let name = String::from_utf16_lossy(unsafe { name.as_wide() });
        match (self.open)(&name) {
            Some(s) => to_hresult(
                in_stream
                    .write(Some(s.as_com().clone()))
                    .map_err(Error::from),
            ),
            None => {
                let _ = in_stream.write(None);
                S_FALSE
            }
        }
    }
}
