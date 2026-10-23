//! A block's body as postfix [`Op`]s, which [`decode`](crate::decode) runs twice: once
//! for the size, once for the name.

use crate::{CLOSE, FoldError, GROUP_CLOSE, GROUP_OPEN, RANGE, SEP, reserved};

/// Each one leaves one element list on the stack.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Op {
    /// One element, `s[a..b]`.
    Lit(usize, usize),
    /// A number set, `s[a..b]` between its braces.
    Num(usize, usize),
    /// The top `n` lists, one after another.
    Cat(usize),
    /// Each element under the top followed by each element of the top.
    Mul,
}

/// The seq at `*i`, up to the `,`, `}` or `»` after it.
pub(crate) fn seq(s: &str, i: &mut usize, depth: u32, ops: &mut Vec<Op>) -> Result<(), FoldError> {
    let mut n = 0;
    while !matches!(peek(s, *i), None | Some(SEP | GROUP_CLOSE | CLOSE)) {
        term(s, i, depth, ops)?;
        n += 1;
    }
    match n {
        0 => ops.push(Op::Lit(*i, *i)),
        1 => {}
        n => ops.push(Op::Cat(n)),
    }
    Ok(())
}

fn term(s: &str, i: &mut usize, depth: u32, ops: &mut Vec<Op>) -> Result<(), FoldError> {
    let start = *i;
    while let Some(c) = peek(s, *i)
        && !reserved(c)
    {
        *i += c.len_utf8();
    }
    if *i == start && peek(s, *i) != Some(GROUP_OPEN) {
        return Err(FoldError::Syntax);
    }
    ops.push(Op::Lit(start, *i));
    while peek(s, *i) == Some(GROUP_OPEN) {
        group(s, i, depth, ops)?;
        ops.push(Op::Mul);
    }
    Ok(())
}

/// A number set when the content is one, else alternatives.
fn group(s: &str, i: &mut usize, depth: u32, ops: &mut Vec<Op>) -> Result<(), FoldError> {
    if depth >= 8 {
        return Err(FoldError::TooDeep);
    }
    *i += GROUP_OPEN.len_utf8();
    if let Some(end) = numset(s, *i) {
        ops.push(Op::Num(*i, end));
        *i = end + GROUP_CLOSE.len_utf8();
        return Ok(());
    }
    let mut n = 1;
    seq(s, i, depth + 1, ops)?;
    while peek(s, *i) == Some(SEP) {
        *i += SEP.len_utf8();
        seq(s, i, depth + 1, ops)?;
        n += 1;
    }
    if peek(s, *i) != Some(GROUP_CLOSE) {
        return Err(FoldError::Syntax);
    }
    *i += GROUP_CLOSE.len_utf8();
    if n > 1 {
        ops.push(Op::Cat(n));
    }
    Ok(())
}

/// Where the `}` of the number set starting at `i` is, if it is one.
fn numset(s: &str, mut i: usize) -> Option<usize> {
    loop {
        i = digits(s, i)?;
        if peek(s, i) == Some(RANGE) {
            i = digits(s, i + RANGE.len_utf8())?;
        }
        match peek(s, i)? {
            SEP => i += SEP.len_utf8(),
            GROUP_CLOSE => return Some(i),
            _ => return None,
        }
    }
}

/// The end of one or more ASCII digits at `i`.
fn digits(s: &str, i: usize) -> Option<usize> {
    let n = s
        .as_bytes()
        .get(i..)?
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    (n > 0).then_some(i + n)
}

pub(crate) fn peek(s: &str, i: usize) -> Option<char> {
    s.get(i..)?.chars().next()
}
