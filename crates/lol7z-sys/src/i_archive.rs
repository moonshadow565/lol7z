//! Archive handler interfaces (7-Zip `CPP/7zip/Archive/IArchive.h`). IID group 6.
//!
//! Enums are open (`open-enum`): any integer is a valid value, so they can appear in
//! FFI signatures even when 7-Zip sends a value this crate doesn't name. Flag types are
//! `#[repr(transparent)]` for the same reason.

use crate::i_progress::{IProgress, IProgress_Impl, IProgress_Vtbl};
use crate::i_stream::{IInStream, ISequentialInStream, ISequentialOutStream};
use crate::propid::PropId;
use crate::propvar::PropVariant;
use bitflags::bitflags;
use core::ffi::c_void;
use open_enum::open_enum;
use windows_core::{BSTR, GUID, HRESULT, IUnknown, IUnknown_Vtbl, OutRef, PCWSTR, Ref, interface};

// ---------------------------------------------------------------- enums / flags

/// IArchive.h:44. Returned by `IOutArchive::GetFileTimeType`.
#[open_enum]
#[repr(i32)]
#[derive(Debug, Clone, Copy, Hash)]
pub enum FileTimeType {
    NotDefined = -1,
    Windows = 0,
    Unix,
    Dos,
    Ns1,
}

bitflags! {
    /// IArchive.h:56. Value of the `HandlerPropId::Flags` handler property.
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ArcInfoFlags: u32 {
        const KeepName        = 1 << 0;
        const AltStreams      = 1 << 1;
        const NtSecure        = 1 << 2;
        const FindSignature   = 1 << 3;
        const MultiSignature  = 1 << 4;
        const UseGlobalOffset = 1 << 5;
        const StartOpen       = 1 << 6;
        const PureStartOpen   = 1 << 7;
        const BackwardOpen    = 1 << 8;
        const PreArc          = 1 << 9;
        const SymLinks        = 1 << 10;
        const HardLinks       = 1 << 11;
        const ByExtOnlyOpen   = 1 << 12;
        const HashHandler     = 1 << 13;
        const CTime           = 1 << 14;
        const CTime_Default   = 1 << 15;
        const ATime           = 1 << 16;
        const ATime_Default   = 1 << 17;
        const MTime           = 1 << 18;
        const MTime_Default   = 1 << 19;
    }
}

/// IArchive.h:82. Value of the `HandlerPropId::TimeFlags` handler property.
///
/// Packs two fields: bits 0..=25 hold one bit per supported timestamp precision, and
/// bits 27..=31 hold the default precision as a number. Precision values are those of
/// `FileTimeType` (0..=3), or 16 + N for N decimal digits of sub-second precision.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ArcTimeFlags(pub u32);

impl ArcTimeFlags {
    const DEFAULT_SHIFT: u32 = 27;
    const DEFAULT_MASK: u32 = 0x1F << Self::DEFAULT_SHIFT;

    pub const fn new() -> Self {
        Self(0)
    }

    /// Marks `prec` (0..=25) as supported.
    pub const fn with(self, prec: u32) -> Self {
        Self(self.0 | 1 << prec)
    }

    /// Sets the default precision (0..=31).
    pub const fn with_default(self, prec: u32) -> Self {
        Self((self.0 & !Self::DEFAULT_MASK) | (prec << Self::DEFAULT_SHIFT))
    }

    pub const fn supports(self, prec: u32) -> bool {
        self.0 & (1 << prec) != 0
    }

    pub const fn default_prec(self) -> u32 {
        self.0 >> Self::DEFAULT_SHIFT
    }
}

/// IArchive.h:99. Property ids for the `GetHandlerProperty2` export.
#[open_enum]
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialOrd, Ord, Hash)]
pub enum HandlerPropId {
    Name = 0,
    ClassID,
    Extension,
    AddExtension,
    Update,
    KeepName,
    Signature,
    MultiSignature,
    SignatureOffset,
    AltStreams,
    NtSecure,
    Flags,
    TimeFlags,
}

