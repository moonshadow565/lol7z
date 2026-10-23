//! Safe API for writing 7-Zip format plugins, on top of [`lol7z_sys`].
//!
//! A format implements three traits:
//! - [`Format`]: name, class id, extensions, signature, and `open`.
//! - [`Archive`]: the opened archive: item count, item properties, item data.
//! - [`Update`] (optional): writing new archives.
//!
//! [`formats!`] then exports them from a `cdylib` that 7-Zip loads from its `Formats`
//! folder. See `examples/toy.rs` for a complete format.
//!
//! 7-Zip's callbacks are wrapped as [`OpenCallback`], [`ExtractCallback`] and
//! [`UpdateCallback`], and its streams as `std::io` types ([`InStream`], [`OutStream`]).
//! Errors are [`Error`], an `HRESULT`.
//!
//! The COM layer doesn't catch panics: a panic that reaches 7-Zip aborts the process.

mod alloc;
mod archive;
mod com_stream;
mod error;
#[doc(hidden)]
pub mod export;
mod extract;
mod format;
mod handler;
mod in_stream;
mod open;
mod options;
mod out_stream;
mod progress;
mod prop;
mod prop_info;
mod raw_prop;
mod update;

pub use alloc::TryAlloc;
pub use archive::Archive;
pub use com_stream::ItemStream;
pub use error::{Error, Result};
pub use extract::{ExtractCallback, Indices, ItemOut};
pub use format::{Ext, Format};
pub use in_stream::{InStream, SeqInStream, Slice};
pub use open::{OpenCallback, OpenReport, Volumes};
pub use options::{Opt, Options};
pub use out_stream::OutStream;
pub use progress::{Progress, ProgressReader, ProgressWriter};
pub use prop::Prop;
pub use prop_info::{PropInfo, standard_vt};
pub use raw_prop::{RawProp, RawPropInfo};
pub use update::{Update, UpdateCallback, UpdateItem};

pub use lol7z_sys as sys;
pub use lol7z_sys::i_archive::{
    ArcInfoFlags, ArcTimeFlags, AskMode, EventIndexType, FileTimeType, IsArcResult,
    OperationResult, ParentType, UpdateNotifyOp,
};
pub use lol7z_sys::propid::{ErrorFlags, PropId};
pub use windows_core::{GUID, HRESULT, OutRef};
