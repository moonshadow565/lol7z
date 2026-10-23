//! [`decode`]: a folded name back to the name.

use crate::parse::{self, Op};
use crate::{CLOSE, FoldError, MAX_DECODED, OPEN, RANGE, SEP};

/// The name `name` folds; `name` itself when it has no block. The size is worked out
/// before anything is expanded.
pub fn decode(name: &str) -> Result<String, FoldError> {
    let Some(open) = name.find(OPEN) else {
        if name.contains(CLOSE) {
            return Err(FoldError::Blocks);
        }
        let mut out = String::new();
        out.try_reserve_exact(name.len())?;
        out.push_str(name);
        return Ok(out);
    };
    let pre = &name[..open];
    if pre.contains(CLOSE) {
        return Err(FoldError::Blocks);
    }
    let mut i = open + OPEN.len_utf8();
    let mut ops = Vec::new();
    // At most two ops per character, one more for an empty body.
    ops.try_reserve(2 * (name.len() - i) + 1)?;
    parse::seq(name, &mut i, 0, &mut ops)?;
    if parse::peek(name, i) != Some(CLOSE) {
        return Err(FoldError::Syntax);
    }
    let post = &name[i + CLOSE.len_utf8()..];
    if post.contains([OPEN, CLOSE]) {
        return Err(FoldError::Blocks);
    }
    let total = size(name, &ops)?
        .checked_add((pre.len() + post.len()) as u64)
        .filter(|&t| t <= MAX_DECODED)
        .ok_or(FoldError::TooLarge)?;
    let mut out = Vec::new();
    out.try_reserve_exact(total as usize)?;
    out.extend_from_slice(pre.as_bytes());
    expand(name, &ops, &mut out)?;
    out.extend_from_slice(post.as_bytes());
    String::from_utf8(out).map_err(|_| FoldError::Syntax)
}

/// The bytes the block expands to, checking every list's element count and bytes on
/// the way.
fn size(s: &str, ops: &[Op]) -> Result<u64, FoldError> {
    let mut stack: Vec<(u64, u64)> = Vec::new();
    stack.try_reserve(ops.len())?;
    for &op in ops {
        let (c, t) = match op {
            Op::Lit(a, b) => (1, b.saturating_sub(a) as u64),
            Op::Num(a, b) => num_size(s.get(a..b).ok_or(FoldError::Syntax)?)?,
            Op::Cat(n) => {
                let k = stack.len().checked_sub(n).ok_or(FoldError::Syntax)?;
                let sum = stack
                    .get(k..)
                    .unwrap_or(&[])
                    .iter()
                    .try_fold((0u64, 0u64), |(c, t), &(x, y)| {
                        Some((c.checked_add(x)?, t.checked_add(y)?))
                    });
                stack.truncate(k);
                sum.ok_or(FoldError::TooLarge)?
            }
            Op::Mul => {
                let (Some((gc, gt)), Some((c, t))) = (stack.pop(), stack.pop()) else {
                    return Err(FoldError::Syntax);
                };
                // Each of the c elements gets each of the gc: t is repeated gc times, gt c times.
                c.checked_mul(gc)
                    .zip(
                        t.checked_mul(gc)
                            .zip(gt.checked_mul(c))
                            .and_then(|(x, y)| x.checked_add(y)),
                    )
                    .ok_or(FoldError::TooLarge)?
            }
        };
        if c > MAX_DECODED || t > MAX_DECODED {
            return Err(FoldError::TooLarge);
        }
        stack.push((c, t));
    }
    match stack[..] {
        [(_, t)] => Ok(t),
        _ => Err(FoldError::Syntax),
    }
}

/// A number set's element count and bytes, duplicates counted.
fn num_size(set: &str) -> Result<(u64, u64), FoldError> {
    let (mut c, mut t) = (0u64, 0u64);
    for it in set.split(SEP) {
        let (lo, hi, width) = item(it)?;
        let n = hi - lo + 1;
        let bytes = if width > 0 {
            n * width as u64
        } else {
            natural_bytes(lo, hi)
        };
        c += n;
        t += bytes;
        if c > MAX_DECODED || t > MAX_DECODED {
            return Err(FoldError::TooLarge);
        }
    }
    Ok((c, t))
}

/// An item's numbers: `lo..=hi`, zero-padded to `width`, or as they are for width 0.
fn item(it: &str) -> Result<(u64, u64, usize), FoldError> {
    let (a, b) = it.split_once(RANGE).unwrap_or((it, it));
    if a.len() > 9 || b.len() > 9 {
        return Err(FoldError::BadRange);
    }
    let width = if a.len() == b.len() {
        a.len()
    } else if a.len() < b.len() && (a == "0" || !a.starts_with('0')) && !b.starts_with('0') {
        0
    } else {
        return Err(FoldError::BadRange);
    };
    let (lo, hi) = (number(a), number(b));
    if lo > hi {
        return Err(FoldError::BadRange);
    }
    Ok((lo, hi, width))
}

/// Up to 9 ASCII digits.
fn number(d: &str) -> u64 {
    d.bytes()
        .fold(0, |n, b| n * 10 + u64::from(b.wrapping_sub(b'0') % 10))
}

