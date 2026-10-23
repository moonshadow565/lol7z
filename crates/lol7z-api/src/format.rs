//! The [`Format`] trait: what 7-Zip lists, and how to open it.

use crate::archive::Archive;
use crate::error::Result;
use crate::in_stream::InStream;
use crate::open::OpenCallback;
use crate::options::{Opt, Options};
use lol7z_sys::i_archive::{ArcInfoFlags, ArcTimeFlags, IsArcResult};
use windows_core::GUID;

/// An archive format: what 7-Zip lists, and how to open it.
///
/// Implement it on a marker type and register that type with [`formats!`](crate::formats).
pub trait Format: 'static {
    /// Shown in 7-Zip's format list (`7z i`) and selected with `7z -t<NAME>`.
    const NAME: &'static str;
    /// Identifies the format to 7-Zip. Generate a new GUID for each format; 7-Zip's own
    /// formats use `23170F69-40C1-278A-1000-000110xx0000`.
    const CLASS_ID: GUID;
    /// The file extensions that pick this format. The first one names new archives.
    const EXTENSIONS: &'static [Ext];
    /// Byte strings at [`SIGNATURE_OFFSET`](Self::SIGNATURE_OFFSET) that identify the
    /// format, any one of which matches. 7-Zip uses them to pick a handler when the
    /// extension doesn't match. With more than one, each must be under 256 bytes.
    const SIGNATURES: &'static [&'static [u8]] = &[];
    const SIGNATURE_OFFSET: u32 = 0;
    const FLAGS: ArcInfoFlags = ArcInfoFlags::empty();
    /// Timestamp precisions the format stores. Empty: not reported.
    const TIME_FLAGS: ArcTimeFlags = ArcTimeFlags::new();

    type Archive: Archive;

    /// Parses the archive.
    ///
    /// - Not this format: return [`Error::NotArchive`](crate::Error::NotArchive), and
    ///   7-Zip tries other formats.
    /// - Damaged, but some items are readable: record the damage with
    ///   [`OpenCallback::error`] and return what you read. 7-Zip lists those items and
    ///   shows the error, as it does for a truncated zip.
    /// - Damaged and unusable: return [`Error::Damaged`](crate::Error::Damaged). 7-Zip
    ///   reports the archive as broken instead of trying other formats. `?` on a short
    ///   read gives `Damaged(UnexpectedEnd)`.
    ///
    /// The stream is at position 0. Keep it (it's cheap to clone) to read item data
    /// later. Validate sizes against [`InStream::size`] before allocating: running out of
    /// memory aborts the host process.
    fn open(stream: InStream, cb: &OpenCallback<'_>, options: &Options) -> Result<Self::Archive>;

    /// A quick check of the file's first bytes, before 7-Zip calls `open`. The default
    /// compares [`SIGNATURES`](Self::SIGNATURES), and says yes when there are none.
    fn is_arc(head: &[u8]) -> IsArcResult {
        if Self::SIGNATURES.is_empty() {
            return IsArcResult::Yes;
        }
        let start = Self::SIGNATURE_OFFSET as usize;
        let mut need_more = false;
        for sig in Self::SIGNATURES {
            match head.get(start..start.saturating_add(sig.len())) {
                Some(got) if got == *sig => return IsArcResult::Yes,
                Some(_) => {}
                None => need_more = true,
            }
        }
        if need_more {
            IsArcResult::NeedMore
        } else {
            IsArcResult::No
        }
    }

    /// Checks one `-m` switch when 7-Zip passes it. Return
    /// [`Error::INVALID_ARG`](crate::Error::INVALID_ARG) for a name or value the format
    /// doesn't take, and 7-Zip stops with an error instead of ignoring it. The default
    /// accepts everything, which is what 7-Zip does for formats without options.
    fn check_option(_opt: &Opt) -> Result<()> {
        Ok(())
    }
}

/// One file extension of a [`Format`].
///
/// 7-Zip compares only what follows the last dot of a file name, so `X.wad.client`
/// matches `client`, never `wad.client`. [`formats!`](crate::formats) fails to compile
/// for an extension with a dot or a space in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ext {
    pub ext: &'static str,
    /// For formats that wrap one unnamed file: the extension that file gets in place of
    /// this one, as `x.tgz` extracts to `x.tar`.
    pub inner: Option<&'static str>,
}

impl Ext {
    pub const fn new(ext: &'static str) -> Self {
        Self { ext, inner: None }
    }

    /// `inner` includes its dot: `Ext::wrapping("tgz", ".tar")`.
    pub const fn wrapping(ext: &'static str, inner: &'static str) -> Self {
        Self {
            ext,
            inner: Some(inner),
        }
    }
}

/// Compile-time checks of a format's constants, run by [`formats!`](crate::formats).
pub(crate) const fn check<F: Format>() {
    assert!(!F::EXTENSIONS.is_empty(), "a format needs an extension");
    let mut i = 0;
    while i < F::EXTENSIONS.len() {
        let ext = F::EXTENSIONS[i].ext.as_bytes();
        assert!(!ext.is_empty(), "empty extension");
        let mut j = 0;
        while j < ext.len() {
            assert!(
                ext[j] != b'.',
                "7-Zip matches only the text after the last dot: drop the dots"
            );
            assert!(ext[j] != b' ', "an extension can't contain a space");
            j += 1;
        }
        i += 1;
    }
    if F::SIGNATURES.len() > 1 {
        let mut i = 0;
        while i < F::SIGNATURES.len() {
            assert!(F::SIGNATURES[i].len() < 256, "a signature is too long");
            i += 1;
        }
    }
}
