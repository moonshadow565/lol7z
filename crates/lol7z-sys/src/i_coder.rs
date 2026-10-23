//! Coder interfaces (7-Zip `CPP/7zip/ICoder.h`). IID group 4.

use crate::i_stream::{ISequentialInStream, ISequentialOutStream};
use crate::propvar::PropVariant;
use core::ffi::c_void;
use open_enum::open_enum;
use windows_core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, OutRef, Ref, interface};

// ---------------------------------------------------------------- enums

/// ICoder.h:104
#[open_enum]
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialOrd, Ord, Hash)]
pub enum CoderPropId {
    DefaultProp = 0,
    DictionarySize,
    UsedMemorySize,
    Order,
    BlockSize,
    PosStateBits,
    LitContextBits,
    LitPosBits,
    NumFastBytes,
    MatchFinder,
    MatchFinderCycles,
    NumPasses,
    Algorithm,
    NumThreads,
    EndMarker,
    Level,
    ReduceSize,
    ExpectedDataSize,
    BlockSize2,
    CheckSize,
    Filter,
    MemUse,
    Affinity,
    BranchOffset,
    HashBits,
    NumThreadGroups,
    ThreadGroup,
    AffinityInGroup,
}

/// ICoder.h:405. Property ids for `ICompressCodecsInfo::GetProperty` and `IHashers::GetHasherProp`.
#[open_enum]
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialOrd, Ord, Hash)]
pub enum MethodPropId {
    ID = 0,
    Name,
    Decoder,
    Encoder,
    PackStreams,
    UnpackStreams,
    Description,
    DecoderIsAssigned,
    EncoderIsAssigned,
    DigestSize,
    IsFilter,
}

/// ICoder.h:423. Value for `ModulePropId::InterfaceType`.
#[open_enum]
#[repr(u32)]
#[derive(Debug, Clone, Copy, Hash)]
pub enum ModuleInterfaceType {
    IUnknown_VirtDestructor_No = 0,
    IUnknown_VirtDestructor_Yes = 1,
}

/// ICoder.h:443. Property ids for the `GetModuleProp` export.
#[open_enum]
#[repr(u32)]
#[derive(Debug, Clone, Copy, Hash)]
pub enum ModulePropId {
    InterfaceType = 0,
    Version,
}

// ---------------------------------------------------------------- interfaces

#[interface("23170F69-40C1-278A-0000-000400040000")]
pub unsafe trait ICompressProgressInfo: IUnknown {
    pub fn SetRatioInfo(&self, in_size: Option<&u64>, out_size: Option<&u64>) -> HRESULT;
}

