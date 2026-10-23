//! [`encode`]: a name folded into one block.

use crate::{CLOSE, GROUP_CLOSE, GROUP_OPEN, OPEN, RANGE, SEP, decode, reserved};

/// `name` folded, or `None` when nothing in it repeats or it can't be written safely.
/// Whether the result is short enough is the caller's to check.
///
/// The stem (all before the last `.`, unless that is the first character) is cut into
/// tokens before each `_`, and the tokens into pairs, starting at the first or the
/// second token, whichever gives fewer runs of pairs sharing a first token. Runs of two
/// or more pairs fold, everything from the first such run to the end of the last goes
/// in the block, and each run becomes its first token and its second tokens as a group.
pub fn encode(name: &str) -> Option<String> {
    let (stem, _) = split_ext(name);
    let toks = tokens(stem)?;
    let runs = structure(&toks)?;
    let (&(first, _), &(last, n)) = (runs.first()?, runs.last()?);
    let at = |k: usize| toks.get(k).map_or(stem.len(), |t| offset(stem, t));
    let (a, b) = (at(first), at(last + 2 * n));
    let (outer1, inner, outer2) = (&name[..a], &name[a..b], &name[b..]);
    if outer1.contains([OPEN, CLOSE]) || outer2.contains([OPEN, CLOSE]) || inner.contains(reserved)
    {
        return None;
    }

    // (first run, runs merged, items joined, item count)
    let mut groups: Vec<(usize, usize, String, usize)> = Vec::new();
    groups.try_reserve(runs.len()).ok()?;
    for (r, &(start, count)) in runs.iter().enumerate() {
        let (items, n) = items(&toks, start, count);
        if let Some(g) = groups.last_mut()
            && g.2 == items
            && product(runs.get(g.0..=r)?.iter().map(|&(s, _)| toks[s]))
        {
            g.1 += 1;
            continue;
        }
        groups.push((r, 1, items, n));
    }

    let mut out = String::new();
    out.try_reserve(2 * name.len()).ok()?;
    out.push_str(outer1);
    out.push(OPEN);
    for (r, m, items, n) in groups {
        let words = runs.get(r..r + m)?.iter().map(|&(s, _)| toks[s]);
        if m == 1 {
            words.for_each(|w| out.push_str(w));
        } else {
            out.push('_');
            out.push(GROUP_OPEN);
            for (k, w) in words.enumerate() {
                if k > 0 {
                    out.push(SEP);
                }
                out.push_str(w.get(1..)?);
            }
            out.push(GROUP_CLOSE);
        }
        if n == 1 && m == 1 && items.ends_with(GROUP_CLOSE) {
            out.push_str(&items);
        } else {
            out.push(GROUP_OPEN);
            out.push_str(&items);
            out.push(GROUP_CLOSE);
        }
    }
    if !out[a..].contains(GROUP_OPEN) {
        return None;
    }
    out.push(CLOSE);
    out.push_str(outer2);
    if decode(&out).ok()? != name {
        return None;
    }
    Some(out)
}

/// Stem and extension: the extension starts at the last `.`, unless that is the first
/// character.
pub(crate) fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(d) if d > 0 => name.split_at(d),
        _ => (name, ""),
    }
}

/// `stem` cut before each `_` but a leading one.
pub(crate) fn tokens(stem: &str) -> Option<Vec<&str>> {
    let mut toks = Vec::new();
    toks.try_reserve(stem.bytes().filter(|&b| b == b'_').count() + 1)
        .ok()?;
    let mut j = 0;
    for (i, _) in stem.match_indices('_').filter(|&(i, _)| i > 0) {
        toks.push(&stem[j..i]);
        j = i;
    }
    if !stem.is_empty() {
        toks.push(&stem[j..]);
    }
    Some(toks)
}