/// IArchive.h:121. `ask_extract_mode` in `IArchiveExtractCallback`.
#[open_enum]
#[repr(i32)]
#[derive(Debug, Clone, Copy, Hash)]
pub enum AskMode {
    Extract = 0,
    Test,
    Skip,
    ReadExternal,
}

/// IArchive.h:132. `op_res` in `IArchiveExtractCallback::SetOperationResult`.
#[open_enum]
#[repr(i32)]
#[derive(Debug, Clone, Copy, Hash)]
pub enum OperationResult {
    OK = 0,
    UnsupportedMethod,
    DataError,
    CRCError,
    Unavailable,
    UnexpectedEnd,
    DataAfterEnd,
    IsNotArc,
    HeadersError,
    WrongPassword,
}

/// IArchive.h:151
#[open_enum]
#[repr(u32)]
#[derive(Debug, Clone, Copy, Hash)]
pub enum EventIndexType {
    NoIndex = 0,
    InArcIndex,
    BlockIndex,
    OutArcIndex,
}

/// IArchive.h:330. `parent_type` in `IArchiveGetRawProps::GetParent`.
#[open_enum]
#[repr(u32)]
#[derive(Debug, Clone, Copy, Hash)]
pub enum ParentType {
    Dir = 0,
    AltStream,
}

bitflags! {
    /// IArchive.h:339. `prop_type` in `GetRawProp`. Bit 0 doubles as `Raw` and as the
    /// UTF-16 selector under `Utf`, so test with `==` against the named values.
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PropDataType: u32 {
        const NotDefined = 0;
        const Raw        = 1;
        const ZeroEnd    = 1 << 4;
        const Utf        = 1 << 6;
        const Utf8       = Self::Utf.bits();
        const Utf16      = Self::Utf.bits() | 1;
        const Utf8Z      = Self::Utf8.bits() | Self::ZeroEnd.bits();
        const Utf16Z     = Self::Utf16.bits() | Self::ZeroEnd.bits();
    }
}

/// IArchive.h:460. `notify_op` in `IArchiveUpdateCallbackFile`.
#[open_enum]
#[repr(u32)]
#[derive(Debug, Clone, Copy, Hash)]
pub enum UpdateNotifyOp {
    Add = 0,
    Update,
    Analyze,
    Replicate,
    Repack,
    Skip,
    Delete,
    Header,
    HashRead,
    InFileChanged,
}

bitflags! {
    /// IArchive.h:568. `flags` in `RequestMemoryUse`.
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct RequestMemoryUseFlags: u32 {
        const AllowedSize_WasForced = 1 << 0;
        const DefaultLimit_Exceeded = 1 << 1;
        const MLimit_Exceeded       = 1 << 2;
        const SLimit_Exceeded       = 1 << 3;
        const NoErrorMessage        = 1 << 10;
        const IsReport              = 1 << 11;
        const SkipArc_IsExpected    = 1 << 12;
        const Report_SkipArc        = 1 << 13;
    }
}

bitflags! {
    /// IArchive.h:588. `answer_flags` in `RequestMemoryUse`.
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct RequestMemoryAnswerFlags: u32 {
        const Allow          = 1 << 0;
        const Stop           = 1 << 1;
        const SkipArc        = 1 << 2;
        const Limit_Exceeded = 1 << 10;
    }
}

/// IArchive.h:696. Return value of an `IsArc` function.
#[open_enum]
#[repr(u32)]
#[derive(Debug, Clone, Copy, Hash)]
pub enum IsArcResult {
    No = 0,
    Yes = 1,
    NeedMore = 2,
}

// ---------------------------------------------------------------- interfaces

/// Progress reporting while a handler opens an archive.
#[interface("23170F69-40C1-278A-0000-000600100000")]
pub unsafe trait IArchiveOpenCallback: IUnknown {
    pub fn SetTotal(&self, files: Option<&u64>, bytes: Option<&u64>) -> HRESULT;
    pub fn SetCompleted(&self, files: Option<&u64>, bytes: Option<&u64>) -> HRESULT;
}

