//! [`WadArchive`]: an opened WAD, as 7-Zip lists and extracts it.

use crate::decode::Stored;
use crate::names::Names;
use crate::toc::{Entry, Header, Kind};
use lol7z_api::{
    Archive, InStream, ItemStream, OperationResult, Prop, PropId, PropInfo, RawProp, Result,
};
use std::io::Write;

pub struct WadArchive {
    pub(crate) stream: InStream,
    pub(crate) header: Header,
    pub(crate) items: Vec<Item>,
    names: Names,
    phy_size: u64,
}

pub(crate) struct Item {
    pub entry: Entry,
}

impl WadArchive {
    pub(crate) fn new(
        stream: InStream,
        header: Header,
        items: Vec<Item>,
        names: Names,
        phy_size: u64,
    ) -> Self {
        Self {
            stream,
            header,
            items,
            names,
            phy_size,
        }
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn names(&self) -> &Names {
        &self.names
    }

    pub fn entries(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.items.iter().map(|i| &i.entry)
    }

    pub(crate) fn stored(&self, index: u32) -> Stored<'_> {
        Stored::new(&self.stream, &self.items[index as usize].entry)
    }
}

impl Archive for WadArchive {
    const ITEM_PROPS: &'static [PropInfo] = &[
        PropInfo::new(PropId::Path),
        PropInfo::new(PropId::Size),
        PropInfo::new(PropId::PackSize),
        PropInfo::new(PropId::Method),
        PropInfo::new(PropId::Offset),
        PropInfo::new(PropId::Comment),
    ];
    const ARCHIVE_PROPS: &'static [PropInfo] = &[
        PropInfo::new(PropId::PhySize),
        PropInfo::new(PropId::UnpackVer),
        PropInfo::new(PropId::Comment),
    ];

    fn item_count(&self) -> u32 {
        self.items.len() as u32
    }

    fn item_prop(&self, index: u32, id: PropId) -> Option<Prop<'_>> {
        let e = &self.items[index as usize].entry;
        Some(match id {
            PropId::Path => self.names.string(index).into(),
            PropId::Size => u64::from(e.size_decompressed).into(),
            PropId::PackSize => u64::from(e.size).into(),
            PropId::Method => e.kind.name().into(),
            PropId::Offset => u64::from(e.offset).into(),
            // A link's target, as the only thing a link holds.
            PropId::Comment if e.kind == Kind::Link => {
                self.stored(index).link_target().ok()?.into()
            }
            _ => return None,
        })
    }

    fn archive_prop(&self, id: PropId) -> Option<Prop<'_>> {
        Some(match id {
            PropId::PhySize => self.phy_size.into(),
            PropId::UnpackVer => format!("{}.{}", self.header.major, self.header.minor).into(),
            // Which tables named the items.
            PropId::Comment => self.names.comment().into(),
            _ => return None,
        })
    }

    fn extract_item(&self, index: u32, out: &mut dyn Write) -> Result<OperationResult> {
        self.stored(index).decode(out)
    }

    /// Stored entries only, as 7-Zip's own handlers do; the rest go through `Extract`.
    fn item_stream(&self, index: u32) -> Result<Option<ItemStream>> {
        let e = &self.items[index as usize].entry;
        Ok((e.kind == Kind::Raw).then(|| self.stream.slice(e.offset.into(), e.size.into()).into()))
    }

    fn raw_prop(&self, index: u32, id: PropId) -> Option<RawProp<'_>> {
        match id {
            PropId::Path => Some(RawProp::Utf16Z(self.names.utf16z(index))),
            _ => None,
        }
    }
}
