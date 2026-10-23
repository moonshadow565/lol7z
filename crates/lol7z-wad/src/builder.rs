//! [`Builder`]: [`Names`] put together from the paths the tables offer.

use crate::hash::{MAX_NAME, PathHash, SLASH};
use crate::names::{Form, Names, Stats, display, push_char, show, verified};
use lol7z_api::{Error, Result};
use lol7z_fold::{CLOSE, OPEN};
use std::cmp::Ordering;

pub(crate) struct Builder {
    buf: Vec<u16>,
    /// `[_, 0]`: no name yet.
    spans: Vec<[u32; 2]>,
    /// Named items as named: in each table's path order, so nearly sorted.
    order: Vec<u32>,
    /// Items a table had under a path that doesn't hash to them or can't be shown.
    bad: Vec<u32>,
    stats: Stats,
    /// The first failure, for [`finish`](Self::finish).
    err: Option<Error>,
}

impl Builder {
    pub(crate) fn new(count: usize) -> Result<Self> {
        let mut spans = Vec::new();
        spans.try_reserve_exact(count)?;
        spans.resize(count, [0; 2]);
        let mut buf = Vec::new();
        buf.try_reserve(count.saturating_mul(19))?;
        let mut order = Vec::new();
        order.try_reserve_exact(count)?;
        Ok(Self {
            buf,
            spans,
            order,
            bad: Vec::new(),
            stats: Stats::default(),
            err: None,
        })
    }

    /// `path` for item `i`, unless it has a name already or `path` isn't its.
    pub(crate) fn offer(&mut self, i: usize, hash: PathHash, path: &str) {
        self.try_name(i, hash, path, true);
    }

    /// `path` for item `i` without checking that it hashes to `hash`.
    pub(crate) fn offer_shown(&mut self, i: usize, hash: PathHash, path: &str) {
        self.try_name(i, hash, path, false);
    }

    fn try_name(&mut self, i: usize, hash: PathHash, path: &str, check: bool) {
        if self.err.is_none()
            && let Err(e) = self.name(i, hash, path, check)
        {
            self.err = Some(e);
        }
    }

    fn name(&mut self, i: usize, hash: PathHash, path: &str, check: bool) -> Result<()> {
        if self.spans.get(i).is_none_or(|s| s[1] != 0) {
            return Ok(());
        }
        let start = offset(self.buf.len())?;
        let form = if !check || verified(path, hash) {
            display(path, hash, &mut self.buf)?
        } else {
            None
        };
        let Some(form) = form else {
            self.bad.try_reserve(1)?;
            self.bad.push(offset(i)?);
            return Ok(());
        };
        self.buf.push(0);
        if let Some(s) = self.spans.get_mut(i) {
            *s = [start, offset(self.buf.len())?];
        }
        self.order.push(offset(i)?);
        self.stats.named += 1;
        self.count(form);
        Ok(())
    }

    fn count(&mut self, form: Form) {
        match form {
            Form::Plain => {}
            Form::Folded => self.stats.folded += 1,
            Form::Hashed => self.stats.hashed += 1,
        }
    }

    fn name_of(&self, i: u32) -> &[u16] {
        self.spans
            .get(i as usize)
            .and_then(|&[a, b]| self.buf.get(a as usize..(b as usize).saturating_sub(1)))
            .unwrap_or(&[])
    }

