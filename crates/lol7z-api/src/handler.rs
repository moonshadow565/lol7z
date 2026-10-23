//! The COM objects that serve a [`Format`] to 7-Zip.

use crate::archive::Archive;
use crate::error::{Error, Result, to_hresult};
use crate::extract::{ExtractCallback, Indices};
use crate::format::Format;
use crate::in_stream::InStream;
use crate::open::{OpenCallback, OpenReport};
use crate::options::{Opt, Options};
use crate::out_stream::OutStream;
use crate::prop::Prop;
use crate::raw_prop::RawProp;
use crate::update::{Update, UpdateCallback};
use core::cell::RefCell;
use core::ffi::c_void;
use core::marker::PhantomData;
use lol7z_sys::i_archive::{
    IArchiveExtractCallback, IArchiveGetRawProps, IArchiveGetRawProps_Impl, IArchiveGetRootProps,
    IArchiveGetRootProps_Impl, IArchiveOpenCallback, IArchiveUpdateCallback, IInArchive,
    IInArchive_Impl, IInArchiveGetStream, IInArchiveGetStream_Impl, IOutArchive, IOutArchive_Impl,
    ISetProperties, ISetProperties_Impl, ParentType,
};
use lol7z_sys::i_stream::{IInStream, ISequentialInStream, ISequentialOutStream};
use lol7z_sys::propid::{ErrorFlags, PropId};
use lol7z_sys::propvar::PropVariant;
use windows::Win32::Foundation::E_POINTER;
use windows_core::{BSTR, HRESULT, OutRef, PCWSTR, Ref, implement};

/// What both handler objects hold.
struct State<F: Format> {
    archive: RefCell<Option<F::Archive>>,
    /// From the last `Open`, kept even when it failed: 7-Zip reads it either way.
    report: RefCell<OpenReport>,
    options: RefCell<Options>,
    _format: PhantomData<F>,
}

impl<F: Format> State<F> {
    fn new() -> Self {
        Self {
            archive: RefCell::new(None),
            report: RefCell::default(),
            options: RefCell::default(),
            _format: PhantomData,
        }
    }

    fn open(&self, stream: Ref<'_, IInStream>, cb: Ref<'_, IArchiveOpenCallback>) -> Result<()> {
        self.close();
        let stream = InStream::from_com(stream.ok()?.clone());
        let cb = OpenCallback::new(cb.as_ref());
        let result = F::open(stream, &cb, &self.options.borrow());
        let mut report = cb.into_report();
        let result = match result {
            Ok(archive) => {
                *self.archive.borrow_mut() = Some(archive);
                Ok(())
            }
            // Not this format: nothing to report, and 7-Zip tries the next one.
            Err(Error::NotArchive) | Err(Error::Damaged(ErrorFlags::IsNotArc)) => {
                report = OpenReport::default();
                Err(Error::NotArchive)
            }
            // This format, but broken: S_FALSE with error flags that don't include
            // IsNotArc (OpenArchive.h:170), so 7-Zip reports it instead of moving on.
            Err(Error::Damaged(flags)) => {
                report.errors |= flags;
                report.errors -= ErrorFlags::IsNotArc;
                if report.errors.is_empty() {
                    report.errors = ErrorFlags::HeadersError;
                }
                Err(Error::NotArchive)
            }
            Err(e) => Err(e),
        };
        *self.report.borrow_mut() = report;
        result
    }

    /// Drops the archive outside the borrow, so its `Drop` can't collide with one.
    fn close(&self) {
        drop(self.archive.replace(None));
        self.report.replace(OpenReport::default());
    }

    fn with<T>(&self, f: impl FnOnce(&F::Archive) -> Result<T>) -> Result<T> {
        f(self.archive.borrow().as_ref().ok_or(Error::UNEXPECTED)?)
    }

    /// [`with`](Self::with) for one item. Checks `index` first, which is what lets
    /// [`Archive`] code index its tables directly.
    fn with_item<T>(&self, index: u32, f: impl FnOnce(&F::Archive) -> Result<T>) -> Result<T> {
        self.with(|a| {
            if index < a.item_count() {
                f(a)
            } else {
                Err(Error::INVALID_ARG)
            }
        })
    }

