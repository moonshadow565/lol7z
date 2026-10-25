//! The package's tables, as the reference extractor (cslol-manager's `wxyextract.cpp`)
//! reads them. Little-endian throughout. Two layouts share the `WXYS` magic and an i32
//! version after it.
//!
//! Versions 1 to 5 (`old`): name, author and version as counted strings (i32 length),
//! each a zlib stream missing its first header byte; from 3 on, category and
//! sub-category the same way and a count of preview images (u32 size, bool main, the
//! blob); before 3, one optional preview (u32 size, the blob). Then counted project
//! names (same encoding), from 4 on a delete list (i32 project, counted path), and the
//! file records: i32 project, counted path, i32 unpacked and packed size, and from 2 on
//! a 16-byte checksum. The data follows back to back; a packed entry is a zlib stream
//! whose first byte (first two bytes from 5 on) is left out, and its packed size counts
//! the bytes left out.
//!
//! Versions 6 and 7 (`OINK`): the `OINK` magic, five u16 packed lengths and the five
//! strings (name, author, version, category, sub-category), each a raw deflate stream,
//! XOR-masked first in 7. Then i32 previews (u8 kind, u32 size, the raw deflate blob),
//! u16 projects (u16 length, name), and per project: i32 files, their names (u16
//! length, masked in 7), and then one 61-byte record per file in the order of the names
//! sorted by the XXH64 of `deploy/` plus the lowercased name: 32-byte and 16-byte
//! checksums, the unpacked data's Adler-32 (big-endian; in 7 XOR-masked with the sizes
//! and 0x2A as a little-endian word), i32 unpacked and packed size, u8 method. The data
//! follows back to back.

use lol7z_api::{Error, Result};
use open_enum::open_enum;
use std::io::{BufRead, Read};
use xxhash_rust::xxh64::xxh64;

pub const MAGIC: &[u8; 4] = b"WXYS";
pub const OINK: &[u8; 4] = b"OINK";
/// First version with the `OINK` layout.
pub const OINK_VERSION: i32 = 6;
/// Longest string a table holds once unpacked: a guard against hostile streams only.
pub const MAX_STRING: u64 = 1 << 12;
/// The first byte of a zlib header, which old packages leave out.
pub const ZLIB_CMF: u8 = 0x78;
/// The second, which packages from version 5 leave out too.
pub const ZLIB_FLG: u8 = 0x9C;

#[open_enum]
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum Method {
    Store = 0,
    Deflate = 1,
    Zlib = 2,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Method::Store => "Store",
            Method::Deflate => "Deflate",
            Method::Zlib => "zlib",
            _ => "Unknown",
        }
    }
}

#[open_enum]
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum PreviewKind {
    Image = 0,
    Model = 1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub project: u16,
    /// As stored, decrypted; separators as written.
    pub path: String,
    pub size: u32,
    /// As stored. In old packages this counts the zlib header bytes left out.
    pub packed: u32,
    /// First stored byte.
    pub offset: u64,
    pub method: Method,
    pub checksum: [u8; 16],
    pub checksum2: [u8; 32],
    /// Adler-32 of the unpacked data; 0 in old packages, which don't store it.
    pub adler: u32,
}

impl Entry {
    /// Bytes the entry occupies in the file.
    pub fn stored(&self, version: i32) -> u64 {
        u64::from(self.packed).saturating_sub(header_cut(version))
    }
}

/// Bytes of the zlib header an old package leaves out of each packed entry.
pub fn header_cut(version: i32) -> u64 {
    match version {
        v if v >= OINK_VERSION => 0,
        v if v <= 4 => 1,
        _ => 2,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preview {
    pub kind: PreviewKind,
    /// First byte of the raw deflate stream.
    pub offset: u64,
    pub size: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub name: String,
    pub author: String,
    pub version: String,
    pub category: String,
    pub sub_category: String,
}

/// Everything the tables say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toc {
    pub version: i32,
    pub meta: Meta,
    pub projects: Vec<String>,
    pub previews: Vec<Preview>,
    pub entries: Vec<Entry>,
    /// Paths the package deletes, by project name.
    pub deletes: Vec<(String, String)>,
    /// Where the entry data starts.
    pub data_start: u64,
}

/// Reads the tables from `r`, at the start of the file, with `size` the file size.
/// `NotArchive` when the magic isn't there. `progress` is called with the entries read
/// so far and the position.
pub fn read(
    r: &mut impl BufRead,
    size: u64,
    mut progress: impl FnMut(u64, u64) -> Result<()>,
) -> Result<Toc> {
    let mut r = Reader { r, pos: 0, size };
    let mut magic = [0; 4];
    if r.fill(&mut magic).is_err() || &magic != MAGIC {
        return Err(Error::NotArchive);
    }
    let version = r.i32()?;
    if version < OINK_VERSION {
        read_old(&mut r, version, &mut progress)
    } else {
        read_oink(&mut r, version, &mut progress)
    }
}

struct Reader<'a, R> {
    r: &'a mut R,
    pos: u64,
    size: u64,
}

