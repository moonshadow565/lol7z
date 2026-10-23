//! A file name too long for the file system, folded into one block of a brace-expansion
//! notation and back. The block is a template: each group element is substituted once
//! and the expansions are concatenated with nothing between them, so
//! `sightward_multi«_skins_skin{0…29,32…39}».bin` reads `_skins_skin{N}` once for every
//! N, in bytewise order, giving `sightward_multi_skins_skin0_skins_skin1_skins_skin10`
//! … `_skins_skin39.bin`. A group whose content is a number set (`{0…29,32,40}`) lists
//! those numbers as strings, sorted bytewise; any other group lists its comma-separated
//! alternatives in the order written, and a term `L{a,b}{c,d}` gives `LacLadLbcLbd`.
//!
//! ```text
//! name    = outer "«" seq "»" outer
//! seq     = { term }                      an empty seq is one empty element
//! term    = lit | [lit] group { group }
//! group   = "{" numset "}" | "{" seq { "," seq } "}"
//! numset  = nitem { "," nitem }
//! nitem   = digits [ "…" digits ]         equal widths keep zero padding
//! digits  = 1*"0".."9"                    a group that parses as a numset is one; over 9 is an error
//! lit     = 1*( char - ( "{" | "}" | "," | "«" | "»" | "…" ) )
//! ```

mod decode;
mod encode;
mod error;
mod parse;
mod split;

pub use decode::decode;
pub use encode::encode;
pub use error::FoldError;
pub use split::{Split, split};

/// Opens the one block in a name.
pub const OPEN: char = '«';
/// Closes the block.
pub const CLOSE: char = '»';
pub const GROUP_OPEN: char = '{';
pub const GROUP_CLOSE: char = '}';
/// Between a group's items.
pub const SEP: char = ',';
/// Between a range's ends, inside a number set.
pub const RANGE: char = '…';

/// Longest name [`decode`] gives, in bytes: a guard against hostile names only.
pub const MAX_DECODED: u64 = 1 << 16;

/// Characters a literal can't hold.
fn reserved(c: char) -> bool {
    matches!(c, GROUP_OPEN | GROUP_CLOSE | SEP | OPEN | CLOSE | RANGE)
}