/// Bytes of `lo..=hi` written without padding, one digit count at a time.
fn natural_bytes(lo: u64, hi: u64) -> u64 {
    let (mut x, mut d, mut t) = (lo, 1u32, 0u64);
    while 10u64.pow(d) <= x {
        d += 1;
    }
    loop {
        let end = hi.min(10u64.pow(d) - 1);
        t += (end - x + 1) * u64::from(d);
        if end == hi {
            return t;
        }
        x = end + 1;
        d += 1;
    }
}

/// Runs `ops` again, writing elements into `out`. Lists sit on a stack one after another,
/// each a run of elements; `ends` holds where each element ends.
fn expand(s: &str, ops: &[Op], out: &mut Vec<u8>) -> Result<(), FoldError> {
    let base = out.len();
    let mut ends: Vec<usize> = Vec::new();
    let mut lists: Vec<usize> = Vec::new();
    lists.try_reserve(ops.len())?;
    let (mut tmp, mut tmp_ends) = (Vec::new(), Vec::new());
    let start = |ends: &[usize], e: usize| match e.checked_sub(1) {
        None => Some(base),
        Some(p) => ends.get(p).copied(),
    };
    for &op in ops {
        match op {
            Op::Lit(a, b) => {
                let lit = s.get(a..b).ok_or(FoldError::Syntax)?;
                lists.push(ends.len());
                out.try_reserve(lit.len())?;
                out.extend_from_slice(lit.as_bytes());
                ends.try_reserve(1)?;
                ends.push(out.len());
            }
            Op::Num(a, b) => {
                lists.push(ends.len());
                numbers(s.get(a..b).ok_or(FoldError::Syntax)?, out, &mut ends)?;
            }
            Op::Cat(n) => {
                let k = lists.len().checked_sub(n.saturating_sub(1));
                lists.truncate(k.ok_or(FoldError::Syntax)?);
            }
            Op::Mul => {
                let (Some(g), Some(&a)) = (lists.pop(), lists.last()) else {
                    return Err(FoldError::Syntax);
                };
                let (Some(xs), Some(ys)) = (ends.get(a..g), ends.get(g..)) else {
                    return Err(FoldError::Syntax);
                };
                let (from, mid) = start(&ends, a)
                    .zip(start(&ends, g))
                    .ok_or(FoldError::Syntax)?;
                let bytes = (mid - from) * ys.len() + (out.len() - mid) * xs.len();
                tmp.clear();
                tmp.try_reserve(bytes)?;
                tmp_ends.clear();
                tmp_ends.try_reserve(xs.len() * ys.len())?;
                let mut x0 = from;
                for &x1 in xs {
                    let x = out.get(x0..x1).ok_or(FoldError::Syntax)?;
                    let mut y0 = mid;
                    for &y1 in ys {
                        tmp.extend_from_slice(x);
                        tmp.extend_from_slice(out.get(y0..y1).ok_or(FoldError::Syntax)?);
                        tmp_ends.push(from + tmp.len());
                        y0 = y1;
                    }
                    x0 = x1;
                }
                out.truncate(from);
                out.try_reserve(tmp.len())?;
                out.extend_from_slice(&tmp);
                ends.truncate(a);
                ends.try_reserve(tmp_ends.len())?;
                ends.extend_from_slice(&tmp_ends);
            }
        }
    }
    if lists.len() == 1 {
        Ok(())
    } else {
        Err(FoldError::Syntax)
    }
}

/// A number set's elements: every number of every item as a string, sorted bytewise,
/// each once.
fn numbers(set: &str, out: &mut Vec<u8>, ends: &mut Vec<usize>) -> Result<(), FoldError> {
    let (count, bytes) = num_size(set)?;
    let mut tmp = Vec::new();
    tmp.try_reserve_exact(bytes as usize)?;
    let mut spans: Vec<(usize, usize)> = Vec::new();
    spans.try_reserve_exact(count as usize)?;
    for it in set.split(SEP) {
        let (lo, hi, width) = item(it)?;
        for v in lo..=hi {
            let a = tmp.len();
            write_number(&mut tmp, v, width);
            spans.push((a, tmp.len()));
        }
    }
    spans.sort_unstable_by(|x, y| tmp[x.0..x.1].cmp(&tmp[y.0..y.1]));
    spans.dedup_by(|x, y| tmp[x.0..x.1] == tmp[y.0..y.1]);
    out.try_reserve(tmp.len())?;
    ends.try_reserve(spans.len())?;
    for (a, b) in spans {
        out.extend_from_slice(&tmp[a..b]);
        ends.push(out.len());
    }
    Ok(())
}

/// `v` in decimal, zero-padded to `width`.
fn write_number(out: &mut Vec<u8>, mut v: u64, width: usize) {
    let mut d = [b'0'; 20];
    let mut n = 0;
    while n < d.len() && (v > 0 || n == 0 || n < width) {
        d[d.len() - 1 - n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    out.extend_from_slice(&d[d.len() - n..]);
}
