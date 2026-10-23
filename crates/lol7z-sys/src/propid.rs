use bitflags::bitflags;
use open_enum::open_enum;

#[open_enum]
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialOrd, Ord, Hash)]
pub enum PropId {
    NoProperty = 0,
    MainSubfile,
    HandlerItemIndex,
    Path,
    Name,
    Extension,
    IsDir,
    Size,
    PackSize,
    Attrib,
    CTime,
    ATime,
    MTime,
    Solid,
    Commented,
    Encrypted,
    SplitBefore,
    SplitAfter,
    DictionarySize,
    CRC,
    Type,
    IsAnti,
    Method,
    HostOS,
    FileSystem,
    User,
    Group,
    Block,
    Comment,
    Position,
    Prefix,
    NumSubDirs,
    NumSubFiles,
    UnpackVer,
    Volume,
    IsVolume,
    Offset,
    Links,
    NumBlocks,
    NumVolumes,
    TimeType,
    Bit64,
    BigEndian,
    Cpu,
    PhySize,
    HeadersSize,
    Checksum,
    Characts,
    Va,
    Id,
    ShortName,
    CreatorApp,
    SectorSize,
    PosixAttrib,
    SymLink,
    Error,
    TotalSize,
    FreeSpace,
    ClusterSize,
    VolumeName,
    LocalName,
    Provider,
    NtSecure,
    IsAltStream,
    IsAux,
    IsDeleted,
    IsTree,
    Sha1,
    Sha256,
    ErrorType,
    NumErrors,
    ErrorFlags,
    WarningFlags,
    Warning,
    NumStreams,
    NumAltStreams,
    AltStreamsSize,
    VirtualSize,
    UnpackSize,
    TotalPhySize,
    VolumeIndex,
    SubType,
    ShortComment,
    CodePage,
    IsNotArcType,
    PhySizeCantBeDetected,
    ZerosTailIsAllowed,
    TailSize,
    EmbeddedStubSize,
    NtReparse,
    HardLink,
    INode,
    StreamId,
    ReadOnly,
    OutName,
    CopyLink,
    ArcFileName,
    IsHash,
    ChangeTime,
    UserId,
    GroupId,
    DeviceMajor,
    DeviceMinor,
    DevMajor,
    DevMinor,
    UserDefined = 0x10000,
}

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ErrorFlags: u32 {
        const IsNotArc              = 1 << 0;
        const HeadersError          = 1 << 1;
        const EncryptedHeadersError = 1 << 2;
        const UnavailableStart      = 1 << 3;
        const UnconfirmedStart      = 1 << 4;
        const UnexpectedEnd         = 1 << 5;
        const DataAfterEnd          = 1 << 6;
        const UnsupportedMethod     = 1 << 7;
        const UnsupportedFeature    = 1 << 8;
        const DataError             = 1 << 9;
        const CrcError              = 1 << 10;
        const Unsupported           = 1 << 11;
    }
}
