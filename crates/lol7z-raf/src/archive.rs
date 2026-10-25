//! [`RafArchive`]: an opened archive, as 7-Zip lists and extracts it.

use crate::decode;
use crate::toc::{Entry, Toc};
use lol7z_api::{
    Archive, Error, InStream, ItemStream, OperationResult, Prop, PropId, PropInfo, RawProp, Result,
};
use std::io::Write;

/// How an entry's data is stored, told from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Store,
    Zlib,
    /// The data file isn't there to look at.
    Unknown,
}

impl Method {
    pub fn name(self) -> Option<&'static str> {
        match self {
            Method::Store => Some("Store"),
            Method::Zlib => Some("zlib"),
            Method::Unknown => None,
        }
    }
}

/// Items are the index's entries, in its order.
pub struct RafArchive {
    /// `None` when the data file is missing: items list but don't extract.
    data: Option<InStream>,
    data_name: String,
    data_size: u64,
    toc: Toc,
    methods: Vec<Method>,
    /// Each item's name, 0-terminated, end to end.
    names: Vec<u16>,
    spans: Vec<[u32; 2]>,
    phy_size: u64,
}

impl RafArchive {
    pub(crate) fn build(
        data: Option<InStream>,
        data_name: String,
        data_size: u64,
        toc: Toc,
        methods: Vec<Method>,
        phy_size: u64,
    ) -> Result<Self> {
        let count = toc.entries.len();
        let mut spans = Vec::new();
        spans.try_reserve_exact(count)?;
        let mut names: Vec<u16> = Vec::new();
        let bytes: usize = toc.entries.iter().map(|e| e.path.len() + 1).sum();
        names.try_reserve_exact(bytes)?;
        for e in &toc.entries {
            let start = names.len();
            let path = e.path.trim_start_matches(['/', '\\']);
            names.try_reserve(path.len() + 1)?;
            names.extend(path.encode_utf16().map(|u| {
                if u == u16::from(b'\\') {
                    u16::from(b'/')
                } else {
                    u
                }
            }));
            names.push(0);
            let span =
                [start, names.len()].map(|n| u32::try_from(n).map_err(|_| Error::OUT_OF_MEMORY));
            spans.push([span[0]?, span[1]?]);
        }
        Ok(Self {
            data,
            data_name,
            data_size,
            toc,
            methods,
            names,
            spans,
            phy_size,
        })
    }

    pub fn toc(&self) -> &Toc {
        &self.toc
    }

    pub fn methods(&self) -> &[Method] {
        &self.methods
    }

    /// The data file's name, present or not.
    pub fn data_name(&self) -> &str {
        &self.data_name
    }

    fn entry(&self, index: u32) -> &Entry {
        &self.toc.entries[index as usize]
    }

    fn method(&self, index: u32) -> Method {
        self.methods[index as usize]
    }

    fn utf16z(&self, index: u32) -> &[u16] {
        self.spans
            .get(index as usize)
            .and_then(|&[a, b]| self.names.get(a as usize..b as usize))
            .unwrap_or(&[])
    }

    fn comment(&self) -> String {
        let mut s = format!("data in {}", self.data_name);
        if self.toc.manager_index != 0 {
            s.push_str(&format!("; manager index {}", self.toc.manager_index));
        }
        s
    }
}

impl Archive for RafArchive {
    const ITEM_PROPS: &'static [PropInfo] = &[
        PropInfo::new(PropId::Path),
        PropInfo::new(PropId::Size),
        PropInfo::new(PropId::PackSize),
        PropInfo::new(PropId::Method),
        PropInfo::new(PropId::Offset),
    ];
    const ARCHIVE_PROPS: &'static [PropInfo] = &[
        PropInfo::new(PropId::PhySize),
        PropInfo::new(PropId::TotalPhySize),
        PropInfo::new(PropId::NumVolumes),
        PropInfo::new(PropId::UnpackVer),
        PropInfo::new(PropId::Comment),
    ];

    fn item_count(&self) -> u32 {
        self.toc.entries.len() as u32
    }

    fn item_prop(&self, index: u32, id: PropId) -> Option<Prop<'_>> {
        let e = self.entry(index);
        Some(match id {
            PropId::Path => {
                let s = self.utf16z(index);
                String::from_utf16_lossy(s.strip_suffix(&[0]).unwrap_or(s)).into()
            }
            PropId::IsDir => false.into(),
            // zlib doesn't store the unpacked size; only a stored entry knows its own.
            PropId::Size if self.method(index) == Method::Store => u64::from(e.length).into(),
            PropId::PackSize => u64::from(e.length).into(),
            PropId::Method => self.method(index).name()?.into(),
            PropId::Offset => u64::from(e.offset).into(),
            _ => return None,
        })
    }

    fn archive_prop(&self, id: PropId) -> Option<Prop<'_>> {
        Some(match id {
            PropId::PhySize => self.phy_size.into(),
            PropId::TotalPhySize => self.phy_size.saturating_add(self.data_size).into(),
            PropId::NumVolumes => if self.data.is_some() { 2u32 } else { 1 }.into(),
            PropId::UnpackVer => self.toc.version.to_string().into(),
            PropId::Comment => self.comment().into(),
            _ => return None,
        })
    }

    fn extract_item(&self, index: u32, out: &mut dyn Write) -> Result<OperationResult> {
        let Some(data) = &self.data else {
            return Ok(OperationResult::Unavailable);
        };
        decode::extract(data, self.entry(index), self.method(index), out)
    }

    /// Stored entries only, as 7-Zip's own handlers do; the rest go through `Extract`.
    fn item_stream(&self, index: u32) -> Result<Option<ItemStream>> {
        let Some(data) = &self.data else {
            return Ok(None);
        };
        let e = self.entry(index);
        Ok((self.method(index) == Method::Store)
            .then(|| data.slice(e.offset.into(), e.length.into()).into()))
    }

    fn raw_prop(&self, index: u32, id: PropId) -> Option<RawProp<'_>> {
        match id {
            PropId::Path => Some(RawProp::Utf16Z(self.utf16z(index))),
            _ => None,
        }
    }
}
