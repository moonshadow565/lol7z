//! Item names: what a chunk lists as, and what a name a host gives means. The listing
//! is the mod project layout a package unpacks to:
//!
//! ```text
//! mod.config.json                  the metadata document, as the project's config
//! README.md  LICENSE  thumbnail.webp
//! hashes/<file>                    _meta_/hashes/<file>
//! content/<layer>/game_data.json   the layer's game-data document
//! content/<layer>/<WAD>/<path>     a content chunk; without the WAD part when it has none
//! _meta_/<other>                   a meta chunk the format doesn't name
//! ```
//!
//! Characters Windows can't store and over-long last components are shown as the WAD
//! plugin shows them ([`lol7z_wad::display`]), and read back the same way.

use crate::toc::{NONE, Record, Tables};
use lol7z_api::{Error, Result};
use lol7z_fold::{CLOSE, FoldError, GROUP_OPEN, OPEN};
use lol7z_wad::PathHash;
use std::borrow::Cow;
use xxhash_rust::xxh64::xxh64;

pub const CONTENT: &str = "content";
pub const META: &str = "_meta_";
pub const META_PREFIX: &str = "_meta_/";
pub const INFO: &str = "_meta_/info.msgpack";
pub const README: &str = "_meta_/readme.md";
pub const LICENSE: &str = "_meta_/license";
pub const THUMBNAIL: &str = "_meta_/thumbnail.webp";
pub const HASHES_PREFIX: &str = "_meta_/hashes/";
/// Where the project keeps its tables.
pub const HASHES: &str = "hashes";
pub const GAME_DATA: &str = "game_data.json";
pub const BASE: &str = "base";
/// Longest file name Windows allows, in UTF-16 units.
const MAX_NAME: usize = 255;

/// `content/<name>`.
pub fn layer_dir(name: &str) -> String {
    format!("{CONTENT}/{name}")
}

/// The file under `_meta_/hashes/`, for a path directly under it.
pub fn hashes_file(path: &str) -> Option<&str> {
    path.strip_prefix(HASHES_PREFIX)
        .filter(|f| !f.is_empty() && !f.contains('/'))
}

/// Appends the listing name of `r` to `out`. Indices must be valid or `NONE`.
pub(crate) fn show(t: &Tables, r: &Record, out: &mut String) -> Result<()> {
    let path = &t.paths[r.path_index as usize];
    if r.is_meta() {
        let shown: Cow<'_, str> = match path.as_str() {
            README => "README.md".into(),
            LICENSE => "LICENSE".into(),
            THUMBNAIL => "thumbnail.webp".into(),
            p => match hashes_file(p) {
                Some(f) => format!("{HASHES}/{f}").into(),
                None => p.into(),
            },
        };
        out.try_reserve(shown.len())?;
        out.push_str(&shown);
        return Ok(());
    }
    out.try_reserve(path.len() + 80)?;
    if r.layer_index != NONE {
        out.push_str(&layer_dir(&t.layers[r.layer_index as usize].name));
        out.push('/');
    }
    if r.wad_index != NONE {
        let wad = &t.wads[r.wad_index as usize];
        out.try_reserve(wad.len() + 1)?;
        out.push_str(wad);
        out.push('/');
    }
    out.push_str(path);
    Ok(())
}

/// A chunk as a host names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// The stored path, `/` separators, as the host spelled it.
    pub path: String,
    pub hash: PathHash,
    /// `None` for a meta chunk.
    pub layer: Option<String>,
    pub wad: Option<String>,
}

/// What a name a host gives means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    Chunk(Chunk),
    /// A layer's folder, `content/<name>`.
    Layer(String),
    /// `content/<layer>/game_data.json`.
    GameData(String),
    /// `mod.config.json`.
    Config,
    /// A folder nothing is read from: `content`, `_meta_`, `hashes`, a WAD folder.
    Folder,
    /// A project file that isn't package content: `.modignore`.
    Ignored,
}

