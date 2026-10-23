//! Path hashes: the game's XXH64 of the lowercased path with `/` separators.

use crate::hasher::Hasher;
use core::fmt;
use lol7z_api::{Error, Result};
use lol7z_fold::{CLOSE, FoldError, GROUP_OPEN, OPEN, Split};
use xxhash_rust::xxh64::xxh64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathHash(pub u64);

/// What an empty path component and the clash marker are shown as.
pub(crate) const SLASH: u16 = 0xF02F;
/// Longest file name Windows allows, in UTF-16 units.
pub(crate) const MAX_NAME: usize = 255;

impl PathHash {
    /// The hash `path` names, reading back what [`Names`](crate::Names) shows: one
    /// trailing U+F02F is dropped, a unit in U+F000..=U+F0FF is its low byte, and a
    /// component that is only U+F02F is empty. In the last component, a block `«H»` (16
    /// hex digits) is the hash H wherever it is, a folded block is expanded,
    /// `«FULL-OUTER»` is FULL while the path around it still hashes to OUTER, and a stem
    /// of 16 hex digits is the hash itself. Any other path is hashed the way the game
    /// does.
    ///
    /// `INVALID_ARG` for a block that doesn't decode, and for a `«FULL-OUTER»` that was
    /// moved or renamed, which only [`Resolver`](crate::Resolver) can hash.
    pub fn from_path(path: &str) -> Result<Self> {
        Self::read(path, |_| None)
    }

    /// [`from_path`](Self::from_path), with `known` giving FULL's table path for a
    /// `«FULL-OUTER»` that no longer matches. OUTER is of folder, preamble and extension
    /// end to end, so units moved between preamble and extension go unnoticed, by design.
    pub(crate) fn read(path: &str, known: impl FnOnce(Self) -> Option<String>) -> Result<Self> {
        let plain = path
            .bytes()
            .all(|b| b.is_ascii() && !b.is_ascii_uppercase() && b != b'\\');
        if plain {
            return Ok(hex_stem(last(path)).map_or_else(|| Self(xxh64(path.as_bytes(), 0)), Self));
        }
        let path = path.strip_suffix('\u{F02F}').unwrap_or(path);
        let (dir, name) = path.split_at(path.rfind(['/', '\\']).map_or(0, |i| i + 1));
        if let Some((a, b)) = block(name) {
            let (pre, inner, ext) = (
                &name[..a],
                &name[a + OPEN.len_utf8()..b],
                &name[b + CLOSE.len_utf8()..],
            );
            if let Some(h) = hex(inner) {
                return Ok(Self(h));
            }
            if let Some((full, outer)) = inner
                .split_once('-')
                .and_then(|(f, o)| Some((hex(f)?, hex(o)?)))
            {
                if shown(&[dir, pre, ext]) == Self(outer) {
                    return Ok(Self(full));
                }
                let known = known(Self(full)).ok_or(Error::INVALID_ARG)?;
                let was = last(&known);
                let s = hidden(was);
                let middle = was.get(s.pre..s.ext).ok_or(Error::INVALID_ARG)?;
                return Ok(shown(&[dir, pre, middle, ext]));
            }
            if inner.contains(GROUP_OPEN) {
                // Longer than anything shown, and costly to decode.
                if name.encode_utf16().nth(MAX_NAME).is_some() {
                    return Err(Error::INVALID_ARG);
                }
                let name = lol7z_fold::decode(name).map_err(|e| match e {
                    FoldError::OutOfMemory => Error::OUT_OF_MEMORY,
                    _ => Error::INVALID_ARG,
                })?;
                return Ok(shown(&[dir, &name]));
            }
        }
        Ok(hex_stem(name).map_or_else(|| shown(&[path]), Self))
    }

    /// The game's hash of `parts` end to end.
    pub(crate) fn of_game(parts: &[&str]) -> Self {
        if let [path] = parts
            && !path.bytes().any(|b| b.is_ascii_uppercase())
        {
            return Self(xxh64(path.as_bytes(), 0));
        }
        let mut h = Hasher::new();
        for p in parts {
            h.game(p);
        }
        h.finish()
    }

    /// 16 hex digits.
    pub(crate) fn to_hex16(self) -> [u16; 16] {
        let mut s = [0u16; 16];
        for (i, c) in s.iter_mut().enumerate() {
            let d = ((self.0 >> (60 - 4 * i)) & 0xF) as u8;
            *c = u16::from(if d < 10 { b'0' + d } else { b'a' + d - 10 });
        }
        s
    }
}

/// What of a last component too long even folded stays shown around its
/// `«FULL-OUTER»`: 35 units for the block, 1 for a clash mark.
pub(crate) fn hidden(name: &str) -> Split {
    lol7z_fold::split(name, MAX_NAME - 36)
}

/// Where the block in `name` opens and closes.
fn block(name: &str) -> Option<(usize, usize)> {
    let a = name.find(OPEN)?;
    let b = name[a..].find(CLOSE)?;
    Some((a, a + b))
}

/// The hash of `parts` end to end, read as [`Names`](crate::Names) shows them.
fn shown(parts: &[&str]) -> PathHash {
    let mut h = Hasher::new();
    for p in parts {
        h.shown(p);
    }
    h.finish()
}

/// The last component.
pub(crate) fn last(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The hash a name's stem spells, if it's 16 hex digits.
pub(crate) fn hex_stem(name: &str) -> Option<u64> {
    hex(name.split('.').next().unwrap_or(name))
}

/// Exactly 16 hex digits.
fn hex(s: &str) -> Option<u64> {
    if s.len() != 16 {
        return None;
    }
    s.bytes().try_fold(0u64, |h, b| {
        let d = (b as char).to_digit(16)?;
        Some(h << 4 | u64::from(d))
    })
}

impl fmt::Display for PathHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::LowerHex for PathHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}
