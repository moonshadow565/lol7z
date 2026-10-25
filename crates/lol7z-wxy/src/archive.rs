//! [`WxyArchive`]: an opened package, as 7-Zip lists and extracts it.

use crate::decode;
use crate::toc::{Entry, Method, PreviewKind, Toc};
use lol7z_api::{
    Archive, Error, InStream, ItemStream, OperationResult, Prop, PropId, PropInfo, RawProp, Result,
};
use std::io::Write;

pub const RAW: &str = "RAW";
pub const META: &str = "META";
pub const INFO: &str = "META/info.json";

/// One listed item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// A game file, by entry.
    Entry(u32),
    /// `META/info.json`, made from the package's name, author and version.
    Info,
    /// A preview, by index.
    Preview(u32),
}

/// Items are the entries, then the info file, then the previews.
pub struct WxyArchive {
    stream: InStream,
    toc: Toc,
    items: Vec<Item>,
    /// Each item's name, 0-terminated, end to end.
    names: Vec<u16>,
    spans: Vec<[u32; 2]>,
    info: Vec<u8>,
    phy_size: u64,
}

impl WxyArchive {
    pub(crate) fn build(stream: InStream, toc: Toc, phy_size: u64) -> Result<Self> {
        let info = serde_json::to_vec_pretty(&serde_json::json!({
            "Name": toc.meta.name,
            "Author": toc.meta.author,
            "Version": toc.meta.version,
            "Description": "Converted from .wxy",
        }))
        .expect("a JSON value serializes");

        let count = toc.entries.len() + 1 + toc.previews.len();
        let mut items = Vec::new();
        items.try_reserve_exact(count)?;
        let mut spans = Vec::new();
        spans.try_reserve_exact(count)?;
        let mut names: Vec<u16> = Vec::new();
        let bytes: usize = toc.entries.iter().map(|e| e.path.len() + 5).sum();
        names.try_reserve_exact(bytes + 16 + 24 * toc.previews.len())?;
        let mut push = |items: &mut Vec<Item>, item: Item, name: &str| -> Result<()> {
            let start = names.len();
            names.try_reserve(name.len() + 1)?;
            names.extend(name.encode_utf16().map(|u| {
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
            items.push(item);
            Ok(())
        };
        for (i, e) in toc.entries.iter().enumerate() {
            let path = e.path.trim_start_matches(['/', '\\']);
            push(&mut items, Item::Entry(i as u32), &format!("{RAW}/{path}"))?;
        }
        push(&mut items, Item::Info, INFO)?;
        let mut first_png = false;
        let mut n = 0;
        for (i, p) in toc.previews.iter().enumerate() {
            let ext = extension(&decode::preview_head(&stream, p), p.kind);
            let name = if !first_png && ext == "png" {
                first_png = true;
                format!("{META}/image.png")
            } else {
                n += 1;
                format!("{META}/image{}.{ext}", n - 1)
            };
            push(&mut items, Item::Preview(i as u32), &name)?;
        }
        Ok(Self {
            stream,
            toc,
            items,
            names,
            spans,
            info,
            phy_size,
        })
    }

    pub fn toc(&self) -> &Toc {
        &self.toc
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The `info.json` shown.
    pub fn info(&self) -> &[u8] {
        &self.info
    }

    fn item(&self, index: u32) -> Item {
        self.items[index as usize]
    }

    fn entry(&self, i: u32) -> &Entry {
        &self.toc.entries[i as usize]
    }

    fn utf16z(&self, index: u32) -> &[u16] {
        self.spans
            .get(index as usize)
            .and_then(|&[a, b]| self.names.get(a as usize..b as usize))
            .unwrap_or(&[])
    }

    fn comment(&self) -> String {
        let m = &self.toc.meta;
        let mut parts: Vec<String> = Vec::new();
        let title = [m.name.as_str(), m.version.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        match (title.is_empty(), m.author.is_empty()) {
            (false, false) => parts.push(format!("{title} by {}", m.author)),
            (false, true) => parts.push(title),
            (true, false) => parts.push(format!("by {}", m.author)),
            (true, true) => {}
        }
        match (m.category.is_empty(), m.sub_category.is_empty()) {
            (false, false) => parts.push(format!("{}/{}", m.category, m.sub_category)),
            (false, true) => parts.push(m.category.clone()),
            (true, false) => parts.push(m.sub_category.clone()),
            (true, true) => {}
        }
        if !self.toc.projects.is_empty() {
            parts.push(format!("projects: {}", self.toc.projects.join(", ")));
        }
        if !self.toc.deletes.is_empty() {
            parts.push(format!("deletes: {}", self.toc.deletes.len()));
        }
        parts.join("; ")
    }
}

/// The extension a preview's first bytes say, else `bin`.
fn extension(head: &[u8], kind: PreviewKind) -> &'static str {
    let magics: &[(&[u8], &str)] = &[
        (b"\x89PNG\r\n\x1a\n", "png"),
        (b"\xFF\xD8\xFF", "jpg"),
        (b"GIF8", "gif"),
        (b"BM", "bmp"),
        (b"DDS ", "dds"),
        (b"II*\0", "tif"),
    ];
    if let Some((_, ext)) = magics.iter().find(|(m, _)| head.starts_with(m)) {
        return ext;
    }
    if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        return "webp";
    }
    match kind {
        PreviewKind::Model => "model",
        _ => "bin",
    }
}

impl Archive for WxyArchive {
    const ITEM_PROPS: &'static [PropInfo] = &[
        PropInfo::new(PropId::Path),
        PropInfo::new(PropId::Size),
        PropInfo::new(PropId::PackSize),
        PropInfo::new(PropId::Method),
        PropInfo::new(PropId::Offset),
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
        let item = self.item(index);
        Some(match (item, id) {
            (_, PropId::Path) => {
                let s = self.utf16z(index);
                String::from_utf16_lossy(s.strip_suffix(&[0]).unwrap_or(s)).into()
            }
            (_, PropId::IsDir) => false.into(),
            (Item::Entry(e), PropId::Size) => u64::from(self.entry(e).size).into(),
            (Item::Entry(e), PropId::PackSize) => self.entry(e).stored(self.toc.version).into(),
            (Item::Entry(e), PropId::Method) => self.entry(e).method.name().into(),
            (Item::Entry(e), PropId::Offset) => self.entry(e).offset.into(),
            (Item::Info, PropId::Size | PropId::PackSize) => (self.info.len() as u64).into(),
            (Item::Info, PropId::Method) => "Store".into(),
            (Item::Preview(p), PropId::PackSize) => {
                u64::from(self.toc.previews[p as usize].size).into()
            }
            (Item::Preview(_), PropId::Method) => "Deflate".into(),
            (Item::Preview(p), PropId::Offset) => self.toc.previews[p as usize].offset.into(),
            _ => return None,
        })
    }

    fn archive_prop(&self, id: PropId) -> Option<Prop<'_>> {
        Some(match id {
            PropId::PhySize => self.phy_size.into(),
            PropId::UnpackVer => self.toc.version.to_string().into(),
            PropId::Comment => self.comment().into(),
            _ => return None,
        })
    }

    fn extract_item(&self, index: u32, out: &mut dyn Write) -> Result<OperationResult> {
        match self.item(index) {
            Item::Entry(e) => decode::extract(&self.stream, self.toc.version, self.entry(e), out),
            Item::Info => {
                out.write_all(&self.info)?;
                Ok(OperationResult::OK)
            }
            Item::Preview(p) => {
                decode::preview(&self.stream, &self.toc.previews[p as usize], out)?;
                Ok(OperationResult::OK)
            }
        }
    }

    /// Stored entries only, as 7-Zip's own handlers do; the rest go through `Extract`.
    fn item_stream(&self, index: u32) -> Result<Option<ItemStream>> {
        let Item::Entry(i) = self.item(index) else {
            return Ok(None);
        };
        let e = self.entry(i);
        Ok((e.method == Method::Store).then(|| self.stream.slice(e.offset, e.size.into()).into()))
    }

    fn raw_prop(&self, index: u32, id: PropId) -> Option<RawProp<'_>> {
        match id {
            PropId::Path => Some(RawProp::Utf16Z(self.utf16z(index))),
            _ => None,
        }
    }
}
