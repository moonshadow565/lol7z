//! The [`Archive`] trait: an opened archive.

use crate::com_stream::ItemStream;
use crate::error::{Error, Result};
use crate::extract::{ExtractCallback, Indices};
use crate::prop::Prop;
use crate::prop_info::PropInfo;
use crate::raw_prop::{RawProp, RawPropInfo};
use lol7z_sys::i_archive::{OperationResult, ParentType};
use lol7z_sys::propid::PropId;
use std::io::Write;

/// An opened archive.
///
/// Every `index` passed in is below [`item_count`](Self::item_count): the COM layer
/// checks first.
///
/// Methods take `&self` because 7-Zip calls back into the archive while it's
/// extracting (to name each output file). Keep what `open` parsed immutable; put any
/// mutable state behind a `RefCell` borrowed only briefly, never across a callback.
///
/// 7-Zip uses the archive from one thread at a time, but not always the same thread
/// (its file manager lists on one thread and extracts on another). Don't keep
/// thread-local state in it. There is no `Send` bound because [`InStream`](crate::InStream),
/// which most archives hold, wraps a COM pointer and can't be `Send`: 7-Zip's reference
/// counts aren't atomic.
pub trait Archive: 'static {
    /// The item properties [`item_prop`](Self::item_prop) can return, in the order
    /// 7-Zip should show them.
    const ITEM_PROPS: &'static [PropInfo];
    const ARCHIVE_PROPS: &'static [PropInfo] = &[];

    fn item_count(&self) -> u32;

    /// `None`: no value for this item.
    fn item_prop(&self, index: u32, id: PropId) -> Option<Prop<'_>>;

    /// Properties of the archive as a whole, such as `PhySize`.
    fn archive_prop(&self, _id: PropId) -> Option<Prop<'_>> {
        None
    }

    /// Used by the default [`extract`](Self::extract). Reads the `IsDir` property.
    fn is_dir(&self, index: u32) -> bool {
        self.item_prop(index, PropId::IsDir)
            .and_then(|p| p.as_bool())
            == Some(true)
    }

    /// Writes one item's data, for formats whose items decode independently. Used by the
    /// default [`extract`](Self::extract); formats that override `extract` can ignore it.
    ///
    /// A bad item doesn't stop extraction: return `Ok` with its result, or an
    /// [`Error::Damaged`] (which is what `?` on a short read gives), and the default
    /// `extract` reports it and moves on. Other errors stop the whole operation: the disk
    /// is full, the user cancelled, the archive file can't be read.
    fn extract_item(&self, _index: u32, _out: &mut dyn Write) -> Result<OperationResult> {
        Err(Error::NOT_IMPL)
    }

    /// Extracts or tests `items`. The default reports progress by the `Size` property
    /// and calls [`extract_item`](Self::extract_item) for each item that isn't a
    /// directory. Solid formats override it.
    fn extract(&self, items: Indices<'_>, cb: &ExtractCallback<'_>) -> Result<()> {
        let size = |i| {
            self.item_prop(i, PropId::Size)
                .and_then(|p| p.as_u64())
                .unwrap_or(0)
        };
        cb.set_total(items.iter().map(size).fold(0, u64::saturating_add))?;
        let mut done = 0u64;
        for index in items.iter() {
            cb.set_completed(done)?;
            if self.is_dir(index) {
                cb.empty_item(index)?;
                continue;
            }
            let mut out = cb.item(index)?;
            if out.is_wanted() {
                let r = {
                    let mut w = cb.progress().writer(&mut out, done);
                    self.extract_item(index, &mut w)
                };
                let result = match r {
                    Ok(r) => r,
                    Err(e) => match e.operation_result() {
                        Some(r) => r,
                        None => {
                            out.abandon();
                            return Err(e);
                        }
                    },
                };
                out.finish(result)?;
            }
            done = done.saturating_add(size(index));
        }
        cb.set_completed(done)
    }

    /// The item's data as a stream 7-Zip reads itself, so it can open an archive nested
    /// in this one without extracting it. `None` (the default): not available.
    fn item_stream(&self, _index: u32) -> Result<Option<ItemStream>> {
        Ok(None)
    }

    /// Raw properties 7-Zip can show as columns, read through
    /// [`raw_prop`](Self::raw_prop).
    const RAW_PROPS: &'static [RawPropInfo] = &[];

    /// A property 7-Zip reads in place, without a BSTR. 7-Zip asks for `Path` (or `Name`
    /// in a tree) this way first, and uses [`item_prop`](Self::item_prop) when this
    /// returns `None` (the default).
    fn raw_prop(&self, _index: u32, _id: PropId) -> Option<RawProp<'_>> {
        None
    }

    /// The item's parent, for archives stored as a tree. 7-Zip builds paths from
    /// parents and `Name`s instead of reading `Path` when [`archive_prop`](Self::archive_prop)
    /// returns `IsTree = true`. `None` (the default): a top-level item.
    ///
    /// `ParentType::AltStream` makes the item an alternate data stream of its parent.
    fn parent(&self, _index: u32) -> Option<(u32, ParentType)> {
        None
    }

    /// A property of the tree's root, which isn't an item (e.g. the root folder's
    /// security descriptor).
    fn root_prop(&self, _id: PropId) -> Option<Prop<'_>> {
        None
    }

    fn root_raw_prop(&self, _id: PropId) -> Option<RawProp<'_>> {
        None
    }
}
