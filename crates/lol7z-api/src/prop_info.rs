//! [`PropInfo`]: the properties an archive declares, and the types 7-Zip expects.

use crate::error::Result;
use lol7z_sys::propid::PropId;
use windows::Win32::System::Variant::{
    VARENUM, VT_BOOL, VT_BSTR, VT_EMPTY, VT_FILETIME, VT_UI4, VT_UI8,
};
use windows_core::{BSTR, OutRef};

/// One entry of [`Archive::ITEM_PROPS`](crate::Archive::ITEM_PROPS) or
/// [`Archive::ARCHIVE_PROPS`](crate::Archive::ARCHIVE_PROPS): the columns 7-Zip shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropInfo {
    pub id: PropId,
    pub vt: VARENUM,
    /// `None` for standard properties; 7-Zip has its own names for those.
    pub name: Option<&'static str>,
}

impl PropInfo {
    /// A standard property, with the type 7-Zip expects for it.
    pub const fn new(id: PropId) -> Self {
        Self {
            id,
            vt: standard_vt(id),
            name: None,
        }
    }

    /// A property of your own: `id` is `PropId::UserDefined` or above, shown as `name`.
    pub const fn custom(id: PropId, name: &'static str, vt: VARENUM) -> Self {
        Self {
            id,
            vt,
            name: Some(name),
        }
    }

    /// The `GetPropertyInfo` out-parameters. A null `name` means a standard property.
    pub(crate) fn write_to(
        &self,
        name: OutRef<'_, BSTR>,
        prop_id: OutRef<'_, u32>,
        var_type: OutRef<'_, u16>,
    ) -> Result<()> {
        name.write(self.name.map_or_else(BSTR::new, BSTR::from))?;
        prop_id.write(self.id.0)?;
        Ok(var_type.write(self.vt.0)?)
    }
}

/// The type 7-Zip expects for a standard property (`k7z_PROPID_To_VARTYPE`, PropId.cpp).
/// `VT_EMPTY` for ids it doesn't define.
pub const fn standard_vt(id: PropId) -> VARENUM {
    use PropId as P;
    match id {
        P::MainSubfile
        | P::HandlerItemIndex
        | P::Attrib
        | P::DictionarySize
        | P::CRC
        | P::Volume
        | P::TimeType
        | P::Checksum
        | P::SectorSize
        | P::PosixAttrib
        | P::ClusterSize
        | P::ErrorFlags
        | P::WarningFlags
        | P::NumAltStreams
        | P::UserId
        | P::GroupId
        | P::DeviceMajor
        | P::DeviceMinor
        | P::DevMajor
        | P::DevMinor => VT_UI4,

        P::Size
        | P::PackSize
        | P::Block
        | P::Position
        | P::NumSubDirs
        | P::NumSubFiles
        | P::Offset
        | P::Links
        | P::NumBlocks
        | P::NumVolumes
        | P::PhySize
        | P::HeadersSize
        | P::Va
        | P::TotalSize
        | P::FreeSpace
        | P::NumErrors
        | P::NumStreams
        | P::AltStreamsSize
        | P::VirtualSize
        | P::UnpackSize
        | P::TotalPhySize
        | P::VolumeIndex
        | P::TailSize
        | P::EmbeddedStubSize
        | P::INode
        | P::StreamId => VT_UI8,

        P::Path
        | P::Name
        | P::Extension
        | P::Type
        | P::Method
        | P::HostOS
        | P::FileSystem
        | P::User
        | P::Group
        | P::Comment
        | P::Prefix
        | P::UnpackVer
        | P::Cpu
        | P::Characts
        | P::Id
        | P::ShortName
        | P::CreatorApp
        | P::SymLink
        | P::Error
        | P::VolumeName
        | P::LocalName
        | P::Provider
        | P::NtSecure
        | P::Sha1
        | P::Sha256
        | P::ErrorType
        | P::Warning
        | P::SubType
        | P::ShortComment
        | P::CodePage
        | P::NtReparse
        | P::HardLink
        | P::OutName
        | P::CopyLink
        | P::ArcFileName => VT_BSTR,

        P::IsDir
        | P::Solid
        | P::Commented
        | P::Encrypted
        | P::SplitBefore
        | P::SplitAfter
        | P::IsAnti
        | P::IsVolume
        | P::Bit64
        | P::BigEndian
        | P::IsAltStream
        | P::IsAux
        | P::IsDeleted
        | P::IsTree
        | P::IsNotArcType
        | P::PhySizeCantBeDetected
        | P::ZerosTailIsAllowed
        | P::ReadOnly
        | P::IsHash => VT_BOOL,

        P::CTime | P::ATime | P::MTime | P::ChangeTime => VT_FILETIME,

        _ => VT_EMPTY,
    }
}
