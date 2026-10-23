//! [`Names`]: item paths from the Mimir hash tables, in a form 7-Zip can extract on
//! Windows. Units that can't be in a file name become U+F000 plus the character, as
//! 7-Zip does for `\`; a last component too long is folded, or failing that partly hidden
//! behind its hash. [`PathHash::from_path`] reads all of it back.

use crate::builder::Builder;
use crate::hash::{self, MAX_NAME, PathHash, SLASH};
use lol7z_api::Result;
use lol7z_fold::{CLOSE, OPEN};
use ltk_hashdb::{Casing, HashDb, HashKind, KeyConfig, KeyWidth};
use ltk_mimir_cache::{HashStore, OpenError, Table};
use std::collections::TryReserveError;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Decompressed frames each table keeps while naming.
const FRAME_CACHE: usize = 256 << 10;
const WAD_KEYS: KeyConfig =
    KeyConfig::new(KeyWidth::U64, HashKind::Xxh64, Casing::AsciiInsensitive);

/// How the items got their names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// By their table path, in any form below.
    pub named: u32,
    /// Of `named`, with the last component folded: `preamble«fold»postamble`.
    pub folded: u32,
    /// Of `named`, too long even folded: `preamble«FULL-OUTER».ext`.
    pub hashed: u32,
    /// Of `named`, files whose path is also a folder's, marked with a trailing U+F02F.
    pub clash: u32,
    /// Not in any table: `«H»`.
    pub unknown: u32,
    /// In a table under a path that hashes to something else, or that can't be shown so
    /// it reads back: `«H»`.
    pub unverified: u32,
}

/// How [`display`] showed a path's last component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Plain,
    /// Too long for Windows, folded: `preamble«fold»postamble`.
    Folded,
    /// Too long even folded: `preamble«FULL-OUTER».ext`, FULL the path's hash and OUTER
    /// the hash of `dir/preamble.ext`.
    Hashed,
}

/// Every item's name, by index.
pub struct Names {
    /// Each name 0-terminated, end to end.
    pub(crate) buf: Vec<u16>,
    /// Each item's name in `buf`, the 0 included.
    pub(crate) spans: Vec<[u32; 2]>,
    pub(crate) comment: String,
    pub(crate) stats: Stats,
}

impl Names {
    /// Names from the machine's shared tables. Without them every item is `«H»`.
    pub fn new(hashes: impl ExactSizeIterator<Item = PathHash>) -> Result<Self> {
        match HashStore::discover() {
            Ok(store) => Self::with_store(&store, hashes),
            Err(_) => Self::build(hashes, &[], "names: none (no cache directory)".into()),
        }
    }

    /// Names from the tables in `dir`, laid out as the shared cache is.
    pub fn with_dir(dir: &Path, hashes: impl ExactSizeIterator<Item = PathHash>) -> Result<Self> {
        Self::with_store(&HashStore::at(dir), hashes)
    }

    /// Names from paths already looked up, by item.
    pub fn from_paths(entries: &[(PathHash, Option<&str>)]) -> Result<Self> {
        let hashes = collect(entries.iter().map(|e| e.0))?;
        let mut b = Builder::new(hashes.len())?;
        for (i, (hash, path)) in entries.iter().enumerate() {
            if let Some(path) = path {
                b.offer(i, *hash, path);
            }
        }
        b.finish(&hashes, "names: given".into())
    }

    /// Names from paths taken as given, shown as [`display`] shows them: for a format
    /// that stores its paths. `hash` is only what a hashed or unshowable name falls
    /// back to.
    pub fn from_shown(entries: &[(PathHash, &str)]) -> Result<Self> {
        let hashes = collect(entries.iter().map(|e| e.0))?;
        let mut b = Builder::new(hashes.len())?;
        for (i, (hash, path)) in entries.iter().enumerate() {
            b.offer_shown(i, *hash, path);
        }
        b.finish(&hashes, "names: stored".into())
    }

    fn with_store(
        store: &HashStore,
        hashes: impl ExactSizeIterator<Item = PathHash>,
    ) -> Result<Self> {
        let [game, lcu] = [Table::Game, Table::Lcu].map(|t| open(store, t));
        let mut dbs = Vec::new();
        let mut labels = Vec::new();
        let mut reasons = Vec::new();
        for r in [game, lcu] {
            match r {
                Ok((db, label)) => {
                    dbs.push(db);
                    labels.push(label);
                }
                Err(reason) => reasons.push(reason),
            }
        }
        reasons.dedup();
        let comment = match (labels.is_empty(), reasons.is_empty()) {
            (true, _) => format!("names: none ({})", reasons.join("; ")),
            (false, true) => format!("names: {}", labels.join(", ")),
            (false, false) => format!("names: {} ({})", labels.join(", "), reasons.join("; ")),
        };
        Self::build(hashes, &dbs, comment)
    }

    /// Looks `hashes` up in each of `dbs` in turn.
    fn build(
        hashes: impl ExactSizeIterator<Item = PathHash>,
        dbs: &[HashDb],
        comment: String,
    ) -> Result<Self> {
        let hashes = collect(hashes)?;
        let mut b = Builder::new(hashes.len())?;
        for db in dbs {
            db.for_each_batch(&hashes, |i, hash, path| {
                if let Some(path) = path {
                    b.offer(i, PathHash(hash), path);
                }
            });
        }
        b.finish(&hashes, comment)
    }