/// Reads `name` back: U+F000 mapped characters, an empty component shown as U+F02F,
/// a trailing clash mark, and a folded or hashed block in the last component. Then
/// the project layout: the root files, `hashes/<file>`, `_meta_/...` as is, and under
/// `content/<layer>` a folder named `*.wad.client` or `*.wad` as the WAD (stored
/// lowercased), a WAD-root file whose stem is 16 hex digits as its hash.
///
/// `INVALID_ARG` for a name outside the layout, a block that doesn't decode, or a name
/// that escapes: a `..` component, a `:`.
pub fn read(name: &str) -> Result<Read> {
    let name = name.strip_suffix(['/', '\\']).unwrap_or(name);
    let mut parts: Vec<String> = Vec::new();
    let n = name.split(['/', '\\']).count();
    parts.try_reserve_exact(n)?;
    for (i, part) in name.split(['/', '\\']).enumerate() {
        parts.push(component(part, i + 1 == n)?);
    }
    if parts.iter().any(|p| p == ".." || p.contains(':')) {
        return Err(Error::INVALID_ARG);
    }
    let [first, rest @ ..] = &parts[..] else {
        return Err(Error::INVALID_ARG);
    };
    let is = |s: &str, what: &str| s.eq_ignore_ascii_case(what);
    if rest.is_empty() {
        return Ok(match first.as_str() {
            f if is(f, "mod.config.json") => Read::Config,
            f if is(f, ".modignore") => Read::Ignored,
            f if is(f, "README.md") => meta(README),
            f if is(f, "LICENSE") || is(f, "LICENSE.md") || is(f, "LICENSE.txt") => meta(LICENSE),
            f if is(f, "thumbnail.webp") => meta(THUMBNAIL),
            f if is(f, HASHES) || is(f, CONTENT) || f == META => Read::Folder,
            _ => return Err(Error::INVALID_ARG),
        });
    }
    if first == META {
        return Ok(meta(&parts.join("/")));
    }
    if is(first, HASHES) {
        return match rest {
            [file] => Ok(meta(&format!("{HASHES_PREFIX}{file}"))),
            _ => Err(Error::INVALID_ARG),
        };
    }
    if !is(first, CONTENT) {
        return Err(Error::INVALID_ARG);
    }
    let [layer, rest @ ..] = rest else {
        return Ok(Read::Folder);
    };
    if !is_slug(layer) {
        return Err(Error::INVALID_ARG);
    }
    match rest {
        [] => return Ok(Read::Layer(layer.clone())),
        [f] if is(f, GAME_DATA) => return Ok(Read::GameData(layer.clone())),
        _ => {}
    }
    let (wad, rest) = match rest {
        [w, rest @ ..] if is_wad(w) => (Some(w.to_ascii_lowercase()), rest),
        _ => (None, rest),
    };
    if rest.is_empty() {
        return Ok(Read::Folder);
    }
    let mut path = rest.join("/");
    if path.len() >= META_PREFIX.len()
        && path[..META_PREFIX.len()].eq_ignore_ascii_case(META_PREFIX)
    {
        return Err(Error::INVALID_ARG);
    }
    let hash = match rest {
        [only] if let Some(h) = hex_stem(only) => {
            path.make_ascii_lowercase();
            PathHash(h)
        }
        _ => PathHash(xxh64(path.to_ascii_lowercase().as_bytes(), 0)),
    };
    Ok(Read::Chunk(Chunk {
        path,
        hash,
        layer: Some(layer.clone()),
        wad,
    }))
}

fn meta(path: &str) -> Read {
    Read::Chunk(Chunk {
        path: path.to_owned(),
        hash: PathHash(xxh64(path.to_ascii_lowercase().as_bytes(), 0)),
        layer: None,
        wad: None,
    })
}

/// One shown component read back. The last one may carry a clash mark or a block.
fn component(part: &str, last: bool) -> Result<String> {
    let part = if last {
        part.strip_suffix('\u{F02F}').unwrap_or(part)
    } else {
        part
    };
    if part == "\u{F02F}" {
        return Ok(String::new());
    }
    let mut s = String::new();
    s.try_reserve(part.len())?;
    s.extend(part.chars().map(|c| match u32::from(c) {
        u @ 0xF000..=0xF0FF => char::from(u as u8),
        _ => c,
    }));
    if !last || !s.contains(OPEN) {
        return Ok(s);
    }
    let Some((a, b)) = block(&s) else {
        return Ok(s);
    };
    let (inner, ext) = (&s[a + OPEN.len_utf8()..b], &s[b + CLOSE.len_utf8()..]);
    let hash = hex(inner).or_else(|| inner.split_once('-').and_then(|(f, o)| hex(o).and(hex(f))));
    if let Some(h) = hash {
        return Ok(format!("{h:016x}{ext}"));
    }
    if !inner.contains(GROUP_OPEN) {
        return Ok(s);
    }
    // Longer than anything shown, and costly to decode.
    if s.encode_utf16().nth(MAX_NAME).is_some() {
        return Err(Error::INVALID_ARG);
    }
    lol7z_fold::decode(&s).map_err(|e| match e {
        FoldError::OutOfMemory => Error::OUT_OF_MEMORY,
        _ => Error::INVALID_ARG,
    })
}

fn block(name: &str) -> Option<(usize, usize)> {
    let a = name.find(OPEN)?;
    let b = name[a..].find(CLOSE)?;
    Some((a, a + b))
}

/// Non-empty ASCII lowercase letters, digits, `-` and `_`, neither first nor last.
pub fn is_slug(s: &str) -> bool {
    let ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_';
    let edge = |b: &u8| matches!(b, b'-' | b'_');
    !s.is_empty()
        && s.bytes().all(ok)
        && !s.as_bytes().first().is_some_and(edge)
        && !s.as_bytes().last().is_some_and(edge)
}

fn is_wad(s: &str) -> bool {
    let ends = |suffix: &str| {
        let at = s.len().wrapping_sub(suffix.len());
        s.len() >= suffix.len() && s.is_char_boundary(at) && s[at..].eq_ignore_ascii_case(suffix)
    };
    ends(".wad.client") || ends(".wad")
}

/// The hash a name's stem spells, if it's 16 hex digits.
pub(crate) fn hex_stem(name: &str) -> Option<u64> {
    hex(name.split('.').next().unwrap_or(name))
}

fn hex(s: &str) -> Option<u64> {
    if s.len() != 16 {
        return None;
    }
    s.bytes().try_fold(0u64, |h, b| {
        let d = (b as char).to_digit(16)?;
        Some(h << 4 | u64::from(d))
    })
}

/// A name that would leave the output directory (section 12.3 of the format).
pub(crate) fn escapes(s: &str) -> bool {
    s.starts_with('/')
        || s.starts_with('\\')
        || s.contains(':')
        || s.contains('\\')
        || s.split('/').any(|c| c == "..")
}
