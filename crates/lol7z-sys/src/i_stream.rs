//! Stream interfaces (7-Zip `CPP/7zip/IStream.h`). IID group 3.

use crate::propid::PropId;
use crate::propvar::PropVariant;
use core::ffi::c_void;
use windows::Win32::Foundation::FILETIME;
use windows_core::{HRESULT, IUnknown, IUnknown_Vtbl, OutRef, interface};

pub use windows::Win32::System::Com::{
    STREAM_SEEK, STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET,
};

/// IStream.h:120
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct CStreamFileProps {
    pub Size: u64,
    pub VolID: u64,
    pub FileID_Low: u64,
    pub FileID_High: u64,
    pub NumLinks: u32,
    pub Attrib: u32,
    pub CTime: FILETIME,
    pub ATime: FILETIME,
    pub MTime: FILETIME,
}

/// Forward-only reader. A call may return fewer bytes than asked; 0 bytes means end of data.
#[interface("23170F69-40C1-278A-0000-000300010000")]
pub unsafe trait ISequentialInStream: IUnknown {
    pub fn Read(&self, data: *mut c_void, size: u32, processed: OutRef<u32>) -> HRESULT;
}

/// Forward-only writer. A call may accept fewer bytes than given.
#[interface("23170F69-40C1-278A-0000-000300020000")]
pub unsafe trait ISequentialOutStream: IUnknown {
    pub fn Write(&self, data: *const c_void, size: u32, processed: OutRef<u32>) -> HRESULT;
}

/// Seekable reader.
#[interface("23170F69-40C1-278A-0000-000300030000")]
pub unsafe trait IInStream: ISequentialInStream {
    pub fn Seek(&self, offset: i64, seek_origin: STREAM_SEEK, new_position: OutRef<u64>)
    -> HRESULT;
}

/// Seekable, resizable writer.
#[interface("23170F69-40C1-278A-0000-000300040000")]
pub unsafe trait IOutStream: ISequentialOutStream {
    pub fn Seek(&self, offset: i64, seek_origin: STREAM_SEEK, new_position: OutRef<u64>)
    -> HRESULT;
    pub fn SetSize(&self, new_size: u64) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000300060000")]
pub unsafe trait IStreamGetSize: IUnknown {
    pub fn GetSize(&self, size: OutRef<u64>) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000300070000")]
pub unsafe trait IOutStreamFinish: IUnknown {
    pub fn OutStreamFinish(&self) -> HRESULT;
}

/// Any argument may be NULL. `FILETIME` has no `OutRef` support in `windows` 0.62, so those stay raw.
#[interface("23170F69-40C1-278A-0000-000300080000")]
pub unsafe trait IStreamGetProps: IUnknown {
    pub fn GetProps(
        &self,
        size: OutRef<u64>,
        c_time: *mut FILETIME,
        a_time: *mut FILETIME,
        m_time: *mut FILETIME,
        attrib: OutRef<u32>,
    ) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000300090000")]
pub unsafe trait IStreamGetProps2: IUnknown {
    pub fn GetProps2(&self, props: *mut CStreamFileProps) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-0003000A0000")]
pub unsafe trait IStreamGetProp: IUnknown {
    pub fn GetProperty(&self, prop_id: PropId, value: OutRef<PropVariant>) -> HRESULT;
    pub fn ReloadProps(&self) -> HRESULT;
}

/// Semantics: IStream.h:144
#[interface("23170F69-40C1-278A-0000-000300100000")]
pub unsafe trait IStreamSetRestriction: IUnknown {
    pub fn SetRestriction(&self, begin: u64, end: u64) -> HRESULT;
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_core::Interface;

    #[test]
    fn iids_match_7zip() {
        let ids = [
            (ISequentialInStream::IID, 0x01),
            (ISequentialOutStream::IID, 0x02),
            (IInStream::IID, 0x03),
            (IOutStream::IID, 0x04),
            (IStreamGetSize::IID, 0x06),
            (IOutStreamFinish::IID, 0x07),
            (IStreamGetProps::IID, 0x08),
            (IStreamGetProps2::IID, 0x09),
            (IStreamGetProp::IID, 0x0A),
            (IStreamSetRestriction::IID, 0x10),
        ];
        for (iid, sub) in ids {
            assert_eq!(
                (iid.data1, iid.data2, iid.data3),
                (0x23170F69, 0x40C1, 0x278A)
            );
            assert_eq!(iid.data4, [0, 0, 0, 0x03, 0, sub, 0, 0]);
        }
    }

    #[test]
    fn layouts() {
        assert_eq!(size_of::<STREAM_SEEK>(), 4);
        assert_eq!(size_of::<CStreamFileProps>(), 4 * 8 + 2 * 4 + 3 * 8);
    }
}
