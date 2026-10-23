//! A complete toy format, built as a 7-Zip plugin DLL:
//!
//! ```text
//! cargo build -p lol7z_api --example toy
//! copy target\debug\examples\toy.dll "<7-Zip folder>\Formats\"
//! ```
//!
//! Layout: `TOY\x01`, item count (u32), then per item a name length (u16), the UTF-8
//! name, a data length (u32) and the data. Names ending in `/` are directories.
//! All integers little-endian.

use lol7z_api::{
    Archive, Error, ErrorFlags, Ext, Format, GUID, InStream, ItemStream, OpenCallback,
    OperationResult, Opt, Options, OutStream, Prop, PropId, PropInfo, RawProp, Result, Update,
    UpdateCallback,
};
use std::io::{self, Read, Write};

pub struct Toy;

pub struct ToyArchive {
    stream: InStream,
    items: Vec<Item>,
    phy_size: u64,
}

pub struct Item {
    /// As stored: directories end in `/`.
    pub path: String,
    /// As shown: no trailing `/`, UTF-16, 0-terminated.
    pub path16: Vec<u16>,
    pub offset: u64,
    pub size: u32,
}

impl Item {
    fn is_dir(&self) -> bool {
        self.path.ends_with('/')
    }
}

const MAGIC: &[u8; 4] = b"TOY\x01";
/// Name length and data length.
const ITEM_HEADER: u64 = 6;

impl Format for Toy {
    const NAME: &'static str = "Toy";
    const CLASS_ID: GUID = GUID::from_u128(0x6c746b37_7a00_4a1b_9c3d_746f79000001);
    const EXTENSIONS: &'static [Ext] = &[Ext::new("toy")];
    const SIGNATURES: &'static [&'static [u8]] = &[MAGIC];

    type Archive = ToyArchive;

    /// Takes `-mx` (compression level, which the toy doesn't use) and nothing else.
    fn check_option(opt: &Opt) -> Result<()> {
        if opt.name.eq_ignore_ascii_case("x") {
            Ok(())
        } else {
            Err(Error::INVALID_ARG)
        }
    }

    fn open(stream: InStream, cb: &OpenCallback<'_>, _options: &Options) -> Result<ToyArchive> {
        let file_size = stream.size()?;
        let mut r = io::BufReader::new(stream.clone());
        let mut magic = [0; 4];
        if r.read_exact(&mut magic).is_err() || &magic != MAGIC {
            return Err(Error::NotArchive);
        }
        // Cut off before the count: nothing to list. `?` makes it Damaged(UnexpectedEnd).
        let count = read_u32(&mut r)?;
        cb.set_total(Some(count.into()), Some(file_size))?;

        // Every item takes at least ITEM_HEADER bytes, which bounds the allocation even
        // when `count` is garbage.
        let fit = file_size.saturating_sub(8) / ITEM_HEADER;
        let mut items = Vec::with_capacity(u64::from(count).min(fit) as usize);
        let mut pos = 8u64;
        for i in 0..count {
            let (name_len, path, size) = match read_item_header(&mut r) {
                Ok(header) => header,
                // Truncated: keep the items read so far, and let 7-Zip say so.
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    cb.error(ErrorFlags::UnexpectedEnd);
                    break;
                }
                Err(e) => return Err(e.into()),
            };
            let offset = pos + ITEM_HEADER + u64::from(name_len);
            pos = offset + u64::from(size);
            let path16 = path
                .trim_end_matches('/')
                .encode_utf16()
                .chain([0])
                .collect();
            items.push(Item {
                path,
                path16,
                offset,
                size,
            });
            if pos > file_size {
                // This item's data is cut short; extracting it reports UnexpectedEnd.
                cb.error(ErrorFlags::UnexpectedEnd);
                break;
            }
            r.seek_relative(i64::from(size))?;
            if i % 1024 == 0 {
                cb.set_completed(Some(i.into()), Some(pos))?;
            }
        }
        Ok(ToyArchive {
            stream,
            items,
            phy_size: pos,
        })
    }
}

fn read_item_header(r: &mut impl Read) -> io::Result<(u16, String, u32)> {
    let name_len = read_u16(r)?;
    let mut name = vec![0; name_len as usize];
    r.read_exact(&mut name)?;
    let size = read_u32(r)?;
    Ok((name_len, String::from_utf8_lossy(&name).into_owned(), size))
}

