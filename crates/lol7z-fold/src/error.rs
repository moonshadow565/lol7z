//! [`FoldError`]: why a name doesn't decode.

use core::fmt;
use std::collections::TryReserveError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldError {
    /// Not the grammar: an unbalanced brace, a reserved character out of place.
    Syntax,
    /// A `»` without its `«`, or a second block.
    Blocks,
    /// Groups nested deeper than 8.
    TooDeep,
    /// A number longer than 9 digits, mixed widths, or a range that runs backwards.
    BadRange,
    /// Longer than [`MAX_DECODED`](crate::MAX_DECODED).
    TooLarge,
    OutOfMemory,
}

impl fmt::Display for FoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Syntax => "not a folded name",
            Self::Blocks => "not exactly one block",
            Self::TooDeep => "groups nested too deep",
            Self::BadRange => "bad number range",
            Self::TooLarge => "expands too far",
            Self::OutOfMemory => "out of memory",
        })
    }
}

impl std::error::Error for FoldError {}

impl From<TryReserveError> for FoldError {
    fn from(_: TryReserveError) -> Self {
        Self::OutOfMemory
    }
}