    fn item_count(&self) -> u32 {
        self.archive
            .borrow()
            .as_ref()
            .map_or(0, Archive::item_count)
    }

    fn item_prop(&self, index: u32, id: PropId, value: OutRef<'_, PropVariant>) -> Result<()> {
        self.with_item(index, |a| Prop::write_to(value, a.item_prop(index, id)))
    }

    /// 7-Zip asks for archive properties even after a failed `Open`: the report answers
    /// the error properties, and anything else is "no value".
    fn archive_prop(&self, id: PropId, value: OutRef<'_, PropVariant>) -> Result<()> {
        let report = self.report.borrow();
        let archive = self.archive.borrow();
        let v = report
            .prop(id)
            .or_else(|| archive.as_ref().and_then(|a| a.archive_prop(id)));
        Prop::write_to(value, v)
    }

    fn extract(
        &self,
        indices: *const u32,
        num_items: u32,
        test_mode: i32,
        cb: Ref<'_, IArchiveExtractCallback>,
    ) -> Result<()> {
        let cb = cb.ok()?;
        self.with(|a| {
            let count = a.item_count();
            let items = if num_items == u32::MAX {
                Indices::All(count)
            } else if num_items == 0 {
                Indices::List(&[])
            } else if indices.is_null() {
                return Err(Error::INVALID_ARG);
            } else {
                let list = unsafe { core::slice::from_raw_parts(indices, num_items as usize) };
                if list.iter().any(|&i| i >= count) {
                    return Err(Error::INVALID_ARG);
                }
                Indices::List(list)
            };
            a.extract(items, &ExtractCallback::new(cb, test_mode != 0))
        })
    }

    fn item_stream(&self, index: u32, stream: OutRef<'_, ISequentialInStream>) -> Result<()> {
        self.with_item(index, |a| {
            Ok(stream.write(a.item_stream(index)?.map(|s| s.0))?)
        })
    }

    fn set_options(
        &self,
        names: *const PCWSTR,
        values: *const PropVariant,
        num: u32,
    ) -> Result<()> {
        let values = unsafe { PropVariant::slice(values, num) };
        let names = if names.is_null() || num == 0 {
            &[][..]
        } else {
            unsafe { core::slice::from_raw_parts(names, num as usize) }
        };
        if names.len() != values.len() {
            return Err(Error::INVALID_ARG);
        }
        let opts = names
            .iter()
            .zip(values)
            .map(|(name, v)| {
                let name = if name.is_null() {
                    String::new()
                } else {
                    String::from_utf16_lossy(unsafe { name.as_wide() })
                };
                let value = Prop::from_variant(v);
                if value.is_none() && !v.is_empty() {
                    return Err(Error::INVALID_ARG);
                }
                let opt = Opt { name, value };
                F::check_option(&opt)?;
                Ok(opt)
            })
            .collect::<Result<_>>()?;
        *self.options.borrow_mut() = Options(opts);
        Ok(())
    }

    fn parent(&self, index: u32, parent: OutRef<'_, u32>, kind: OutRef<'_, u32>) -> Result<()> {
        self.with_item(index, |a| {
            let (p, t) = a.parent(index).unwrap_or((u32::MAX, ParentType::Dir));
            parent.write(p)?;
            Ok(kind.write(t.0)?)
        })
    }

    /// The pointer outlives the borrow, but not the data: it belongs to the archive,
    /// which stays put until `Close`, and 7-Zip doesn't use it after that.
    fn raw_prop(&self, index: u32, id: PropId, out: RawOut<'_>) -> Result<()> {
        self.with_item(index, |a| out.write(a.raw_prop(index, id)))
    }

    fn root_prop(&self, id: PropId, value: OutRef<'_, PropVariant>) -> Result<()> {
        let archive = self.archive.borrow();
        Prop::write_to(value, archive.as_ref().and_then(|a| a.root_prop(id)))
    }

    fn root_raw_prop(&self, id: PropId, out: RawOut<'_>) -> Result<()> {
        let archive = self.archive.borrow();
        out.write(archive.as_ref().and_then(|a| a.root_raw_prop(id)))
    }
}

/// The out-parameters of `GetRawProp` and `GetRootRawProp`.
struct RawOut<'a> {
    data: *mut *const c_void,
    size: OutRef<'a, u32>,
    kind: OutRef<'a, u32>,
}

