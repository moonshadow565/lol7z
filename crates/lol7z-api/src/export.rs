//! The DLL exports 7-Zip looks for in a plugin. [`formats!`](crate::formats) generates
//! them; the functions here do the work.

use crate::error::to_hresult;
use crate::format::{Format, check};
use crate::handler::{Handler, UpdateHandler};
use crate::update::Update;
use core::ffi::c_void;
use lol7z_sys::i_archive::{ArcInfoFlags, Func_IsArc, HandlerPropId, IsArcResult};
use lol7z_sys::propvar::PropVariant;
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, E_INVALIDARG, E_POINTER};
use windows_core::{GUID, HRESULT, IUnknown, Interface, OutRef};

/// One registered format. Built by [`formats!`](crate::formats).
#[doc(hidden)]
pub struct Entry {
    class_id: GUID,
    create: fn() -> IUnknown,
    prop: fn(HandlerPropId) -> PropVariant,
    is_arc: Func_IsArc,
}

impl Entry {
    /// Evaluated in a `static`, so a bad constant in `F` fails the build.
    pub const fn read<F: Format>() -> Self {
        check::<F>();
        Self {
            class_id: F::CLASS_ID,
            create: || Handler::<F>::new().into(),
            prop: Self::handler_prop::<F, false>,
            is_arc: Self::is_arc::<F>,
        }
    }

    pub const fn update<F: Update>() -> Self {
        check::<F>();
        Self {
            class_id: F::CLASS_ID,
            create: || UpdateHandler::<F>::new().into(),
            prop: Self::handler_prop::<F, true>,
            is_arc: Self::is_arc::<F>,
        }
    }

    /// The answers LoadCodecs.cpp reads at startup. Types matter: it rejects the whole
    /// DLL when a value has the wrong VT.
    fn handler_prop<F: Format, const UPDATE: bool>(id: HandlerPropId) -> PropVariant {
        let exts = F::EXTENSIONS;
        let sigs = F::SIGNATURES;
        let join = |parts: &mut dyn Iterator<Item = &str>| parts.collect::<Vec<_>>().join(" ");
        match id {
            HandlerPropId::Name => F::NAME.into(),
            // The GUID as it sits in memory, which is how 7-Zip reads it back.
            HandlerPropId::ClassID => {
                let g = F::CLASS_ID;
                let mut b = [0; 16];
                b[0..4].copy_from_slice(&g.data1.to_le_bytes());
                b[4..6].copy_from_slice(&g.data2.to_le_bytes());
                b[6..8].copy_from_slice(&g.data3.to_le_bytes());
                b[8..16].copy_from_slice(&g.data4);
                PropVariant::from_bytes(&b)
            }
            // Space-separated lists, matched up by position; "*" fills a gap (AddExts in
            // LoadCodecs.cpp).
            HandlerPropId::Extension => join(&mut exts.iter().map(|e| e.ext)).as_str().into(),
            HandlerPropId::AddExtension if exts.iter().any(|e| e.inner.is_some()) => {
                join(&mut exts.iter().map(|e| e.inner.unwrap_or("*")))
                    .as_str()
                    .into()
            }
            HandlerPropId::Update => UPDATE.into(),
            HandlerPropId::Flags => {
                let multi = if sigs.len() > 1 {
                    ArcInfoFlags::MultiSignature
                } else {
                    ArcInfoFlags::empty()
                };
                (F::FLAGS | multi).bits().into()
            }
            HandlerPropId::TimeFlags if F::TIME_FLAGS.0 != 0 => F::TIME_FLAGS.0.into(),
            HandlerPropId::Signature if sigs.len() == 1 => PropVariant::from_bytes(sigs[0]),
            // Each signature prefixed by its length (ParseSignatures in LoadCodecs.cpp).
            HandlerPropId::MultiSignature if sigs.len() > 1 => {
                let mut b = Vec::new();
                for sig in sigs {
                    b.push(sig.len() as u8);
                    b.extend_from_slice(sig);
                }
                PropVariant::from_bytes(&b)
            }
            HandlerPropId::SignatureOffset => F::SIGNATURE_OFFSET.into(),
            _ => PropVariant::default(),
        }
    }

    unsafe extern "system" fn is_arc<F: Format>(p: *const u8, size: usize) -> IsArcResult {
        let head = if p.is_null() || size == 0 {
            &[][..]
        } else {
            unsafe { core::slice::from_raw_parts(p, size) }
        };
        F::is_arc(head)
    }
}

/// `major.minor` as `GetModuleProp` reports it (`7z i` prints it; Main.cpp:1101).
pub const fn version(major: &str, minor: &str) -> u32 {
    (parse_u16(major) as u32) << 16 | parse_u16(minor) as u32
}

const fn parse_u16(s: &str) -> u16 {
    let b = s.as_bytes();
    let mut n: u16 = 0;
    let mut i = 0;
    while i < b.len() {
        assert!(b[i].is_ascii_digit(), "version parts must be numbers");
        n = n * 10 + (b[i] - b'0') as u16;
        i += 1;
    }
    n
}

/// The formats a plugin exports; each method is one DLL export.
#[doc(hidden)]
pub struct Formats<'a>(pub &'a [Entry]);

