//! [`PropVariant`]: `windows`' `PROPVARIANT` made usable with `windows-core`'s `OutRef`.
//!
//! `windows-core` lets `OutRef<T>` / `&mut T` out-parameters work for any `Clone + Default`
//! type marked `TypeKind = CloneType` (that is how `BSTR` works). `PROPVARIANT` isn't marked,
//! and the orphan rule stops us marking it, so this newtype carries the marker and forwards
//! everything else to the inner `PROPVARIANT`: its `Drop` (`PropVariantClear`), its `From`
//! conversions, and its `TryFrom<&PROPVARIANT>` conversions via [`PropVariant::get`].
//!
//! In interfaces:
//! - out-parameter: `value: OutRef<PropVariant>`. Implement with `value.write(size.into()).into()`;
//!   call with `&mut pv` (any previous value in `pv` is freed first).
//! - in-array: `values: *const PropVariant`, read with [`PropVariant::slice`].

use core::ops::{Deref, DerefMut};
use windows::Win32::Foundation::{FILETIME, SysAllocStringByteLen};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Variant::{VT_EMPTY, VT_FILETIME};

#[repr(transparent)]
#[derive(Clone, Default, Debug, PartialEq)]
pub struct PropVariant(pub PROPVARIANT);

// Same marker windows-core puts on BSTR. `TypeKind`/`CloneType` are public but #[doc(hidden)],
// so re-check this impl when bumping windows-core.
impl windows_core::TypeKind for PropVariant {
    type TypeKind = windows_core::CloneType;
}

impl Deref for PropVariant {
    type Target = PROPVARIANT;
    fn deref(&self) -> &PROPVARIANT {
        &self.0
    }
}

impl DerefMut for PropVariant {
    fn deref_mut(&mut self) -> &mut PROPVARIANT {
        &mut self.0
    }
}

/// Anything `windows` can turn into a `PROPVARIANT`: `u32`, `u64`, `i32`, `i64`, `bool`,
/// `&str`, `BSTR`, `PROPVARIANT`, ...
impl<T: Into<PROPVARIANT>> From<T> for PropVariant {
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

impl PropVariant {
    /// Reads the value with one of `windows`' `TryFrom<&PROPVARIANT>` impls, e.g. `pv.get::<u64>()`.
    pub fn get<'a, T: TryFrom<&'a PROPVARIANT>>(&'a self) -> Result<T, T::Error> {
        T::try_from(&self.0)
    }

    /// `VT_FILETIME` (`windows` has no `From` impl for it).
    pub fn from_filetime(time: FILETIME) -> Self {
        let mut v = Self::default();
        unsafe {
            let inner = &mut v.0.Anonymous.Anonymous;
            inner.vt = VT_FILETIME;
            inner.Anonymous.filetime = time;
        }
        v
    }

    /// Raw bytes as a binary `VT_BSTR`, as used for class ids and signatures.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        unsafe { SysAllocStringByteLen(Some(bytes)) }.into()
    }

    pub fn filetime(&self) -> Option<FILETIME> {
        let inner = unsafe { &self.0.Anonymous.Anonymous };
        (inner.vt == VT_FILETIME).then(|| unsafe { inner.Anonymous.filetime })
    }

    /// `VT_EMPTY`: the property is not available.
    pub fn is_empty(&self) -> bool {
        self.0.vt() == VT_EMPTY
    }

    /// Views a `(*const PropVariant, count)` array parameter as a slice.
    ///
    /// # Safety
    /// `ptr` must point to `len` initialized values that outlive `'a`, or be NULL / `len == 0`.
    pub unsafe fn slice<'a>(ptr: *const PropVariant, len: u32) -> &'a [PropVariant] {
        if ptr.is_null() || len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(ptr, len as usize) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_core::{BSTR, HRESULT, IUnknown, IUnknown_Vtbl, OutRef, implement, interface};

    #[interface("6A0A6D2B-0000-4000-8000-000000000001")]
    unsafe trait IProps: IUnknown {
        fn GetProperty(&self, index: u32, value: OutRef<PropVariant>) -> HRESULT;
    }

    #[implement(IProps)]
    struct Props;

    impl IProps_Impl for Props_Impl {
        unsafe fn GetProperty(&self, index: u32, value: OutRef<PropVariant>) -> HRESULT {
            match index {
                0 => value.write(42u64.into()).into(),
                1 => value.write("dir/file.bin".into()).into(),
                2 => value
                    .write(PropVariant::from_filetime(FILETIME {
                        dwLowDateTime: 1,
                        dwHighDateTime: 2,
                    }))
                    .into(),
                3 => value.write(PropVariant::from_bytes(b"MYF")).into(),
                _ => HRESULT(0), // not available: stays VT_EMPTY
            }
        }
    }

    #[test]
    fn layout() {
        assert_eq!(size_of::<PropVariant>(), size_of::<PROPVARIANT>());
    }

    /// Implemented in Rust, called through the vtable; `pv` is reused, so each call must free
    /// the previous value (BSTR, FILETIME, bytes) without leaking or double-freeing.
    #[test]
    fn out_ref_round_trip() {
        let props: IProps = Props.into();
        let mut pv = PropVariant::default();

        unsafe { props.GetProperty(0, &mut pv) }.unwrap();
        assert_eq!(pv.get::<u64>().unwrap(), 42);

        unsafe { props.GetProperty(1, &mut pv) }.unwrap();
        assert_eq!(pv.get::<BSTR>().unwrap(), "dir/file.bin");

        unsafe { props.GetProperty(2, &mut pv) }.unwrap();
        assert_eq!(
            pv.filetime(),
            Some(FILETIME {
                dwLowDateTime: 1,
                dwHighDateTime: 2
            })
        );

        // Odd length on purpose: a binary BSTR keeps its exact byte count.
        unsafe { props.GetProperty(3, &mut pv) }.unwrap();
        let p = unsafe { pv.Anonymous.Anonymous.Anonymous.bstrVal.as_ptr() }.cast::<u8>();
        assert_eq!(unsafe { p.cast::<u32>().sub(1).read_unaligned() }, 3);
        assert_eq!(unsafe { core::slice::from_raw_parts(p, 3) }, b"MYF");

        unsafe { props.GetProperty(9, &mut pv) }.unwrap();
        assert!(pv.is_empty());
    }

    #[test]
    fn slice_helper() {
        let vals = [PropVariant::from(1u32), PropVariant::from(true)];
        let s = unsafe { PropVariant::slice(vals.as_ptr(), 2) };
        assert_eq!(s[0].get::<u32>().unwrap(), 1);
        assert!(s[1].get::<bool>().unwrap());
        assert!(unsafe { PropVariant::slice(core::ptr::null(), 5) }.is_empty());
    }
}
