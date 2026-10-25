//! 7-Zip plugin for Riot Archive Files (`*.raf`), the RADS archives of League of Legends
//! before WADs, read-only.
//!
//! A `.raf` is only the index; the data sits in `<name>.raf.dat` next to it, which 7-Zip
//! opens as a second volume. Entries list under their stored game paths (`DATA/...`,
//! `LEVELS/...`). A compressed entry shows no size until it's extracted, since zlib
//! doesn't store one.
//!
//! Which entries are compressed is recorded nowhere in the archive: it's the deploy mode
//! in the release manifest (`releases/<ver>/releasemanifest`), 22 for compressed and 6
//! for stored, which 7-Zip doesn't have. So each entry's first two bytes are checked for
//! a zlib header instead. Over every RADS release manifest ever published, the stored
//! files are WAD clients, FMOD banks (`.fsb`, `.fev`), Wwise banks (`.bnk`), Scaleform
//! `.gfx` and four `.xml`, and none of their magics (`RW`, `FSB`, `FEV`, `BKHD`, `CFX`,
//! `<`) can begin a zlib header, so the check can't mistake them. On the real archives
//! checked, it agreed with the manifest on every entry.

mod archive;
mod decode;
mod format;
pub mod toc;

pub use archive::{Method, RafArchive};
pub use decode::is_zlib;
pub use format::Raf;