/// Host side of `IInArchive::Extract`. Contract: IArchive.h:183
#[interface("23170F69-40C1-278A-0000-000600200000")]
pub unsafe trait IArchiveExtractCallback: IProgress {
    pub fn GetStream(
        &self,
        index: u32,
        out_stream: OutRef<ISequentialOutStream>,
        ask_extract_mode: AskMode,
    ) -> HRESULT;
    pub fn PrepareOperation(&self, ask_extract_mode: AskMode) -> HRESULT;
    pub fn SetOperationResult(&self, op_res: OperationResult) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600220000")]
pub unsafe trait IArchiveExtractCallbackMessage2: IUnknown {
    pub fn ReportExtractResult(
        &self,
        index_type: EventIndexType,
        index: u32,
        op_res: OperationResult,
    ) -> HRESULT;
}

/// For multi-volume archives; query it from the open callback.
#[interface("23170F69-40C1-278A-0000-000600300000")]
pub unsafe trait IArchiveOpenVolumeCallback: IUnknown {
    pub fn GetProperty(&self, prop_id: PropId, value: OutRef<PropVariant>) -> HRESULT;
    pub fn GetStream(&self, name: PCWSTR, in_stream: OutRef<IInStream>) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600400000")]
pub unsafe trait IInArchiveGetStream: IUnknown {
    pub fn GetStream(&self, index: u32, stream: OutRef<ISequentialInStream>) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600500000")]
pub unsafe trait IArchiveOpenSetSubArchiveName: IUnknown {
    pub fn SetSubArchiveName(&self, name: PCWSTR) -> HRESULT;
}

/// The archive handler a format DLL implements. Contract: IArchive.h:277
///
/// The `prop_id` out-parameters of the `*Info` methods stay `u32`: `OutRef` only supports
/// `windows-core` types. Write `PropId::Size.0` and so on.
#[interface("23170F69-40C1-278A-0000-000600600000")]
pub unsafe trait IInArchive: IUnknown {
    pub fn Open(
        &self,
        stream: Ref<IInStream>,
        max_check_start_position: Option<&u64>,
        open_callback: Ref<IArchiveOpenCallback>,
    ) -> HRESULT;
    pub fn Close(&self) -> HRESULT;
    pub fn GetNumberOfItems(&self, num_items: OutRef<u32>) -> HRESULT;
    pub fn GetProperty(&self, index: u32, prop_id: PropId, value: OutRef<PropVariant>) -> HRESULT;
    pub fn Extract(
        &self,
        indices: *const u32,
        num_items: u32,
        test_mode: i32,
        extract_callback: Ref<IArchiveExtractCallback>,
    ) -> HRESULT;
    pub fn GetArchiveProperty(&self, prop_id: PropId, value: OutRef<PropVariant>) -> HRESULT;
    pub fn GetNumberOfProperties(&self, num_props: OutRef<u32>) -> HRESULT;
    pub fn GetPropertyInfo(
        &self,
        index: u32,
        name: OutRef<BSTR>,
        prop_id: OutRef<u32>,
        var_type: OutRef<u16>,
    ) -> HRESULT;
    pub fn GetNumberOfArchiveProperties(&self, num_props: OutRef<u32>) -> HRESULT;
    pub fn GetArchivePropertyInfo(
        &self,
        index: u32,
        name: OutRef<BSTR>,
        prop_id: OutRef<u32>,
        var_type: OutRef<u16>,
    ) -> HRESULT;
}

/// Out-parameters stay `u32`: write `ParentType::Dir.0`, `PropDataType::Utf16Z.bits()`.
#[interface("23170F69-40C1-278A-0000-000600700000")]
pub unsafe trait IArchiveGetRawProps: IUnknown {
    pub fn GetParent(&self, index: u32, parent: OutRef<u32>, parent_type: OutRef<u32>) -> HRESULT;
    pub fn GetRawProp(
        &self,
        index: u32,
        prop_id: PropId,
        data: *mut *const c_void,
        data_size: OutRef<u32>,
        prop_type: OutRef<u32>,
    ) -> HRESULT;
    pub fn GetNumRawProps(&self, num_props: OutRef<u32>) -> HRESULT;
    pub fn GetRawPropInfo(&self, index: u32, name: OutRef<BSTR>, prop_id: OutRef<u32>) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600710000")]
pub unsafe trait IArchiveGetRootProps: IUnknown {
    pub fn GetRootProp(&self, prop_id: PropId, value: OutRef<PropVariant>) -> HRESULT;
    pub fn GetRootRawProp(
        &self,
        prop_id: PropId,
        data: *mut *const c_void,
        data_size: OutRef<u32>,
        prop_type: OutRef<u32>,
    ) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600610000")]
pub unsafe trait IArchiveOpenSeq: IUnknown {
    pub fn OpenSeq(&self, stream: Ref<ISequentialInStream>) -> HRESULT;
}

/// Host side of `IOutArchive::UpdateItems`. Contract: IArchive.h:416
#[interface("23170F69-40C1-278A-0000-000600800000")]
pub unsafe trait IArchiveUpdateCallback: IProgress {
    pub fn GetUpdateItemInfo(
        &self,
        index: u32,
        new_data: OutRef<i32>,
        new_props: OutRef<i32>,
        index_in_archive: OutRef<u32>,
    ) -> HRESULT;
    pub fn GetProperty(&self, index: u32, prop_id: PropId, value: OutRef<PropVariant>) -> HRESULT;
    pub fn GetStream(&self, index: u32, in_stream: OutRef<ISequentialInStream>) -> HRESULT;
    /// Not an HRESULT: the only defined value is 0 (item written OK).
    pub fn SetOperationResult(&self, operation_result: i32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600820000")]
pub unsafe trait IArchiveUpdateCallback2: IArchiveUpdateCallback {
    pub fn GetVolumeSize(&self, index: u32, size: OutRef<u64>) -> HRESULT;
    pub fn GetVolumeStream(
        &self,
        index: u32,
        volume_stream: OutRef<ISequentialOutStream>,
    ) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600830000")]
pub unsafe trait IArchiveUpdateCallbackFile: IUnknown {
    pub fn GetStream2(
        &self,
        index: u32,
        in_stream: OutRef<ISequentialInStream>,
        notify_op: UpdateNotifyOp,
    ) -> HRESULT;
    pub fn ReportOperation(
        &self,
        index_type: EventIndexType,
        index: u32,
        notify_op: UpdateNotifyOp,
    ) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600840000")]
pub unsafe trait IArchiveGetDiskProperty: IUnknown {
    pub fn GetDiskProperty(
        &self,
        index: u32,
        prop_id: PropId,
        value: OutRef<PropVariant>,
    ) -> HRESULT;
}

/// Implemented by handlers that can create or modify archives. Contract: IArchive.h:508
#[interface("23170F69-40C1-278A-0000-000600A00000")]
pub unsafe trait IOutArchive: IUnknown {
    pub fn UpdateItems(
        &self,
        out_stream: Ref<ISequentialOutStream>,
        num_items: u32,
        update_callback: Ref<IArchiveUpdateCallback>,
    ) -> HRESULT;
    /// Write a `FileTimeType` value, e.g. `FileTimeType::Unix.0 as u32`.
    pub fn GetFileTimeType(&self, r#type: OutRef<u32>) -> HRESULT;
}

/// Receives user options (`-m...` switches). Value types: IArchive.h:537
#[interface("23170F69-40C1-278A-0000-000600030000")]
pub unsafe trait ISetProperties: IUnknown {
    pub fn SetProperties(
        &self,
        names: *const PCWSTR,
        values: *const PropVariant,
        num_props: u32,
    ) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600040000")]
pub unsafe trait IArchiveKeepModeForNextOpen: IUnknown {
    pub fn KeepModeForNextOpen(&self) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600050000")]
pub unsafe trait IArchiveAllowTail: IUnknown {
    pub fn AllowTail(&self, allow_tail: i32) -> HRESULT;
}

/// `allowed_size` and `answer_flags` are read and written. Contract: IArchive.h:598
#[interface("23170F69-40C1-278A-0000-000600090000")]
pub unsafe trait IArchiveRequestMemoryUseCallback: IUnknown {
    pub fn RequestMemoryUse(
        &self,
        flags: RequestMemoryUseFlags,
        index_type: EventIndexType,
        index: u32,
        path: PCWSTR,
        required_size: u64,
        allowed_size: *mut u64,
        answer_flags: *mut RequestMemoryAnswerFlags,
    ) -> HRESULT;
}

// ---------------------------------------------------------------- DLL exports

// IArchive.h:704. Signatures of the format-related exports.
pub type Func_CreateObject = unsafe extern "system" fn(
    cls_id: *const GUID,
    iid: *const GUID,
    out_object: *mut *mut c_void,
) -> HRESULT;
pub type Func_IsArc = unsafe extern "system" fn(p: *const u8, size: usize) -> IsArcResult;
pub type Func_GetIsArc =
    unsafe extern "system" fn(format_index: u32, is_arc: *mut Option<Func_IsArc>) -> HRESULT;
pub type Func_GetNumberOfFormats = unsafe extern "system" fn(num_formats: *mut u32) -> HRESULT;
pub type Func_GetHandlerProperty =
    unsafe extern "system" fn(prop_id: HandlerPropId, value: OutRef<PropVariant>) -> HRESULT;
pub type Func_GetHandlerProperty2 = unsafe extern "system" fn(
    index: u32,
    prop_id: HandlerPropId,
    value: OutRef<PropVariant>,
) -> HRESULT;
pub type Func_SetCaseSensitive = unsafe extern "system" fn(case_sensitive: i32) -> HRESULT;
pub type Func_SetLargePageMode = unsafe extern "system" fn() -> HRESULT;
pub type Func_SetLargePageMode2 =
    unsafe extern "system" fn(flags: u32, page_size: usize, threshold: usize) -> HRESULT;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i_stream::ISequentialInStream_Impl;
    use windows_core::Interface;

    #[test]
    fn iids_match_7zip() {
        let ids = [
            (ISetProperties::IID, 0x03),
            (IArchiveKeepModeForNextOpen::IID, 0x04),
            (IArchiveAllowTail::IID, 0x05),
            (IArchiveRequestMemoryUseCallback::IID, 0x09),
            (IArchiveOpenCallback::IID, 0x10),
            (IArchiveExtractCallback::IID, 0x20),
            (IArchiveExtractCallbackMessage2::IID, 0x22),
            (IArchiveOpenVolumeCallback::IID, 0x30),
            (IInArchiveGetStream::IID, 0x40),
            (IArchiveOpenSetSubArchiveName::IID, 0x50),
            (IInArchive::IID, 0x60),
            (IArchiveOpenSeq::IID, 0x61),
            (IArchiveGetRawProps::IID, 0x70),
            (IArchiveGetRootProps::IID, 0x71),
            (IArchiveUpdateCallback::IID, 0x80),
            (IArchiveUpdateCallback2::IID, 0x82),
            (IArchiveUpdateCallbackFile::IID, 0x83),
            (IArchiveGetDiskProperty::IID, 0x84),
            (IOutArchive::IID, 0xA0),
        ];
        for (iid, sub) in ids {
            assert_eq!(
                (iid.data1, iid.data2, iid.data3),
                (0x23170F69, 0x40C1, 0x278A)
            );
            assert_eq!(iid.data4, [0, 0, 0, 0x06, 0, sub, 0, 0]);
        }
    }

    #[windows_core::implement(ISequentialInStream)]
    struct Bytes(&'static [u8]);

    impl ISequentialInStream_Impl for Bytes_Impl {
        unsafe fn Read(&self, data: *mut c_void, size: u32, processed: OutRef<u32>) -> HRESULT {
            let n = (size as usize).min(self.0.len());
            unsafe { core::ptr::copy_nonoverlapping(self.0.as_ptr(), data.cast(), n) };
            let _ = processed.write(n as u32);
            HRESULT(0)
        }
    }

    #[windows_core::implement(IInArchiveGetStream)]
    struct Getter;

    impl IInArchiveGetStream_Impl for Getter_Impl {
        unsafe fn GetStream(&self, _index: u32, stream: OutRef<ISequentialInStream>) -> HRESULT {
            match stream.write(Some(Bytes(b"hi").into())) {
                Ok(()) => HRESULT(0),
                Err(e) => e.code(),
            }
        }
    }

    /// Implements an interface in Rust, then calls it through the COM vtable as a client.
    #[test]
    fn out_ref_round_trip() {
        let getter: IInArchiveGetStream = Getter.into();
        let mut stream: Option<ISequentialInStream> = None;
        unsafe { getter.GetStream(0, &mut stream) }.unwrap();
        let mut buf = [0u8; 4];
        let mut n = 0;
        unsafe { stream.unwrap().Read(buf.as_mut_ptr().cast(), 4, &mut n) }.unwrap();
        assert_eq!(&buf[..n as usize], b"hi");
    }

    #[windows_core::implement(IArchiveExtractCallbackMessage2)]
    struct Recorder(std::rc::Rc<core::cell::Cell<(u32, i32)>>);

    impl IArchiveExtractCallbackMessage2_Impl for Recorder_Impl {
        unsafe fn ReportExtractResult(
            &self,
            index_type: EventIndexType,
            _index: u32,
            op_res: OperationResult,
        ) -> HRESULT {
            self.0.set((index_type.0, op_res.0));
            HRESULT(0)
        }
    }

    /// Open enums pass through the vtable unchanged, including values with no name.
    #[test]
    fn open_enum_params_round_trip() {
        let seen = std::rc::Rc::new(core::cell::Cell::new((0, 0)));
        let cb: IArchiveExtractCallbackMessage2 = Recorder(seen.clone()).into();
        unsafe { cb.ReportExtractResult(EventIndexType::InArcIndex, 0, OperationResult(77)) }
            .unwrap();
        assert_eq!(seen.get(), (1, 77));
    }

    #[test]
    fn enum_values() {
        assert_eq!(HandlerPropId::TimeFlags.0, 12);
        assert_eq!(OperationResult::WrongPassword.0, 9);
        assert_eq!(UpdateNotifyOp::InFileChanged.0, 9);
        assert_eq!(FileTimeType::NotDefined.0, -1);
        assert_eq!(PropDataType::Utf16Z.bits(), 0x51);
        assert_eq!(PropDataType::Utf8Z.bits(), 0x50);
    }

    #[test]
    fn open_enums_accept_unknown_values() {
        let future = AskMode(42);
        let name = match future {
            AskMode::Extract | AskMode::Test | AskMode::Skip | AskMode::ReadExternal => "known",
            AskMode(_) => "unknown",
        };
        assert_eq!(name, "unknown");
        assert_eq!(format!("{:?}", AskMode::Test), "Test");
    }

    #[test]
    fn ffi_layouts() {
        assert_eq!(size_of::<AskMode>(), 4);
        assert_eq!(size_of::<PropId>(), 4);
        assert_eq!(size_of::<RequestMemoryAnswerFlags>(), 4);
        assert_eq!(size_of::<ArcTimeFlags>(), 4);
    }

    #[test]
    fn arc_time_flags() {
        let f = ArcTimeFlags::new()
            .with(FileTimeType::Unix.0 as u32)
            .with(16 + 9)
            .with_default(FileTimeType::Unix.0 as u32);
        assert!(f.supports(1) && f.supports(25) && !f.supports(0));
        assert_eq!(f.default_prec(), 1);
        assert_eq!(f.0, (1 << 1) | (1 << 25) | (1 << 27));
    }
}