    /// The name, 0-terminated.
    pub fn utf16z(&self, index: u32) -> &[u16] {
        self.spans
            .get(index as usize)
            .and_then(|&[a, b]| self.buf.get(a as usize..b as usize))
            .unwrap_or(&[])
    }

    pub fn string(&self, index: u32) -> String {
        let s = self.utf16z(index);
        String::from_utf16_lossy(s.strip_suffix(&[0]).unwrap_or(s))
    }

    /// Which tables named the items: `names: game-<version>, lcu-<version>`, or
    /// `names: none (<reason>)`.
    pub fn comment(&self) -> &str {
        &self.comment
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }
}

/// Writes the name `path` shows as: forbidden characters and controls as U+F000 plus
/// the character, an empty component as U+F02F, and a last component longer than
/// Windows allows folded, or failing that as `preamble«FULL-OUTER».ext` with `hash` as
/// FULL. `None`, writing nothing, when a folder's name is too long.
pub fn display(
    path: &str,
    hash: PathHash,
    out: &mut Vec<u16>,
) -> Result<Option<Form>, TryReserveError> {
    show(path, hash, MAX_NAME, out)
}

/// [`display`] with the last component held to `limit`.
pub(crate) fn show(
    path: &str,
    hash: PathHash,
    limit: usize,
    out: &mut Vec<u16>,
) -> Result<Option<Form>, TryReserveError> {
    // An empty component is one unit where the path has none; a block is 35.
    out.try_reserve(path.len().saturating_mul(2).saturating_add(36))?;
    let base = out.len();
    let cut = path.rfind('/').map_or(0, |i| i + 1);
    let (dir, name) = path.split_at(cut);
    for part in dir.split_terminator('/') {
        if !push(out, part, MAX_NAME) {
            out.truncate(base);
            return Ok(None);
        }
        out.push(u16::from(b'/'));
    }
    if push(out, name, limit) {
        return Ok(Some(Form::Plain));
    }
    if let Some(f) = lol7z_fold::encode(name)
        && push(out, &f, limit)
    {
        return Ok(Some(Form::Folded));
    }
    let s = hash::hidden(name);
    let (pre, ext) = (&name[..s.pre], &name[s.ext..]);
    let outer = PathHash::of_game(&[dir, pre, ext]);
    if !pre.is_empty() {
        push(out, pre, usize::MAX);
    }
    push_char(out, OPEN);
    out.extend_from_slice(&hash.to_hex16());
    out.push(u16::from(b'-'));
    out.extend_from_slice(&outer.to_hex16());
    push_char(out, CLOSE);
    if !ext.is_empty() {
        push(out, ext, usize::MAX);
    }
    Ok(Some(Form::Hashed))
}

/// Writes component `part` as shown, unless that's longer than `limit`.
fn push(out: &mut Vec<u16>, part: &str, limit: usize) -> bool {
    if part.is_empty() {
        out.push(SLASH);
        return true;
    }
    let start = out.len();
    out.extend(part.encode_utf16().map(|u| match u {
        0..0x20 | 0x22 | 0x2A | 0x3A | 0x3C | 0x3E | 0x3F | 0x5C | 0x7C => 0xF000 + u,
        _ => u,
    }));
    if out.len() - start > limit {
        out.truncate(start);
        return false;
    }
    true
}

pub(crate) fn push_char(out: &mut Vec<u16>, c: char) {
    out.extend_from_slice(c.encode_utf16(&mut [0; 2]));
}

/// Whether a table's `path` is `hash`'s, and what [`display`] shows for it reads back.
pub(crate) fn verified(path: &str, hash: PathHash) -> bool {
    PathHash::of_game(&[path]) == hash && showable(path, hash)
}

/// Whether what [`display`] shows for `path` reads back to `hash`: nothing that reads
/// as a mapped unit, a separator or a block, and no stem of 16 hex digits but `hash`'s.
fn showable(path: &str, hash: PathHash) -> bool {
    !path.contains(|c| matches!(c, '\u{F000}'..='\u{F0FF}' | '\\' | OPEN | CLOSE))
        && hash::hex_stem(hash::last(path)).is_none_or(|h| h == hash.0)
}

/// The active version of `table` and its file name without the extension, or why not.
pub(crate) fn open(store: &HashStore, table: Table) -> Result<(HashDb, String), String> {
    let once = || -> Result<(HashDb, PathBuf), OpenError> {
        let path = store.path_for(table)?;
        let db = HashDb::options()
            .frame_cache_bytes(FRAME_CACHE)
            .open(&path)?;
        Ok((db, path))
    };
    // An update may have swept the file between the manifest read and the open.
    let r = match once() {
        Err(OpenError::HashDb(ltk_hashdb::OpenError::Io(e))) if e.kind() == ErrorKind::NotFound => {
            once()
        }
        r => r,
    };
    let (db, path) = r.map_err(|e| match e {
        OpenError::Manifest(e) => e.to_string(),
        OpenError::HashDb(ltk_hashdb::OpenError::Io(e)) => format!("{table}: {e}"),
        OpenError::HashDb(e) => format!("{table}: {e}"),
        e => format!("{table}: {e}"),
    })?;
    if db.key_config() != WAD_KEYS {
        return Err(format!("{table}: keyed {}", db.key_config()));
    }
    let label = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(table.id());
    Ok((db, label.to_owned()))
}

fn collect(hashes: impl ExactSizeIterator<Item = PathHash>) -> Result<Vec<u64>> {
    let mut v = Vec::new();
    v.try_reserve_exact(hashes.len())?;
    v.extend(hashes.map(|h| h.0));
    Ok(v)
}
