//! [`RawProp`]: property data 7-Zip reads in place, from
//! [`Archive::raw_prop`](crate::Archive::raw_prop).

use crate::error::Result;
use core::ffi::c_void;
use lol7z_sys::i_archive::PropDataType;
use lol7z_sys::propid::PropId;
use windows_core::{BSTR, OutRef};

/// Property data handed to 7-Zip as a pointer into the archive, with no BSTR made.
/// It must stay where it is until the archive is closed, which borrowing from `&self`
/// guarantees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawProp<'a> {
    /// UTF-16 text whose last element is 0. 7-Zip reads `Path` (when the archive isn't
    /// a tree) and `Name` (when it is) this way, so listing a large archive allocates
    /// nothing per item. Text without the 0 is treated as not set.
    Utf16Z(&'a [u16]),
    /// Binary data, such as an NT security descriptor (`NtSecure`) or a stored hash.
    Bytes(&'a [u8]),
}

impl RawProp<'_> {
    /// `(data, size in bytes, type)` for `GetRawProp`. Null and `NotDefined` when the
    /// text isn't 0-terminated or doesn't fit the u32 size.
    pub(crate) fn to_raw(prop: Option<Self>) -> (*const c_void, u32, PropDataType) {
        const NONE: (*const c_void, u32, PropDataType) =
            (core::ptr::null(), 0, PropDataType::NotDefined);
        let (ptr, bytes, kind) = match prop {
            Some(RawProp::Utf16Z(s)) if s.last() == Some(&0) => {
                (s.as_ptr().cast(), size_of_val(s), PropDataType::Utf16Z)
            }
            Some(RawProp::Bytes(b)) => (b.as_ptr().cast(), b.len(), PropDataType::Raw),
            _ => return NONE,
        };
        match u32::try_from(bytes) {
            Ok(n) => (ptr, n, kind),
            Err(_) => NONE,
        }
    }
}

/// One entry of [`Archive::RAW_PROPS`](crate::Archive::RAW_PROPS): a raw property 7-Zip
/// can show as a column. Raw `Path` and `Name` don't need an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawPropInfo {
    pub id: PropId,
    /// `None` for standard properties.
    pub name: Option<&'static str>,
}

impl RawPropInfo {
    pub const fn new(id: PropId) -> Self {
        Self { id, name: None }
    }

    pub const fn custom(id: PropId, name: &'static str) -> Self {
        Self {
            id,
            name: Some(name),
        }
    }

    /// The `GetRawPropInfo` out-parameters. A null `name` means a standard property.
    pub(crate) fn write_to(&self, name: OutRef<'_, BSTR>, prop_id: OutRef<'_, u32>) -> Result<()> {
        name.write(self.name.map_or_else(BSTR::new, BSTR::from))?;
        Ok(prop_id.write(self.id.0)?)
    }
}
