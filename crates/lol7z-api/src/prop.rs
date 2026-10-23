//! [`Prop`]: a property value.

use crate::error::{Result, check};
use lol7z_sys::propvar::PropVariant;
use std::borrow::Cow;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Variant::{VT_BOOL, VT_BSTR, VT_FILETIME, VT_UI4, VT_UI8};
use windows_core::{BSTR, HRESULT, OutRef};

/// A property value. Use the variant 7-Zip expects for the property ([`standard_vt`](crate::standard_vt)):
/// `Path` is `Str`, `Size` is `U64`, `MTime` is `FileTime`, `IsDir` is `Bool`, and so on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prop<'a> {
    Bool(bool),
    U32(u32),
    U64(u64),
    Str(Cow<'a, str>),
    /// 100 ns ticks since 1601-01-01 UTC, as in a Windows `FILETIME`.
    FileTime(u64),
    /// Binary data, sent as a BSTR holding raw bytes.
    Bytes(Cow<'a, [u8]>),
}

/// Ticks between 1601-01-01 and 1970-01-01.
const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;

impl Prop<'_> {
    /// Calls a 7-Zip method that fills in a `PROPVARIANT`, and reads what it wrote. `None`
    /// when there's no value.
    pub(crate) fn read_from(
        get: impl FnOnce(&mut PropVariant) -> HRESULT,
    ) -> Result<Option<Prop<'static>>> {
        let mut v = PropVariant::default();
        check(get(&mut v))?;
        Ok(Prop::from_variant(&v))
    }

    /// "No value" is an empty `PROPVARIANT`.
    pub(crate) fn write_to(value: OutRef<'_, PropVariant>, prop: Option<Prop<'_>>) -> Result<()> {
        Ok(value.write(prop.map_or_else(PropVariant::default, Prop::into_variant))?)
    }

    /// A `FileTime` from a `SystemTime`, clamped to the range `FILETIME` can hold.
    pub fn time(t: SystemTime) -> Prop<'static> {
        let ticks = |d: std::time::Duration| {
            d.as_secs()
                .saturating_mul(10_000_000)
                .saturating_add(u64::from(d.subsec_nanos() / 100))
        };
        Prop::FileTime(match t.duration_since(UNIX_EPOCH) {
            Ok(d) => UNIX_EPOCH_TICKS.saturating_add(ticks(d)),
            Err(e) => UNIX_EPOCH_TICKS.saturating_sub(ticks(e.duration())),
        })
    }

    pub fn into_owned(self) -> Prop<'static> {
        match self {
            Prop::Bool(b) => Prop::Bool(b),
            Prop::U32(n) => Prop::U32(n),
            Prop::U64(n) => Prop::U64(n),
            Prop::Str(s) => Prop::Str(Cow::Owned(s.into_owned())),
            Prop::FileTime(t) => Prop::FileTime(t),
            Prop::Bytes(b) => Prop::Bytes(Cow::Owned(b.into_owned())),
        }
    }

    pub fn into_variant(self) -> PropVariant {
        match self {
            Prop::Bool(b) => b.into(),
            Prop::U32(n) => n.into(),
            Prop::U64(n) => n.into(),
            Prop::Str(s) => (&*s).into(),
            Prop::FileTime(t) => PropVariant::from_filetime(FILETIME {
                dwLowDateTime: t as u32,
                dwHighDateTime: (t >> 32) as u32,
            }),
            Prop::Bytes(b) => PropVariant::from_bytes(&b),
        }
    }

    /// `None` for `VT_EMPTY` and for types this enum has no variant for.
    /// A `VT_BSTR` is read as text; nothing 7-Zip sends to a handler is binary.
    pub fn from_variant(v: &PropVariant) -> Option<Prop<'static>> {
        let vt = v.vt();
        Some(if vt == VT_BOOL {
            Prop::Bool(v.get::<bool>().ok()?)
        } else if vt == VT_UI4 {
            Prop::U32(v.get::<u32>().ok()?)
        } else if vt == VT_UI8 {
            Prop::U64(v.get::<u64>().ok()?)
        } else if vt == VT_BSTR {
            Prop::Str(Cow::Owned(v.get::<BSTR>().ok()?.to_string()))
        } else if vt == VT_FILETIME {
            let t = v.filetime()?;
            Prop::FileTime(u64::from(t.dwHighDateTime) << 32 | u64::from(t.dwLowDateTime))
        } else {
            return None;
        })
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The text, moved out without a copy when it's already owned.
    pub fn into_string(self) -> Option<String> {
        match self {
            Prop::Str(s) => Some(s.into_owned()),
            _ => None,
        }
    }

    /// `U32` or `U64`.
    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            Prop::U32(n) => Some(n.into()),
            Prop::U64(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Prop::Bool(b) => Some(b),
            _ => None,
        }
    }
}

impl From<bool> for Prop<'_> {
    fn from(b: bool) -> Self {
        Prop::Bool(b)
    }
}

impl From<u32> for Prop<'_> {
    fn from(n: u32) -> Self {
        Prop::U32(n)
    }
}

impl From<u64> for Prop<'_> {
    fn from(n: u64) -> Self {
        Prop::U64(n)
    }
}

impl<'a> From<&'a str> for Prop<'a> {
    fn from(s: &'a str) -> Self {
        Prop::Str(Cow::Borrowed(s))
    }
}

impl From<String> for Prop<'_> {
    fn from(s: String) -> Self {
        Prop::Str(Cow::Owned(s))
    }
}