impl Formats<'_> {
    /// # Safety
    /// The pointers are 7-Zip's `CreateObject` arguments.
    pub unsafe fn create_object(
        &self,
        cls_id: *const GUID,
        iid: *const GUID,
        out_object: *mut *mut c_void,
    ) -> HRESULT {
        if cls_id.is_null() || iid.is_null() || out_object.is_null() {
            return E_POINTER;
        }
        unsafe { *out_object = core::ptr::null_mut() };
        let cls_id = unsafe { *cls_id };
        let Some(entry) = self.0.iter().find(|e| e.class_id == cls_id) else {
            return CLASS_E_CLASSNOTAVAILABLE;
        };
        // A read-only handler answers E_NOINTERFACE for IOutArchive here.
        unsafe { (entry.create)().query(iid, out_object) }
    }

    /// # Safety
    /// `num_formats` is 7-Zip's `GetNumberOfFormats` argument.
    pub unsafe fn number_of_formats(&self, num_formats: *mut u32) -> HRESULT {
        if num_formats.is_null() {
            return E_POINTER;
        }
        unsafe { *num_formats = self.0.len() as u32 };
        HRESULT(0)
    }

    pub fn handler_property2(
        &self,
        index: u32,
        prop_id: HandlerPropId,
        value: OutRef<'_, PropVariant>,
    ) -> HRESULT {
        let Some(entry) = self.0.get(index as usize) else {
            return E_INVALIDARG;
        };
        to_hresult(value.write((entry.prop)(prop_id)))
    }

    /// # Safety
    /// `is_arc` is 7-Zip's `GetIsArc` argument.
    pub unsafe fn is_arc(&self, index: u32, is_arc: *mut Option<Func_IsArc>) -> HRESULT {
        if is_arc.is_null() {
            return E_POINTER;
        }
        let f = self.0.get(index as usize).map(|e| e.is_arc);
        unsafe { *is_arc = f };
        if f.is_some() {
            HRESULT(0)
        } else {
            E_INVALIDARG
        }
    }

    /// `NModulePropID` (ICoder.h:443). `kInterfaceType` is 0, no virtual destructor in
    /// IUnknown, which is what 7-Zip expects from a Windows module; `kVersion` is
    /// [`version`].
    pub fn module_prop(
        &self,
        prop_id: u32,
        value: OutRef<'_, PropVariant>,
        version: u32,
    ) -> HRESULT {
        let v = match prop_id {
            0 => 0u32.into(),
            1 => version.into(),
            _ => PropVariant::default(),
        };
        to_hresult(value.write(v))
    }
}

/// Exports `formats` from this DLL as a 7-Zip plugin: `CreateObject`,
/// `GetNumberOfFormats`, `GetHandlerProperty2`, `GetIsArc` and `GetModuleProp`.
/// Each entry is `read Type` or `update Type`, where `Type` implements [`Format`] or
/// [`Update`]. Use it once, in a `cdylib` crate. `7z i` shows the crate's
/// `major.minor` version.
///
/// ```ignore
/// lol7z_api::formats! {
///     update Wad,
///     read Bin,
/// }
/// ```
///
/// Drop the DLL in the `Formats` folder next to `7z.dll`.
#[macro_export]
macro_rules! formats {
    ($($kind:ident $format:ty),+ $(,)?) => {
        static __LOL7Z_FORMATS: $crate::export::Formats<'static> =
            $crate::export::Formats(&[$($crate::__format_entry!($kind $format)),+]);

        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn CreateObject(
            cls_id: *const $crate::GUID,
            iid: *const $crate::GUID,
            out_object: *mut *mut ::core::ffi::c_void,
        ) -> $crate::HRESULT {
            unsafe { __LOL7Z_FORMATS.create_object(cls_id, iid, out_object) }
        }

        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn GetNumberOfFormats(num_formats: *mut u32) -> $crate::HRESULT {
            unsafe { __LOL7Z_FORMATS.number_of_formats(num_formats) }
        }

        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn GetHandlerProperty2(
            index: u32,
            prop_id: $crate::sys::i_archive::HandlerPropId,
            value: $crate::OutRef<'_, $crate::sys::propvar::PropVariant>,
        ) -> $crate::HRESULT {
            __LOL7Z_FORMATS.handler_property2(index, prop_id, value)
        }

        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn GetIsArc(
            index: u32,
            is_arc: *mut ::core::option::Option<$crate::sys::i_archive::Func_IsArc>,
        ) -> $crate::HRESULT {
            unsafe { __LOL7Z_FORMATS.is_arc(index, is_arc) }
        }

        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn GetModuleProp(
            prop_id: u32,
            value: $crate::OutRef<'_, $crate::sys::propvar::PropVariant>,
        ) -> $crate::HRESULT {
            // The plugin crate's own version: `env!` expands in that crate.
            const VERSION: u32 = $crate::export::version(
                env!("CARGO_PKG_VERSION_MAJOR"),
                env!("CARGO_PKG_VERSION_MINOR"),
            );
            __LOL7Z_FORMATS.module_prop(prop_id, value, VERSION)
        }

        // The exports must match the signatures 7-Zip calls them with.
        const _: () = {
            let _: $crate::sys::i_archive::Func_CreateObject = CreateObject;
            let _: $crate::sys::i_archive::Func_GetNumberOfFormats = GetNumberOfFormats;
            let _: $crate::sys::i_archive::Func_GetHandlerProperty2 = GetHandlerProperty2;
            let _: $crate::sys::i_archive::Func_GetIsArc = GetIsArc;
        };
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __format_entry {
    (read $format:ty) => {
        $crate::export::Entry::read::<$format>()
    };
    (update $format:ty) => {
        $crate::export::Entry::update::<$format>()
    };
}
