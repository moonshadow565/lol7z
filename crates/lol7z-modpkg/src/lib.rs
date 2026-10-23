//! 7-Zip plugin for LeagueToolkit mod packages (`*.modpkg`).
//!
//! A content chunk lists as `content/<priority>-<layer>/<WAD>/<stored path>`, without
//! the WAD part when it has none; a meta chunk as its stored path under `_meta_/`; and
//! every layer as a folder, so an empty one survives a round trip. Characters Windows
//! can't store and over-long names are shown as the WAD plugin shows them. Adding files
//! reads those names back: the layer folder gives the layer and its priority, a folder
//! named `*.wad.client` or `*.wad` under it is the WAD target, and a WAD-root file whose
//! stem is 16 hex digits is a hex-named chunk. The metadata document is carried through
//! as it is; a package written without one gets a minimal one.
//!
//! `-mx=N` picks the zstd level (0 stores); `.bnk` and `.wpk` are stored raw. Reads and
//! writes format version 1.

mod archive;
pub mod config;
mod decode;
mod format;
pub mod meta;
pub mod path;
pub mod toc;
mod update;
mod writer;

pub use archive::{Item, ModpkgArchive};
pub use config::Config;
pub use format::Modpkg;
pub use meta::Metadata;