    /// Marks files that are also folders, then names the rest by hash.
    pub(crate) fn finish(mut self, hashes: &[u64], comment: String) -> Result<Names> {
        if let Some(e) = self.err {
            return Err(e);
        }
        let mut order = std::mem::take(&mut self.order);
        order.sort_unstable_by(|&a, &b| dirs_first(self.name_of(a), self.name_of(b)));
        let mut clash = Vec::new();
        let mut i = 0;
        while i < order.len() {
            let name = order.get(i).map_or(&[][..], |&a| self.name_of(a));
            let mut j = i + 1;
            while order.get(j).is_some_and(|&b| same(self.name_of(b), name)) {
                j += 1;
            }
            let next = order.get(j).map_or(&[][..], |&b| self.name_of(b));
            if next.get(name.len()) == Some(&u16::from(b'/'))
                && next.get(..name.len()).is_some_and(|n| same(n, name))
            {
                let same = order.get(i..j).unwrap_or(&[]);
                clash.try_reserve(same.len())?;
                clash.extend_from_slice(same);
            }
            i = j;
        }
        for i in clash {
            self.mark(i, hashes.get(i as usize).copied().unwrap_or(0))?;
        }

        self.bad.sort_unstable();
        self.bad.dedup();
        let unnamed = |s: Option<&[u32; 2]>| s.is_some_and(|s| s[1] == 0);
        self.stats.unverified = self
            .bad
            .iter()
            .filter(|&&i| unnamed(self.spans.get(i as usize)))
            .count() as u32;
        for (span, &hash) in self.spans.iter_mut().zip(hashes) {
            if span[1] == 0 {
                self.buf.try_reserve(19)?;
                let start = offset(self.buf.len())?;
                push_char(&mut self.buf, OPEN);
                self.buf.extend_from_slice(&PathHash(hash).to_hex16());
                push_char(&mut self.buf, CLOSE);
                self.buf.push(0);
                *span = [start, offset(self.buf.len())?];
                self.stats.unknown += 1;
            }
        }
        self.stats.unknown = self.stats.unknown.saturating_sub(self.stats.unverified);
        Ok(Names {
            buf: self.buf,
            spans: self.spans,
            comment,
            stats: self.stats,
        })
    }

    /// Renames file `i` to its name and U+F02F, shown again a unit shorter if it's at
    /// the limit.
    fn mark(&mut self, i: u32, hash: u64) -> Result<()> {
        let Some(&[a, b]) = self.spans.get(i as usize) else {
            return Ok(());
        };
        let (a, end) = (a as usize, (b as usize).saturating_sub(1));
        let Some(name) = self.buf.get(a..end) else {
            return Ok(());
        };
        let dir = name
            .iter()
            .rposition(|&u| u == u16::from(b'/'))
            .map_or(0, |p| p + 1);
        // Only a plain name can be a folder's, so it reads back.
        let path = if name.len() - dir < MAX_NAME {
            None
        } else {
            Some(undisplay(name)?)
        };
        self.buf.try_reserve(end - a + 36)?;
        let start = offset(self.buf.len())?;
        let form = match &path {
            Some(path) => show(path, PathHash(hash), MAX_NAME - 1, &mut self.buf)?,
            None => None,
        };
        match form {
            Some(form) => self.count(form),
            None => self.buf.extend_from_within(a..end),
        }
        self.buf.push(SLASH);
        self.buf.push(0);
        self.stats.clash += 1;
        if let Some(s) = self.spans.get_mut(i as usize) {
            *s = [start, offset(self.buf.len())?];
        }
        Ok(())
    }
}

/// ASCII case folded, as Windows compares names.
fn lower(u: u16) -> u16 {
    if (0x41..=0x5A).contains(&u) {
        u + 0x20
    } else {
        u
    }
}

fn same(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| lower(x) == lower(y))
}

/// `/` before any other unit, so whatever is under a folder sorts right after a file of
/// its name; ASCII case folded.
fn dirs_first(a: &[u16], b: &[u16]) -> Ordering {
    let n = a
        .iter()
        .zip(b)
        .take_while(|(x, y)| lower(**x) == lower(**y))
        .count();
    let key = |u: Option<&u16>| {
        u.map(|&u| {
            if u == u16::from(b'/') {
                0
            } else {
                u32::from(lower(u)) + 1
            }
        })
    };
    key(a.get(n)).cmp(&key(b.get(n)))
}

/// The path a plain name was shown for.
fn undisplay(name: &[u16]) -> Result<String> {
    let mut s = String::new();
    s.try_reserve(name.len().saturating_mul(3))?;
    for (k, part) in name.split(|&u| u == u16::from(b'/')).enumerate() {
        if k > 0 {
            s.push('/');
        }
        if part == [SLASH] {
            continue;
        }
        let units = part.iter().map(|&u| match u {
            0xF000..=0xF0FF => u - 0xF000,
            _ => u,
        });
        s.extend(char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)));
    }
    Ok(s)
}

fn offset(n: usize) -> Result<u32> {
    u32::try_from(n).map_err(|_| Error::OUT_OF_MEMORY)
}
