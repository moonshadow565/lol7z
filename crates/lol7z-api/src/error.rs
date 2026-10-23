//! [`Error`]: a failure 7-Zip can show, either as an `HRESULT` or as damage flags.

use core::fmt;
use lol7z_sys::i_archive::OperationResult;
use lol7z_sys::propid::ErrorFlags;
use std::collections::TryReserveError;
use std::io;
use windows::Win32::Foundation::{
    E_ABORT, E_FAIL, E_INVALIDARG, E_NOTIMPL, E_OUTOFMEMORY, E_UNEXPECTED, S_FALSE,
};
use windows_core::HRESULT;

pub type Result<T, E = Error> = core::result::Result<T, E>;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// From [`Format::open`](crate::Format::open): the stream is not this format, and
    /// 7-Zip should try other handlers.
    NotArchive,
    /// The data is this format but damaged. 7-Zip shows the flags: "Unexpected end of
    /// archive", "Headers Error", "CRC Failed" and so on.
    ///
    /// From `open`, 7-Zip reports the archive as broken and stops trying other formats.
    /// Extracting, the default [`Archive::extract`](crate::Archive::extract) reports it as
    /// the item's result and goes on with the next item.
    ///
    /// `?` on an `io::Error` of kind `UnexpectedEof` gives `UnexpectedEnd`, and of kind
    /// `InvalidData` gives `DataError`.
    Damaged(ErrorFlags),
    /// Anything else: the user cancelled, a read failed, out of memory. Passed to 7-Zip
    /// as it is, and it stops the operation.
    Hresult(HRESULT),
}

impl Error {
    pub const FAIL: Self = Self::Hresult(E_FAIL);
    /// The user cancelled. 7-Zip's callbacks return it; pass it on with `?`.
    pub const ABORT: Self = Self::Hresult(E_ABORT);
    pub const NOT_IMPL: Self = Self::Hresult(E_NOTIMPL);
    pub const INVALID_ARG: Self = Self::Hresult(E_INVALIDARG);
    pub const OUT_OF_MEMORY: Self = Self::Hresult(E_OUTOFMEMORY);
    pub const UNEXPECTED: Self = Self::Hresult(E_UNEXPECTED);
    pub const UNEXPECTED_END: Self = Self::Damaged(ErrorFlags::UnexpectedEnd);
    pub const HEADERS_ERROR: Self = Self::Damaged(ErrorFlags::HeadersError);
    pub const DATA_ERROR: Self = Self::Damaged(ErrorFlags::DataError);
    pub const CRC_ERROR: Self = Self::Damaged(ErrorFlags::CrcError);

    /// What 7-Zip gets when this is returned from a COM method. `Open` sends
    /// [`Damaged`](Self::Damaged) as `S_FALSE` plus the flags; anywhere else it has no
    /// HRESULT of its own and becomes `E_FAIL`.
    pub fn code(self) -> HRESULT {
        match self {
            Self::NotArchive => S_FALSE,
            Self::Damaged(_) => E_FAIL,
            Self::Hresult(hr) => hr,
        }
    }

    /// The item result for this error, or `None` when it should stop the operation.
    pub fn operation_result(self) -> Option<OperationResult> {
        let flags = match self {
            Self::NotArchive => return Some(OperationResult::IsNotArc),
            Self::Damaged(flags) => flags,
            Self::Hresult(_) => return None,
        };
        let pairs = [
            (ErrorFlags::UnexpectedEnd, OperationResult::UnexpectedEnd),
            (ErrorFlags::CrcError, OperationResult::CRCError),
            (
                ErrorFlags::UnsupportedMethod,
                OperationResult::UnsupportedMethod,
            ),
            (
                ErrorFlags::UnsupportedFeature,
                OperationResult::UnsupportedMethod,
            ),
            (ErrorFlags::Unsupported, OperationResult::UnsupportedMethod),
            (ErrorFlags::HeadersError, OperationResult::HeadersError),
            (ErrorFlags::DataAfterEnd, OperationResult::DataAfterEnd),
            (ErrorFlags::IsNotArc, OperationResult::IsNotArc),
        ];
        Some(
            pairs
                .into_iter()
                .find(|(f, _)| flags.contains(*f))
                .map_or(OperationResult::DataError, |(_, r)| r),
        )
    }
}

/// Fails on error HRESULTs only: `S_FALSE` is a success code and means something
/// different at each call site; [`found`] handles the common meaning.
pub(crate) fn check(hr: HRESULT) -> Result<()> {
    if hr.is_ok() {
        Ok(())
    } else {
        Err(Error::Hresult(hr))
    }
}

/// For 7-Zip methods that answer `S_FALSE` for "there isn't one" (a file that can't be
/// opened, a volume that doesn't exist): `value` unless the call said so or failed.
pub(crate) fn found<T>(hr: HRESULT, value: Option<T>) -> Result<Option<T>> {
    if hr == S_FALSE {
        return Ok(None);
    }
    check(hr)?;
    Ok(value)
}

pub(crate) fn to_hresult<E: Into<Error>>(r: Result<(), E>) -> HRESULT {
    match r {
        Ok(()) => HRESULT(0),
        Err(e) => e.into().code(),
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotArchive => f.write_str("NotArchive"),
            Self::Damaged(flags) => write!(f, "Damaged({flags:?})"),
            Self::Hresult(hr) => write!(f, "Hresult({:#010x})", hr.0 as u32),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotArchive => f.write_str("not an archive of this format"),
            Self::Damaged(flags) => write!(f, "damaged archive: {flags:?}"),
            Self::Hresult(hr) => write!(f, "{:#010x}: {}", hr.0 as u32, hr.message()),
        }
    }
}

impl std::error::Error for Error {}

impl From<windows_core::Error> for Error {
    fn from(e: windows_core::Error) -> Self {
        Self::Hresult(e.code())
    }
}

impl From<HRESULT> for Error {
    fn from(hr: HRESULT) -> Self {
        Self::Hresult(hr)
    }
}

impl From<TryReserveError> for Error {
    fn from(_: TryReserveError) -> Self {
        Self::OUT_OF_MEMORY
    }
}

/// Errors from 7-Zip streams travel through `std::io` as raw OS errors holding the
/// HRESULT, so `E_ABORT` from a write is still `E_ABORT` after `?`. An [`Error`] wrapped
/// in an `io::Error` comes back out unchanged.
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        if let Some(inner) = e.get_ref().and_then(|i| i.downcast_ref::<Error>()) {
            return *inner;
        }
        match e.raw_os_error() {
            Some(code) if code < 0 => Self::Hresult(HRESULT(code)),
            Some(code) => Self::Hresult(HRESULT::from_win32(code as u32)),
            None => match e.kind() {
                io::ErrorKind::UnexpectedEof => Self::UNEXPECTED_END,
                io::ErrorKind::InvalidData => Self::DATA_ERROR,
                io::ErrorKind::OutOfMemory => Self::OUT_OF_MEMORY,
                _ => Self::FAIL,
            },
        }
    }
}

impl From<Error> for io::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Hresult(hr) => io::Error::from_raw_os_error(hr.0),
            Error::Damaged(flags) if flags.contains(ErrorFlags::UnexpectedEnd) => {
                io::Error::new(io::ErrorKind::UnexpectedEof, e)
            }
            _ => io::Error::new(io::ErrorKind::InvalidData, e),
        }
    }
}

impl From<Error> for HRESULT {
    fn from(e: Error) -> Self {
        e.code()
    }
}