/// Contract: ICoder.h:39
#[interface("23170F69-40C1-278A-0000-000400050000")]
pub unsafe trait ICompressCoder: IUnknown {
    pub fn Code(
        &self,
        in_stream: Ref<ISequentialInStream>,
        out_stream: Ref<ISequentialOutStream>,
        in_size: Option<&u64>,
        out_size: Option<&u64>,
        progress: Ref<ICompressProgressInfo>,
    ) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400180000")]
pub unsafe trait ICompressCoder2: IUnknown {
    pub fn Code(
        &self,
        in_streams: *const Option<ISequentialInStream>,
        in_sizes: *const *const u64,
        num_in_streams: u32,
        out_streams: *const Option<ISequentialOutStream>,
        out_sizes: *const *const u64,
        num_out_streams: u32,
        progress: Ref<ICompressProgressInfo>,
    ) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-0004001F0000")]
pub unsafe trait ICompressSetCoderPropertiesOpt: IUnknown {
    pub fn SetCoderPropertiesOpt(
        &self,
        prop_ids: *const CoderPropId,
        props: *const PropVariant,
        num_props: u32,
    ) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400200000")]
pub unsafe trait ICompressSetCoderProperties: IUnknown {
    pub fn SetCoderProperties(
        &self,
        prop_ids: *const CoderPropId,
        props: *const PropVariant,
        num_props: u32,
    ) -> HRESULT;
}

/// Set before `Code` for methods that carry property bytes (LZMA, LZMA2, PPMd, ...).
#[interface("23170F69-40C1-278A-0000-000400220000")]
pub unsafe trait ICompressSetDecoderProperties2: IUnknown {
    pub fn SetDecoderProperties2(&self, data: *const u8, size: u32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400230000")]
pub unsafe trait ICompressWriteCoderProperties: IUnknown {
    pub fn WriteCoderProperties(&self, out_stream: Ref<ISequentialOutStream>) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400240000")]
pub unsafe trait ICompressGetInStreamProcessedSize: IUnknown {
    pub fn GetInStreamProcessedSize(&self, value: OutRef<u64>) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400250000")]
pub unsafe trait ICompressSetCoderMt: IUnknown {
    pub fn SetNumberOfThreads(&self, num_threads: u32) -> HRESULT;
}

/// `finish_mode`: 0 or 1. Semantics: ICoder.h:210
#[interface("23170F69-40C1-278A-0000-000400260000")]
pub unsafe trait ICompressSetFinishMode: IUnknown {
    pub fn SetFinishMode(&self, finish_mode: u32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400270000")]
pub unsafe trait ICompressGetInStreamProcessedSize2: IUnknown {
    pub fn GetInStreamProcessedSize2(&self, stream_index: u32, value: OutRef<u64>) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400280000")]
pub unsafe trait ICompressSetMemLimit: IUnknown {
    pub fn SetMemLimit(&self, mem_usage: u64) -> HRESULT;
}

/// Semantics: ICoder.h:226
#[interface("23170F69-40C1-278A-0000-000400290000")]
pub unsafe trait ICompressReadUnusedFromInBuf: IUnknown {
    pub fn ReadUnusedFromInBuf(
        &self,
        data: *mut c_void,
        size: u32,
        processed_size: OutRef<u32>,
    ) -> HRESULT;
}

/// Semantics: ICoder.h:240
#[interface("23170F69-40C1-278A-0000-000400300000")]
pub unsafe trait ICompressGetSubStreamSize: IUnknown {
    pub fn GetSubStreamSize(&self, sub_stream: u64, value: OutRef<u64>) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400310000")]
pub unsafe trait ICompressSetInStream: IUnknown {
    pub fn SetInStream(&self, in_stream: Ref<ISequentialInStream>) -> HRESULT;
    pub fn ReleaseInStream(&self) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400320000")]
pub unsafe trait ICompressSetOutStream: IUnknown {
    pub fn SetOutStream(&self, out_stream: Ref<ISequentialOutStream>) -> HRESULT;
    pub fn ReleaseOutStream(&self) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400340000")]
pub unsafe trait ICompressSetOutStreamSize: IUnknown {
    pub fn SetOutStreamSize(&self, out_size: Option<&u64>) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400350000")]
pub unsafe trait ICompressSetBufSize: IUnknown {
    pub fn SetInBufSize(&self, stream_index: u32, size: u32) -> HRESULT;
    pub fn SetOutBufSize(&self, stream_index: u32, size: u32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400360000")]
pub unsafe trait ICompressInitEncoder: IUnknown {
    pub fn InitEncoder(&self) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400370000")]
pub unsafe trait ICompressSetInStream2: IUnknown {
    pub fn SetInStream2(&self, stream_index: u32, in_stream: Ref<ISequentialInStream>) -> HRESULT;
    pub fn ReleaseInStream2(&self, stream_index: u32) -> HRESULT;
}

/// In-place filter. `Filter` returns a byte count, not an HRESULT. Semantics: ICoder.h:319
#[interface("23170F69-40C1-278A-0000-000400400000")]
pub unsafe trait ICompressFilter: IUnknown {
    pub fn Init(&self) -> HRESULT;
    pub fn Filter(&self, data: *mut u8, size: u32) -> u32;
}

/// The host's codec list, handed to the `SetCodecs` export.
#[interface("23170F69-40C1-278A-0000-000400600000")]
pub unsafe trait ICompressCodecsInfo: IUnknown {
    pub fn GetNumMethods(&self, num_methods: OutRef<u32>) -> HRESULT;
    pub fn GetProperty(
        &self,
        index: u32,
        prop_id: MethodPropId,
        value: OutRef<PropVariant>,
    ) -> HRESULT;
    pub fn CreateDecoder(&self, index: u32, iid: &GUID, coder: *mut *mut c_void) -> HRESULT;
    pub fn CreateEncoder(&self, index: u32, iid: &GUID, coder: *mut *mut c_void) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400610000")]
pub unsafe trait ISetCompressCodecsInfo: IUnknown {
    pub fn SetCompressCodecsInfo(&self, compress_codecs_info: Ref<ICompressCodecsInfo>) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400800000")]
pub unsafe trait ICryptoProperties: IUnknown {
    pub fn SetKey(&self, data: *const u8, size: u32) -> HRESULT;
    pub fn SetInitVector(&self, data: *const u8, size: u32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-0004008C0000")]
pub unsafe trait ICryptoResetInitVector: IUnknown {
    pub fn ResetInitVector(&self) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400900000")]
pub unsafe trait ICryptoSetPassword: IUnknown {
    pub fn CryptoSetPassword(&self, data: *const u8, size: u32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000400A00000")]
pub unsafe trait ICryptoSetCRC: IUnknown {
    pub fn CryptoSetCRC(&self, crc: u32) -> HRESULT;
}

/// Methods return plain values, not HRESULTs.
#[interface("23170F69-40C1-278A-0000-000400C00000")]
pub unsafe trait IHasher: IUnknown {
    pub fn Init(&self);
    pub fn Update(&self, data: *const c_void, size: u32);
    pub fn Final(&self, digest: *mut u8);
    pub fn GetDigestSize(&self) -> u32;
}

#[interface("23170F69-40C1-278A-0000-000400C10000")]
pub unsafe trait IHashers: IUnknown {
    pub fn GetNumHashers(&self) -> u32;
    pub fn GetHasherProp(
        &self,
        index: u32,
        prop_id: MethodPropId,
        value: OutRef<PropVariant>,
    ) -> HRESULT;
    pub fn CreateHasher(&self, index: u32, hasher: OutRef<IHasher>) -> HRESULT;
}

// ---------------------------------------------------------------- DLL exports

// ICoder.h:466. Signatures of the codec-related exports.
pub type Func_GetNumberOfMethods = unsafe extern "system" fn(num_methods: *mut u32) -> HRESULT;
pub type Func_GetMethodProperty = unsafe extern "system" fn(
    index: u32,
    prop_id: MethodPropId,
    value: OutRef<PropVariant>,
) -> HRESULT;
pub type Func_CreateDecoder = unsafe extern "system" fn(
    index: u32,
    iid: *const GUID,
    out_object: *mut *mut c_void,
) -> HRESULT;
pub type Func_CreateEncoder = unsafe extern "system" fn(
    index: u32,
    iid: *const GUID,
    out_object: *mut *mut c_void,
) -> HRESULT;
pub type Func_GetHashers = unsafe extern "system" fn(hashers: *mut Option<IHashers>) -> HRESULT;
pub type Func_SetCodecs =
    unsafe extern "system" fn(compress_codecs_info: Option<ICompressCodecsInfo>) -> HRESULT;
pub type Func_GetModuleProp =
    unsafe extern "system" fn(prop_id: ModulePropId, value: OutRef<PropVariant>) -> HRESULT;

#[cfg(test)]
mod tests {
    use super::*;
    use windows_core::Interface;

    #[test]
    fn iids_match_7zip() {
        let ids = [
            (ICompressProgressInfo::IID, 0x04),
            (ICompressCoder::IID, 0x05),
            (ICompressCoder2::IID, 0x18),
            (ICompressSetCoderPropertiesOpt::IID, 0x1F),
            (ICompressSetCoderProperties::IID, 0x20),
            (ICompressSetDecoderProperties2::IID, 0x22),
            (ICompressWriteCoderProperties::IID, 0x23),
            (ICompressGetInStreamProcessedSize::IID, 0x24),
            (ICompressSetCoderMt::IID, 0x25),
            (ICompressSetFinishMode::IID, 0x26),
            (ICompressGetInStreamProcessedSize2::IID, 0x27),
            (ICompressSetMemLimit::IID, 0x28),
            (ICompressReadUnusedFromInBuf::IID, 0x29),
            (ICompressGetSubStreamSize::IID, 0x30),
            (ICompressSetInStream::IID, 0x31),
            (ICompressSetOutStream::IID, 0x32),
            (ICompressSetOutStreamSize::IID, 0x34),
            (ICompressSetBufSize::IID, 0x35),
            (ICompressInitEncoder::IID, 0x36),
            (ICompressSetInStream2::IID, 0x37),
            (ICompressFilter::IID, 0x40),
            (ICompressCodecsInfo::IID, 0x60),
            (ISetCompressCodecsInfo::IID, 0x61),
            (ICryptoProperties::IID, 0x80),
            (ICryptoResetInitVector::IID, 0x8C),
            (ICryptoSetPassword::IID, 0x90),
            (ICryptoSetCRC::IID, 0xA0),
            (IHasher::IID, 0xC0),
            (IHashers::IID, 0xC1),
        ];
        for (iid, sub) in ids {
            assert_eq!(
                (iid.data1, iid.data2, iid.data3),
                (0x23170F69, 0x40C1, 0x278A)
            );
            assert_eq!(iid.data4, [0, 0, 0, 0x04, 0, sub, 0, 0]);
        }
    }

    #[test]
    fn enum_values() {
        assert_eq!(CoderPropId::AffinityInGroup.0, 27);
        assert_eq!(MethodPropId::IsFilter.0, 10);
    }
}