impl Archive for ToyArchive {
    const ITEM_PROPS: &'static [PropInfo] = &[
        PropInfo::new(PropId::Path),
        PropInfo::new(PropId::IsDir),
        PropInfo::new(PropId::Size),
        PropInfo::new(PropId::Offset),
    ];
    const ARCHIVE_PROPS: &'static [PropInfo] = &[PropInfo::new(PropId::PhySize)];

    fn item_count(&self) -> u32 {
        self.items.len() as u32
    }

    fn item_prop(&self, index: u32, id: PropId) -> Option<Prop<'_>> {
        let item = &self.items[index as usize];
        Some(match id {
            PropId::Path => item.path.trim_end_matches('/').into(),
            PropId::IsDir => item.is_dir().into(),
            PropId::Size => u64::from(item.size).into(),
            PropId::Offset => item.offset.into(),
            _ => return None,
        })
    }

    fn archive_prop(&self, id: PropId) -> Option<Prop<'_>> {
        match id {
            PropId::PhySize => Some(self.phy_size.into()),
            _ => None,
        }
    }

    fn extract_item(&self, index: u32, out: &mut dyn Write) -> Result<OperationResult> {
        let item = &self.items[index as usize];
        let copied = io::copy(&mut self.stream.slice(item.offset, item.size.into()), out)?;
        Ok(if copied == u64::from(item.size) {
            OperationResult::OK
        } else {
            OperationResult::UnexpectedEnd
        })
    }

    fn item_stream(&self, index: u32) -> Result<Option<ItemStream>> {
        let item = &self.items[index as usize];
        Ok(Some(
            self.stream.slice(item.offset, item.size.into()).into(),
        ))
    }

    /// 7-Zip reads paths through here first, so listing makes no BSTR per item.
    fn raw_prop(&self, index: u32, id: PropId) -> Option<RawProp<'_>> {
        match id {
            PropId::Path => Some(RawProp::Utf16Z(&self.items[index as usize].path16)),
            _ => None,
        }
    }
}

impl Update for Toy {
    fn update(
        existing: Option<&ToyArchive>,
        mut out: OutStream,
        cb: &UpdateCallback<'_>,
        _options: &Options,
    ) -> Result<()> {
        out.write_all(MAGIC)?;
        out.write_all(&cb.len().to_le_bytes())?;
        let mut done = 0u64;
        for item in cb.items() {
            let item = item?;
            cb.set_completed(done)?;
            let old = item
                .old_index
                .zip(existing)
                .map(|(i, a)| &a.items[i as usize]);
            let mut path = match (item.new_props, old) {
                (false, Some(old)) => old.path.trim_end_matches('/').to_owned(),
                _ => cb.path(item.index)?.unwrap_or_default().replace('\\', "/"),
            };
            let is_dir = match (item.new_props, old) {
                (false, Some(old)) => old.is_dir(),
                _ => cb.is_dir(item.index)?,
            };
            if is_dir {
                path.push('/');
            }

            let name_len = u16::try_from(path.len()).map_err(|_| Error::INVALID_ARG)?;
            out.write_all(&name_len.to_le_bytes())?;
            out.write_all(path.as_bytes())?;

            if is_dir {
                out.write_all(&0u32.to_le_bytes())?;
            } else if item.new_data {
                // The size goes before the data, and only the stream knows it for sure.
                let mut data = Vec::new();
                if let Some(mut s) = cb.stream(item.index)? {
                    s.read_to_end(&mut data)?;
                }
                cb.item_done()?;
                let size = u32::try_from(data.len()).map_err(|_| Error::INVALID_ARG)?;
                out.write_all(&size.to_le_bytes())?;
                out.write_all(&data)?;
                done += data.len() as u64;
            } else {
                // Unchanged data: stream it straight across.
                let (Some(old_index), Some(old), Some(a)) = (item.old_index, old, existing) else {
                    return Err(Error::INVALID_ARG);
                };
                out.write_all(&old.size.to_le_bytes())?;
                let copied = io::copy(&mut a.stream.slice(old.offset, old.size.into()), &mut out)?;
                if copied != u64::from(old.size) {
                    return Err(Error::UNEXPECTED_END);
                }
                cb.copied(old_index)?;
                done += copied;
            }
        }
        cb.set_completed(done)
    }
}

fn read_u16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

lol7z_api::formats! {
    update Toy,
}
