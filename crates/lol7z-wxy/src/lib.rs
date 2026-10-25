//! 7-Zip plugin for Wooxy skin packages (`*.wxy`), read-only.
//!
//! A package lists as the mod folder cslol-manager imports it to: every game file under
//! `RAW/<stored path>`, the package's name, author and version as `META/info.json`, and
//! its preview images as `META/image.png`, `META/image1.jpg`, and so on. Paths are
//! shown as stored, with `/` separators. Reads every version the reference extractor
//! does, 1 through 7.

mod archive;
mod decode;
mod format;
pub mod toc;

pub use archive::{Item, WxyArchive};
pub use format::Wxy;
