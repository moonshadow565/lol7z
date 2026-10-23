//! [`ModpkgArchive`]: an opened package, as 7-Zip lists and extracts it.

use crate::config::{self, Config};
use crate::decode;
use crate::meta::Metadata;
use crate::path::{self, GAME_DATA, INFO, layer_dir};
use crate::toc::{Compression, Header, Layer, Record, Tables};
use lol7z_api::{
    Archive, Error, InStream, ItemStream, OperationResult, Prop, PropId, PropInfo, RawProp, Result,
};
use lol7z_wad::{Names, PathHash, display};
use std::io::Write;

/// One listed item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// A chunk, by record.
    Record(u32),
    /// `mod.config.json`, made from the metadata document.
    Config,
    /// `content/<layer>/game_data.json`, by layer.
    GameData(u32),
    /// A layer's folder, by layer.
    Dir(u32),
}

/// Items are the records (less the metadata document when it reads), the config,
/// each layer's game-data file, then one folder per layer.
pub struct ModpkgArchive {
    pub(crate) stream: InStream,
    header: Header,
    signature: Vec<u8>,
    pub(crate) tables: Tables,
    pub(crate) records: Vec<Record>,
    metadata: Option<Metadata>,
    config: Option<Config>,
    config_json: Vec<u8>,
    /// By layer.
    game_data: Vec<Option<Vec<u8>>>,
    items: Vec<Item>,
    /// The file items' names, in item order.
    names: Names,
    dirs: Dirs,
    phy_size: u64,
}

/// The layer folders' names, each 0-terminated.
struct Dirs {
    buf: Vec<u16>,
    spans: Vec<[u32; 2]>,
}

impl Dirs {
    fn new(layers: &[Layer]) -> Result<Self> {
        let mut buf = Vec::new();
        let mut spans = Vec::new();
        spans.try_reserve_exact(layers.len())?;
        for l in layers {
            let dir = layer_dir(&l.name);
            let start = buf.len();
            if display(&dir, PathHash(0), &mut buf)?.is_none() {
                buf.try_reserve(dir.len())?;
                buf.extend(dir.encode_utf16());
            }
            buf.push(0);
            let span =
                [start, buf.len()].map(|n| u32::try_from(n).map_err(|_| Error::OUT_OF_MEMORY));
            spans.push([span[0]?, span[1]?]);
        }
        Ok(Self { buf, spans })
    }

    fn utf16z(&self, i: usize) -> &[u16] {
        self.spans
            .get(i)
            .and_then(|&[a, b]| self.buf.get(a as usize..b as usize))
            .unwrap_or(&[])
    }
}

