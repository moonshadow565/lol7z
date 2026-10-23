//! [`split`]: what of a name stays shown when even folded it's too long.

use crate::encode::{offset, split_ext, structure, tokens};

/// `name[..pre]` and `name[ext..]` stay shown; the middle between them is hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    pub pre: usize,
    pub ext: usize,
}

/// Shows the preamble [`encode`](crate::encode) keeps before its block (the whole stem
/// when nothing folds) and the extension, within `room` UTF-16 units: the extension goes
/// if it alone is longer, then the preamble loses tokens from its end.
pub fn split(name: &str, room: usize) -> Split {
    let units = |s: &str| s.encode_utf16().count();
    let (stem, ext) = split_ext(name);
    let ext = if units(ext) > room {
        name.len()
    } else {
        stem.len()
    };
    let left = room - units(&name[ext..]);
    let mut pre = match tokens(stem).and_then(|t| Some((structure(&t)?, t))) {
        Some((runs, toks)) => runs
            .first()
            .and_then(|r| toks.get(r.0))
            .map_or(0, |t| offset(stem, t)),
        None => stem.len(),
    };
    while units(&name[..pre]) > left {
        pre = stem[..pre].rfind('_').unwrap_or(0);
    }
    Split { pre, ext }
}