impl<'a> RawOut<'a> {
    fn new(data: *mut *const c_void, size: OutRef<'a, u32>, kind: OutRef<'a, u32>) -> Self {
        Self { data, size, kind }
    }

    fn write(self, prop: Option<RawProp<'_>>) -> Result<()> {
        if self.data.is_null() {
            return Err(Error::Hresult(E_POINTER));
        }
        let (data, size, kind) = RawProp::to_raw(prop);
        unsafe { *self.data = data };
        self.size.write(size)?;
        Ok(self.kind.write(kind.bits())?)
    }
}

fn write_count(out: OutRef<'_, u32>, n: usize) -> HRESULT {
    to_hresult(out.write(n as u32))
}

fn at<T>(table: &[T], index: u32) -> Result<&T> {
    table.get(index as usize).ok_or(Error::INVALID_ARG)
}

/// A read-only handler. Every optional interface is there even when the format doesn't
/// use it: 7-Zip treats "nothing to report" the same as a missing interface.
#[implement(
    IInArchive,
    IInArchiveGetStream,
    ISetProperties,
    IArchiveGetRawProps,
    IArchiveGetRootProps
)]
pub(crate) struct Handler<F>
where
    F: Format,
{
    state: State<F>,
}

impl<F: Format> Handler<F> {
    pub(crate) fn new() -> Self {
        Self {
            state: State::new(),
        }
    }
}

/// [`Handler`] plus `IOutArchive`. A separate type because 7-Zip asks for `IOutArchive`
/// to decide whether a format can be written.
#[implement(
    IInArchive,
    IInArchiveGetStream,
    ISetProperties,
    IArchiveGetRawProps,
    IArchiveGetRootProps,
    IOutArchive
)]
pub(crate) struct UpdateHandler<F>
where
    F: Update,
{
    state: State<F>,
}

impl<F: Update> UpdateHandler<F> {
    pub(crate) fn new() -> Self {
        Self {
            state: State::new(),
        }
    }
}

