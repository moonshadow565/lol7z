//! 7-Zip plugin for League of Legends WAD archives (`*.wad`, `*.wad.client`).
//!
//! Items are named by their paths in the Mimir hash tables (`game`, then `lcu`), or by
//! their path hash as `«H»`, 16 hex digits, when the tables don't know them. A last
//! component too long for Windows is folded (see [`lol7z_fold`]), or if still too long
//! shown as `preamble«FULL-OUTER».ext` around its hash. Adding a file reads those forms
//! back, and keeps the hash of a `«H»` or of a stem that is 16 hex digits; any other path
//! is hashed the way the game does. Reads versions 1 through 3.4, writes 3.4.
//!
//! `-mx=N` picks the zstd level (0 stores). Without it, zstd level 3.

mod archive;
mod builder;
mod decode;
pub mod encode;
mod format;
mod hash;
mod hasher;
mod names;
mod resolver;
pub mod toc;
mod update;
mod writer;

pub use archive::WadArchive;
pub use format::Wad;
pub use hash::PathHash;
pub use names::{Form, Names, Stats, display};
pub use resolver::Resolver;
pub use update::zstd_level;