impl<R: BufRead> Reader<'_, R> {
    fn left(&self) -> u64 {
        self.size.saturating_sub(self.pos)
    }

    fn fill(&mut self, buf: &mut [u8]) -> Result<()> {
        self.r.read_exact(buf)?;
        self.pos += buf.len() as u64;
        Ok(())
    }

    fn u8(&mut self) -> Result<u8> {
        let mut b = [0];
        self.fill(&mut b)?;
        Ok(b[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let mut b = [0; 2];
        self.fill(&mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    fn u32(&mut self) -> Result<u32> {
        let mut b = [0; 4];
        self.fill(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    /// A count of things at least `each` bytes long: more than fit is a cut file.
    fn count(&mut self, n: i64, each: u64) -> Result<usize> {
        if n < 0 || n as u64 > self.left() / each.max(1) {
            return Err(Error::UNEXPECTED_END);
        }
        Ok(n as usize)
    }

    fn bytes(&mut self, len: u64) -> Result<Vec<u8>> {
        if len > self.left() {
            return Err(Error::UNEXPECTED_END);
        }
        let mut v = Vec::new();
        v.try_reserve_exact(len as usize)?;
        v.resize(len as usize, 0);
        self.fill(&mut v)?;
        Ok(v)
    }

    /// i32 length, then the bytes; nothing for a length of 0 or less.
    fn counted(&mut self) -> Result<Vec<u8>> {
        let len = self.i32()?;
        if len <= 0 {
            return Ok(Vec::new());
        }
        self.bytes(u64::from(len as u32))
    }

    fn skip(&mut self, n: u64) -> Result<()> {
        if n > self.left() {
            return Err(Error::UNEXPECTED_END);
        }
        let mut left = n;
        while left > 0 {
            let buf = self.r.fill_buf()?;
            if buf.is_empty() {
                return Err(Error::UNEXPECTED_END);
            }
            let k = (buf.len() as u64).min(left) as usize;
            self.r.consume(k);
            left -= k as u64;
        }
        self.pos += n;
        Ok(())
    }
}

fn read_old<R: BufRead>(
    r: &mut Reader<'_, R>,
    version: i32,
    progress: &mut impl FnMut(u64, u64) -> Result<()>,
) -> Result<Toc> {
    let string = |r: &mut Reader<'_, R>| -> Result<String> {
        let packed = r.counted()?;
        Ok(unpack_old(&packed))
    };
    let mut meta = Meta {
        name: string(r)?,
        author: string(r)?,
        version: string(r)?,
        ..Meta::default()
    };
    let mut previews = Vec::new();
    if version >= 3 {
        meta.category = string(r)?;
        meta.sub_category = string(r)?;
        let n = r.i32()?;
        let n = r.count(n.into(), 5)?;
        previews.try_reserve_exact(n)?;
        for _ in 0..n {
            let size = r.u32()?;
            let _main = r.u8()?;
            let offset = r.pos;
            r.skip(size.into())?;
            previews.push(Preview {
                kind: PreviewKind::Image,
                offset: offset + 1,
                size: size.saturating_sub(5),
            });
        }
    } else {
        let size = r.u32()?;
        if size != 0 {
            let offset = r.pos;
            r.skip(size.into())?;
            previews.push(Preview {
                kind: PreviewKind::Image,
                offset: offset + 1,
                size: size.saturating_sub(5),
            });
        }
    }

    let n = r.i32()?;
    let n = r.count(n.into(), 4)?;
    let mut projects = Vec::new();
    projects.try_reserve_exact(n)?;
    for _ in 0..n {
        projects.push(string(r)?);
    }
    let project = |i: i32| -> Result<u16> {
        if i < 0 || i as usize >= projects.len() {
            return Err(Error::HEADERS_ERROR);
        }
        u16::try_from(i).map_err(|_| Error::HEADERS_ERROR)
    };

    let mut deletes = Vec::new();
    if version >= 4 {
        let n = r.i32()?;
        let n = r.count(n.into(), 8)?;
        deletes.try_reserve_exact(n)?;
        for _ in 0..n {
            let p = project(r.i32()?)?;
            let path = string(r)?;
            deletes.push((projects[p as usize].clone(), path));
        }
    }

    let n = r.i32()?;
    let n = r.count(n.into(), if version == 1 { 16 } else { 32 })?;
    let mut entries = Vec::new();
    entries.try_reserve_exact(n)?;
    for i in 0..n {
        let p = project(r.i32()?)?;
        let path = string(r)?;
        let size = r.u32()?;
        let packed = r.u32()?;
        let mut checksum = [0; 16];
        if version != 1 {
            r.fill(&mut checksum)?;
        }
        entries.push(Entry {
            project: p,
            path,
            size,
            packed,
            offset: 0,
            method: if size == packed {
                Method::Store
            } else {
                Method::Zlib
            },
            checksum,
            checksum2: [0; 32],
            adler: 0,
        });
        if i % 1024 == 0 {
            progress(i as u64, r.pos)?;
        }
    }
    let data_start = r.pos;
    let mut offset = data_start;
    for e in &mut entries {
        e.offset = offset;
        offset = offset.saturating_add(e.stored(version));
    }
    Ok(Toc {
        version,
        meta,
        projects,
        previews,
        entries,
        deletes,
        data_start,
    })
}

/// 32 + 16 + 4 + 4 + 4 + 1.
const OINK_RECORD: u64 = 61;

fn read_oink<R: BufRead>(
    r: &mut Reader<'_, R>,
    version: i32,
    progress: &mut impl FnMut(u64, u64) -> Result<()>,
) -> Result<Toc> {
    let mut magic = [0; 4];
    r.fill(&mut magic)?;
    if &magic != OINK {
        return Err(Error::HEADERS_ERROR);
    }
    let mut lens = [0u16; 5];
    for l in &mut lens {
        *l = r.u16()?;
    }
    let mut strings: [String; 5] = Default::default();
    for (s, &len) in strings.iter_mut().zip(&lens) {
        let mut packed = r.bytes(len.into())?;
        if version > 6 {
            mask(&mut packed, 42, 17);
        }
        *s = unpack_raw(&packed);
    }
    let [name, author, ver, category, sub_category] = strings;
    let meta = Meta {
        name,
        author,
        version: ver,
        category,
        sub_category,
    };

    let n = r.i32()?;
    let n = r.count(n.into(), 5)?;
    let mut previews = Vec::new();
    previews.try_reserve_exact(n)?;
    for _ in 0..n {
        let kind = PreviewKind(r.u8()?);
        let size = r.u32()?;
        let offset = r.pos;
        r.skip(size.into())?;
        previews.push(Preview { kind, offset, size });
    }

    let n = r.u16()?;
    let n = r.count(n.into(), 2)?;
    let mut projects = Vec::new();
    projects.try_reserve_exact(n)?;
    for _ in 0..n {
        let len = r.u16()?;
        let bytes = r.bytes(len.into())?;
        projects.push(String::from_utf8_lossy(&bytes).into_owned());
    }

    let mut entries: Vec<Entry> = Vec::new();
    let mut names: Vec<(u64, String)> = Vec::new();
    for p in 0..projects.len() {
        let n = r.i32()?;
        let n = r.count(n.into(), OINK_RECORD + 2)?;
        names.clear();
        names.try_reserve(n)?;
        for _ in 0..n {
            let len = r.u16()?;
            let mut bytes = r.bytes(len.into())?;
            if version > 6 {
                mask(&mut bytes, 17, 42);
            }
            names.push((
                deploy_hash(&bytes),
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }
        names.sort_by_key(|a| a.0);
        entries.try_reserve(n)?;
        for (i, (_, path)) in names.drain(..).enumerate() {
            let mut checksum2 = [0; 32];
            let mut checksum = [0; 16];
            r.fill(&mut checksum2)?;
            r.fill(&mut checksum)?;
            let mut adler = r.u32()?;
            let size = r.u32()?;
            let packed = r.u32()?;
            let method = Method(r.u8()?);
            if version > 6 {
                adler ^= size ^ packed ^ 0x2A;
            }
            // Masked as read, but stored big-endian.
            let adler = adler.swap_bytes();
            entries.push(Entry {
                project: p as u16,
                path,
                size,
                packed,
                offset: 0,
                method,
                checksum,
                checksum2,
                adler,
            });
            if i % 1024 == 0 {
                progress(entries.len() as u64, r.pos)?;
            }
        }
    }
    let data_start = r.pos;
    let mut offset = data_start;
    for e in &mut entries {
        e.offset = offset;
        offset = offset.saturating_add(e.packed.into());
    }
    Ok(Toc {
        version,
        meta,
        projects,
        previews,
        entries,
        deletes: Vec::new(),
        data_start,
    })
}

/// The XOR mask over a string: byte `i` of `n` with `i * a + (n - i) * b`.
fn mask(bytes: &mut [u8], a: u32, b: u32) {
    let n = bytes.len() as u32;
    for (i, c) in (0u32..).zip(bytes.iter_mut()) {
        *c ^= (i.wrapping_mul(a).wrapping_add((n - i).wrapping_mul(b))) as u8;
    }
}

/// XXH64 of `deploy/` and the name lowercased, which orders a project's records.
pub fn deploy_hash(name: &[u8]) -> u64 {
    let mut v = Vec::with_capacity(name.len() + 7);
    v.extend_from_slice(b"deploy/");
    v.extend(name.iter().map(u8::to_ascii_lowercase));
    xxh64(&v, 0)
}

/// An old package's string: a zlib stream without its first header byte. What doesn't
/// inflate is dropped, as the reference does.
fn unpack_old(packed: &[u8]) -> String {
    if packed.is_empty() {
        return String::new();
    }
    let src = [ZLIB_CMF].chain(packed);
    lossy(flate2::read::ZlibDecoder::new(src))
}

/// An `OINK` string: a raw deflate stream.
fn unpack_raw(packed: &[u8]) -> String {
    if packed.is_empty() {
        return String::new();
    }
    lossy(flate2::read::DeflateDecoder::new(packed))
}

fn lossy(r: impl Read) -> String {
    let mut out = Vec::new();
    let _ = r.take(MAX_STRING).read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}