macro_rules! impl_in_archive {
    ($handler:ident: $bound:ident) => {
        impl<F> IInArchive_Impl for $handler<F>
        where
            F: $bound,
        {
            unsafe fn Open(
                &self,
                stream: Ref<'_, IInStream>,
                _max_check_start_position: Option<&u64>,
                open_callback: Ref<'_, IArchiveOpenCallback>,
            ) -> HRESULT {
                to_hresult(self.state.open(stream, open_callback))
            }

            unsafe fn Close(&self) -> HRESULT {
                self.state.close();
                HRESULT(0)
            }

            unsafe fn GetNumberOfItems(&self, num_items: OutRef<'_, u32>) -> HRESULT {
                write_count(num_items, self.state.item_count() as usize)
            }

            unsafe fn GetProperty(
                &self,
                index: u32,
                prop_id: PropId,
                value: OutRef<'_, PropVariant>,
            ) -> HRESULT {
                to_hresult(self.state.item_prop(index, prop_id, value))
            }

            unsafe fn Extract(
                &self,
                indices: *const u32,
                num_items: u32,
                test_mode: i32,
                extract_callback: Ref<'_, IArchiveExtractCallback>,
            ) -> HRESULT {
                to_hresult(
                    self.state
                        .extract(indices, num_items, test_mode, extract_callback),
                )
            }

            unsafe fn GetArchiveProperty(
                &self,
                prop_id: PropId,
                value: OutRef<'_, PropVariant>,
            ) -> HRESULT {
                to_hresult(self.state.archive_prop(prop_id, value))
            }

            unsafe fn GetNumberOfProperties(&self, num_props: OutRef<'_, u32>) -> HRESULT {
                write_count(num_props, <F::Archive as Archive>::ITEM_PROPS.len())
            }

            unsafe fn GetPropertyInfo(
                &self,
                index: u32,
                name: OutRef<'_, BSTR>,
                prop_id: OutRef<'_, u32>,
                var_type: OutRef<'_, u16>,
            ) -> HRESULT {
                let table = <F::Archive as Archive>::ITEM_PROPS;
                to_hresult(at(table, index).and_then(|p| p.write_to(name, prop_id, var_type)))
            }

            unsafe fn GetNumberOfArchiveProperties(&self, num_props: OutRef<'_, u32>) -> HRESULT {
                write_count(num_props, <F::Archive as Archive>::ARCHIVE_PROPS.len())
            }

            unsafe fn GetArchivePropertyInfo(
                &self,
                index: u32,
                name: OutRef<'_, BSTR>,
                prop_id: OutRef<'_, u32>,
                var_type: OutRef<'_, u16>,
            ) -> HRESULT {
                let table = <F::Archive as Archive>::ARCHIVE_PROPS;
                to_hresult(at(table, index).and_then(|p| p.write_to(name, prop_id, var_type)))
            }
        }

        impl<F> IInArchiveGetStream_Impl for $handler<F>
        where
            F: $bound,
        {
            unsafe fn GetStream(
                &self,
                index: u32,
                stream: OutRef<'_, ISequentialInStream>,
            ) -> HRESULT {
                to_hresult(self.state.item_stream(index, stream))
            }
        }

        impl<F> IArchiveGetRawProps_Impl for $handler<F>
        where
            F: $bound,
        {
            unsafe fn GetParent(
                &self,
                index: u32,
                parent: OutRef<'_, u32>,
                parent_type: OutRef<'_, u32>,
            ) -> HRESULT {
                to_hresult(self.state.parent(index, parent, parent_type))
            }

            unsafe fn GetRawProp(
                &self,
                index: u32,
                prop_id: PropId,
                data: *mut *const c_void,
                data_size: OutRef<'_, u32>,
                prop_type: OutRef<'_, u32>,
            ) -> HRESULT {
                let out = RawOut::new(data, data_size, prop_type);
                to_hresult(self.state.raw_prop(index, prop_id, out))
            }

            unsafe fn GetNumRawProps(&self, num_props: OutRef<'_, u32>) -> HRESULT {
                write_count(num_props, <F::Archive as Archive>::RAW_PROPS.len())
            }

            unsafe fn GetRawPropInfo(
                &self,
                index: u32,
                name: OutRef<'_, BSTR>,
                prop_id: OutRef<'_, u32>,
            ) -> HRESULT {
                let table = <F::Archive as Archive>::RAW_PROPS;
                to_hresult(at(table, index).and_then(|p| p.write_to(name, prop_id)))
            }
        }

        impl<F> IArchiveGetRootProps_Impl for $handler<F>
        where
            F: $bound,
        {
            unsafe fn GetRootProp(
                &self,
                prop_id: PropId,
                value: OutRef<'_, PropVariant>,
            ) -> HRESULT {
                to_hresult(self.state.root_prop(prop_id, value))
            }

            unsafe fn GetRootRawProp(
                &self,
                prop_id: PropId,
                data: *mut *const c_void,
                data_size: OutRef<'_, u32>,
                prop_type: OutRef<'_, u32>,
            ) -> HRESULT {
                let out = RawOut::new(data, data_size, prop_type);
                to_hresult(self.state.root_raw_prop(prop_id, out))
            }
        }

        impl<F> ISetProperties_Impl for $handler<F>
        where
            F: $bound,
        {
            unsafe fn SetProperties(
                &self,
                names: *const PCWSTR,
                values: *const PropVariant,
                num_props: u32,
            ) -> HRESULT {
                to_hresult(self.state.set_options(names, values, num_props))
            }
        }
    };
}

impl_in_archive!(Handler_Impl: Format);
impl_in_archive!(UpdateHandler_Impl: Update);

impl<F> IOutArchive_Impl for UpdateHandler_Impl<F>
where
    F: Update,
{
    unsafe fn UpdateItems(
        &self,
        out_stream: Ref<'_, ISequentialOutStream>,
        num_items: u32,
        update_callback: Ref<'_, IArchiveUpdateCallback>,
    ) -> HRESULT {
        let run = || {
            let out = OutStream::from_com(out_stream.ok()?.clone());
            let cb = UpdateCallback::new(update_callback.ok()?, num_items);
            let archive = self.state.archive.borrow();
            F::update(archive.as_ref(), out, &cb, &self.state.options.borrow())
        };
        to_hresult(run())
    }

    unsafe fn GetFileTimeType(&self, r#type: OutRef<'_, u32>) -> HRESULT {
        to_hresult(r#type.write(F::FILE_TIME_TYPE.0 as u32))
    }
}