/// The runs from the first to the last that fold, as (first token, pairs); `None` when
/// none folds.
pub(crate) fn structure(toks: &[&str]) -> Option<Vec<(usize, usize)>> {
    let runs_from = |p: usize| -> Option<Vec<(usize, usize)>> {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        runs.try_reserve(toks.len() / 2).ok()?;
        let mut k = p;
        while k + 1 < toks.len() {
            match runs.last_mut() {
                Some(r) if toks[r.0] == toks[k] => r.1 += 1,
                _ => runs.push((k, 1)),
            }
            k += 2;
        }
        Some(runs)
    };
    let (even, odd) = (runs_from(0)?, runs_from(1)?);
    let mut runs = if odd.len() < even.len() { odd } else { even };
    let first = runs.iter().position(|r| r.1 >= 2)?;
    let last = runs.iter().rposition(|r| r.1 >= 2)?;
    runs.truncate(last + 1);
    runs.drain(..first);
    Some(runs)
}

/// Where `tok`, a slice of `stem`, starts in it.
pub(crate) fn offset(stem: &str, tok: &str) -> usize {
    (tok.as_ptr() as usize).saturating_sub(stem.as_ptr() as usize)
}

/// Words merge into one `_{w1,w2}` head when all start with `_` and not all of what
/// follows is digits, which would read as a number set.
fn product<'a>(mut words: impl Iterator<Item = &'a str> + Clone) -> bool {
    words.clone().all(|w| w.starts_with('_'))
        && !words.all(|w| w.len() > 1 && w.bytes().skip(1).all(|b| b.is_ascii_digit()))
}

/// The second tokens of the run at `start` as group items, joined by `,`, and how many.
/// Two or more in a row with the same stem and rising digit tails become `stem{numset}`.
fn items(toks: &[&str], start: usize, count: usize) -> (String, usize) {
    let suf = |k: usize| toks.get(start + 1 + 2 * k).copied().unwrap_or("");
    let mut out = String::new();
    let mut n = 0;
    let mut k = 0;
    while k < count {
        if n > 0 {
            out.push(SEP);
        }
        n += 1;
        let s = suf(k);
        let Some((stem, d)) = digit_tail(s) else {
            out.push_str(s);
            k += 1;
            continue;
        };
        let mut nums = vec![d];
        let mut j = k + 1;
        while j < count
            && let Some((st, d2)) = digit_tail(suf(j))
            && st == stem
            && nums.last().is_some_and(|&l| d2 > l)
        {
            nums.push(d2);
            j += 1;
        }
        if nums.len() == 1 {
            out.push_str(s);
        } else {
            out.push_str(stem);
            out.push(GROUP_OPEN);
            numset(&nums, &mut out);
            out.push(GROUP_CLOSE);
        }
        k = j;
    }
    (out, n)
}

/// A non-empty stem and the 1 to 9 ASCII digits ending `tok`.
fn digit_tail(tok: &str) -> Option<(&str, &str)> {
    let i = tok.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let d = tok.len() - i;
    (i > 0 && (1..=9).contains(&d)).then(|| tok.split_at(i))
}

/// Ranges, by width (natural numbers apart from each zero-padded width), in order of
/// value; two in a row are written `a,b`.
fn numset(nums: &[&str], out: &mut String) {
    let mut v: Vec<(usize, u32)> = nums
        .iter()
        .map(|d| {
            let w = if *d == "0" || !d.starts_with('0') {
                0
            } else {
                d.len()
            };
            (w, d.bytes().fold(0, |n, b| n * 10 + u32::from(b - b'0')))
        })
        .collect();
    v.sort_unstable();
    // (lo, width, hi)
    let mut runs: Vec<(u32, usize, u32)> = Vec::new();
    for (w, x) in v {
        match runs.last_mut() {
            Some(r) if r.1 == w && r.2 + 1 == x => r.2 = x,
            _ => runs.push((x, w, x)),
        }
    }
    runs.sort_unstable();
    for (k, &(lo, w, hi)) in runs.iter().enumerate() {
        if k > 0 {
            out.push(SEP);
        }
        number(out, lo, w);
        if hi == lo + 1 {
            out.push(SEP);
        } else if hi > lo {
            out.push(RANGE);
        }
        if hi > lo {
            number(out, hi, w);
        }
    }
}

fn number(out: &mut String, v: u32, width: usize) {
    use core::fmt::Write;
    let _ = write!(out, "{v:0width$}");
}