impl ModpkgArchive {
    pub(crate) fn build(
        stream: InStream,
        header: Header,
        signature: Vec<u8>,
        tables: Tables,
        records: Vec<Record>,
        metadata: Option<Metadata>,
        phy_size: u64,
    ) -> Result<Self> {
        let config = metadata
            .as_ref()
            .map(|m| Config::from_package(m, &tables.layers));
        let config_json = config.as_ref().map(Config::to_json).unwrap_or_default();
        let game_data: Vec<Option<Vec<u8>>> = tables
            .layers
            .iter()
            .map(|l| {
                let v = metadata.as_ref()?.layer(&l.name)?.game_data.as_ref()?;
                let manifest = config::game_data_manifest(v);
                Some(serde_json::to_vec_pretty(&manifest).expect("a JSON value serializes"))
            })
            .collect();

        let mut items: Vec<Item> = Vec::new();
        items.try_reserve_exact(records.len() + 1 + 2 * tables.layers.len())?;
        let mut buf = String::new();
        let mut spans: Vec<(PathHash, usize, usize)> = Vec::new();
        spans.try_reserve_exact(records.len() + 1 + tables.layers.len())?;
        for (i, r) in records.iter().enumerate() {
            if config.is_some() && r.is_meta() && tables.paths[r.path_index as usize] == INFO {
                continue;
            }
            let start = buf.len();
            path::show(&tables, r, &mut buf)?;
            spans.push((r.path_hash, start, buf.len()));
            items.push(Item::Record(i as u32));
        }
        if config.is_some() {
            let start = buf.len();
            buf.try_reserve(config::FILE.len())?;
            buf.push_str(config::FILE);
            spans.push((PathHash(0), start, buf.len()));
            items.push(Item::Config);
        }
        for (i, l) in tables.layers.iter().enumerate() {
            if game_data[i].is_none() {
                continue;
            }
            let start = buf.len();
            let name = format!("{}/{GAME_DATA}", layer_dir(&l.name));
            buf.try_reserve(name.len())?;
            buf.push_str(&name);
            spans.push((PathHash(0), start, buf.len()));
            items.push(Item::GameData(i as u32));
        }
        let mut entries: Vec<(PathHash, &str)> = Vec::new();
        entries.try_reserve_exact(spans.len())?;
        entries.extend(spans.iter().map(|&(h, a, b)| (h, &buf[a..b])));
        let names = Names::from_shown(&entries)?;
        items.extend((0..tables.layers.len() as u32).map(Item::Dir));
        let dirs = Dirs::new(&tables.layers)?;
        Ok(Self {
            stream,
            header,
            signature,
            tables,
            records,
            metadata,
            config,
            config_json,
            game_data,
            items,
            names,
            dirs,
            phy_size,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn signature(&self) -> &[u8] {
        &self.signature
    }

    pub fn tables(&self) -> &Tables {
        &self.tables
    }

    pub fn records(&self) -> &[Record] {
        &self.records
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The metadata document, when the package has one that reads.
    pub fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }

    /// The config shown for the document, when it reads.
    pub fn config(&self) -> Option<&Config> {
        self.config.as_ref()
    }

    /// The bytes of a generated file: the config or a layer's game data.
    fn generated(&self, item: Item) -> Option<&[u8]> {
        match item {
            Item::Config => Some(&self.config_json),
            Item::GameData(l) => self.game_data.get(l as usize)?.as_deref(),
            _ => None,
        }
    }

    fn item(&self, index: u32) -> Item {
        self.items[index as usize]
    }
}

impl Archive for ModpkgArchive {
    const ITEM_PROPS: &'static [PropInfo] = &[
        PropInfo::new(PropId::Path),
        PropInfo::new(PropId::IsDir),
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
            (Item::Dir(l), PropId::Path) => String::from_utf16_lossy(
                self.dirs
                    .utf16z(l as usize)
                    .strip_suffix(&[0])
                    .unwrap_or(&[]),
            )
            .into(),
            (Item::Dir(_), PropId::IsDir) => true.into(),
            (Item::Dir(_), _) => return None,
            (_, PropId::Path) => self.names.string(index).into(),
            (_, PropId::IsDir) => false.into(),
            (Item::Record(r), PropId::Size) => self.records[r as usize].uncompressed_size.into(),
            (Item::Record(r), PropId::PackSize) => self.records[r as usize].compressed_size.into(),
            (Item::Record(r), PropId::Method) => self.records[r as usize].compression.name().into(),
            (Item::Record(r), PropId::Offset) => self.records[r as usize].data_offset.into(),
            (_, PropId::Size | PropId::PackSize) => (self.generated(item)?.len() as u64).into(),
            (_, PropId::Method) => "Store".into(),
            _ => return None,
        })
    }

    fn archive_prop(&self, id: PropId) -> Option<Prop<'_>> {
        Some(match id {
            PropId::PhySize => self.phy_size.into(),
            PropId::UnpackVer => self.header.version.to_string().into(),
            PropId::Comment => {
                let layers: Vec<String> = self
                    .tables
                    .layers
                    .iter()
                    .map(|l| format!("{} ({})", l.name, l.priority))
                    .collect();
                let layers = format!("layers: {}", layers.join(", "));
                match self.metadata.as_ref().and_then(Metadata::summary) {
                    Some(summary) => format!("{summary}; {layers}").into(),
                    None => layers.into(),
                }
            }
            _ => return None,
        })
    }

    fn extract_item(&self, index: u32, out: &mut dyn Write) -> Result<OperationResult> {
        match self.item(index) {
            Item::Record(r) => decode::extract(&self.stream, &self.records[r as usize], out),
            Item::Dir(_) => Err(Error::INVALID_ARG),
            item => {
                out.write_all(self.generated(item).ok_or(Error::INVALID_ARG)?)?;
                Ok(OperationResult::OK)
            }
        }
    }

    /// Stored chunks only, as 7-Zip's own handlers do; the rest go through `Extract`.
    fn item_stream(&self, index: u32) -> Result<Option<ItemStream>> {
        let Item::Record(r) = self.item(index) else {
            return Ok(None);
        };
        let r = &self.records[r as usize];
        Ok((r.compression == Compression::Raw)
            .then(|| self.stream.slice(r.data_offset, r.compressed_size).into()))
    }

    fn raw_prop(&self, index: u32, id: PropId) -> Option<RawProp<'_>> {
        match (self.item(index), id) {
            (Item::Dir(l), PropId::Path) => Some(RawProp::Utf16Z(self.dirs.utf16z(l as usize))),
            (_, PropId::Path) => Some(RawProp::Utf16Z(self.names.utf16z(index))),
            _ => None,
        }
    }
}
