//! Plays 7-Zip's part against the toy format: creates the handler through the DLL
//! exports and calls it through the raw COM interfaces, with host callbacks written
//! here the way 7-Zip behaves.

#[path = "../examples/toy.rs"]
mod toy;

use core::cell::{Cell, RefCell};
use lol7z_api::export::{self, Entry, Formats};
use lol7z_api::sys::i_archive::*;
use lol7z_api::sys::i_progress::IProgress_Impl;
use lol7z_api::sys::i_stream::{IInStream, ISequentialInStream, ISequentialOutStream};
use lol7z_api::sys::propvar::PropVariant;
use lol7z_api::{
    ErrorFlags, Ext, Format, InStream, OpenCallback, Options, OutStream, PropId, Result,
    SeqInStream,
};
use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Write};
use std::rc::Rc;
use windows::Win32::Foundation::{E_ABORT, E_INVALIDARG, E_NOINTERFACE, S_FALSE};
use windows::Win32::System::Variant::VT_BSTR;
use windows_core::{BSTR, GUID, HRESULT, Interface, OutRef, PCWSTR, implement, w};

fn build(items: &[(&str, &[u8])]) -> Vec<u8> {
    let mut v = b"TOY\x01".to_vec();
    v.extend((items.len() as u32).to_le_bytes());
    for (name, data) in items {
        v.extend((name.len() as u16).to_le_bytes());
        v.extend(name.as_bytes());
        v.extend((data.len() as u32).to_le_bytes());
        v.extend(*data);
    }
    v
}

fn sample() -> Vec<u8> {
    build(&[
        ("a.txt", b"hello"),
        ("dir/", b""),
        ("dir/b.bin", b"\x00\x01\x02"),
    ])
}

fn create<T: Interface>() -> windows_core::Result<T> {
    let mut p = core::ptr::null_mut();
    unsafe { toy::CreateObject(&toy::Toy::CLASS_ID, &T::IID, &mut p) }.ok()?;
    Ok(unsafe { T::from_raw(p) })
}

fn open(bytes: Vec<u8>) -> IInArchive {
    let arc: IInArchive = create().unwrap();
    let hr = unsafe { arc.Open(InStream::new(Cursor::new(bytes)).as_com(), None, None) };
    assert_eq!(hr, HRESULT(0));
    arc
}

fn prop(arc: &IInArchive, index: u32, id: PropId) -> PropVariant {
    let mut v = PropVariant::default();
    unsafe { arc.GetProperty(index, id, &mut v) }.unwrap();
    v
}

fn path(arc: &IInArchive, index: u32) -> String {
    prop(arc, index, PropId::Path)
        .get::<BSTR>()
        .unwrap()
        .to_string()
}

/// Appends to a shared map entry, so the test can read what was extracted.
struct Sink(Rc<RefCell<BTreeMap<String, Vec<u8>>>>, String);

impl Write for Sink {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0
            .borrow_mut()
            .entry(self.1.clone())
            .or_default()
            .extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[implement(IArchiveExtractCallback)]
struct ExtractHost {
    arc: IInArchive,
    skip: Vec<u32>,
    /// Cancel: `SetCompleted` fails with `E_ABORT` once it has been called this often.
    abort_after: Option<u32>,
    completed: Cell<u32>,
    prepared_at: Cell<u32>,
    files: Rc<RefCell<BTreeMap<String, Vec<u8>>>>,
    log: Rc<RefCell<Vec<String>>>,
    /// `SetCompleted` calls between each item's PrepareOperation and its result.
    during: Rc<RefCell<Vec<u32>>>,
}

impl IProgress_Impl for ExtractHost_Impl {
    unsafe fn SetTotal(&self, total: u64) -> HRESULT {
        self.log.borrow_mut().push(format!("total {total}"));
        HRESULT(0)
    }
    unsafe fn SetCompleted(&self, _: Option<&u64>) -> HRESULT {
        let n = self.completed.get() + 1;
        self.completed.set(n);
        if self.abort_after.is_some_and(|a| n > a) {
            return E_ABORT;
        }
        HRESULT(0)
    }
}

impl IArchiveExtractCallback_Impl for ExtractHost_Impl {
    unsafe fn GetStream(
        &self,
        index: u32,
        out: OutRef<'_, ISequentialOutStream>,
        mode: AskMode,
    ) -> HRESULT {
        // What 7-Zip does here: read the item's path from the handler, re-entering it
        // while its Extract is running.
        let name = path(&self.arc, index);
        self.log.borrow_mut().push(format!("stream {name}"));
        if mode == AskMode::Extract && !self.skip.contains(&index) {
            let s = OutStream::new(Sink(self.files.clone(), name));
            out.write(Some(s.as_com().clone())).unwrap();
        }
        HRESULT(0)
    }
    unsafe fn PrepareOperation(&self, mode: AskMode) -> HRESULT {
        self.log.borrow_mut().push(format!("prepare {mode:?}"));
        self.prepared_at.set(self.completed.get());
        HRESULT(0)
    }
    unsafe fn SetOperationResult(&self, r: OperationResult) -> HRESULT {
        self.log.borrow_mut().push(format!("result {r:?}"));
        let n = self.completed.get() - self.prepared_at.get();
        self.during.borrow_mut().push(n);
        HRESULT(0)
    }
}

struct Extracted {
    hr: HRESULT,
    files: BTreeMap<String, Vec<u8>>,
    log: Vec<String>,
    during: Vec<u32>,
}

fn extract(arc: &IInArchive, indices: Option<&[u32]>, test: bool, skip: &[u32]) -> Extracted {
    extract_with(arc, indices, test, skip, None)
}

fn extract_with(
    arc: &IInArchive,
    indices: Option<&[u32]>,
    test: bool,
    skip: &[u32],
    abort_after: Option<u32>,
) -> Extracted {
    let host = ExtractHost {
        arc: arc.clone(),
        skip: skip.to_vec(),
        abort_after,
        completed: Cell::new(0),
        prepared_at: Cell::new(0),
        files: Rc::default(),
        log: Rc::default(),
        during: Rc::default(),
    };
    let (files, log, during) = (host.files.clone(), host.log.clone(), host.during.clone());
    let cb: IArchiveExtractCallback = host.into();
    let (ptr, n) = indices.map_or((core::ptr::null(), u32::MAX), |l| {
        (l.as_ptr(), l.len() as u32)
    });
    let hr = unsafe { arc.Extract(ptr, n, test as i32, &cb) };
    let files = files.borrow().clone();
    let log = log.borrow().clone();
    let during = during.borrow().clone();
    Extracted {
        hr,
        files,
        log,
        during,
    }
}

fn big() -> Vec<u8> {
    let data: Vec<u8> = (0..10 << 20).map(|i: u32| i as u8).collect();
    build(&[("big", &data)])
}

#[test]
fn progress_inside_large_item() {
    let arc = open(big());
    let r = extract(&arc, None, false, &[]);
    assert_eq!(r.hr, HRESULT(0));
    assert_eq!(r.files["big"].len(), 10 << 20);
    assert_eq!(r.during.len(), 1);
    assert!(
        r.during[0] >= 2,
        "SetCompleted calls inside the item: {:?}",
        r.during
    );
}

#[test]
fn cancel_mid_item() {
    let arc = open(big());
    // The first call comes before the item; the second, 4 MiB in, cancels.
    let r = extract_with(&arc, None, false, &[], Some(1));
    assert_eq!(r.hr, E_ABORT);
    assert!(r.files["big"].len() < 10 << 20);
    assert!(
        !r.log.iter().any(|l| l == "result DataError"),
        "{:?}",
        r.log
    );
    assert_eq!(r.log, ["total 10485760", "stream big", "prepare Extract"]);
}

#[test]
fn list() {
    let bytes = sample();
    let arc = open(bytes.clone());
    let mut n = 0;
    unsafe { arc.GetNumberOfItems(&mut n) }.unwrap();
    assert_eq!(n, 3);
    assert_eq!(path(&arc, 1), "dir");
    assert!(prop(&arc, 1, PropId::IsDir).get::<bool>().unwrap());
    assert_eq!(prop(&arc, 2, PropId::Size).get::<u64>().unwrap(), 3);
    assert!(prop(&arc, 0, PropId::MTime).is_empty());

    let mut phy = PropVariant::default();
    unsafe { arc.GetArchiveProperty(PropId::PhySize, &mut phy) }.unwrap();
    assert_eq!(phy.get::<u64>().unwrap(), bytes.len() as u64);

    unsafe { arc.GetNumberOfProperties(&mut n) }.unwrap();
    assert_eq!(n, 4);
    let (mut name, mut id, mut vt) = (BSTR::new(), 0, 0);
    unsafe { arc.GetPropertyInfo(0, &mut name, &mut id, &mut vt) }.unwrap();
    assert_eq!((name.is_empty(), id, vt), (true, PropId::Path.0, VT_BSTR.0));
    let hr = unsafe { arc.GetPropertyInfo(9, &mut name, &mut id, &mut vt) };
    assert_eq!(hr, E_INVALIDARG);

    let mut v = PropVariant::default();
    assert_eq!(
        unsafe { arc.GetProperty(3, PropId::Path, &mut v) },
        E_INVALIDARG
    );
}

#[test]
fn extract_all() {
    let arc = open(sample());
    let r = extract(&arc, None, false, &[]);
    assert_eq!(r.hr, HRESULT(0));
    let want: BTreeMap<_, _> = [
        ("a.txt".to_owned(), b"hello".to_vec()),
        ("dir/b.bin".to_owned(), b"\0\x01\x02".to_vec()),
    ]
    .into();
    assert_eq!(r.files, want);
    let ok = ["prepare Extract", "result OK"];
    let mut expected = vec!["total 8", "stream a.txt"];
    expected.extend(ok);
    expected.push("stream dir"); // a directory gets a result even without a stream
    expected.extend(ok);
    expected.push("stream dir/b.bin");
    expected.extend(ok);
    assert_eq!(r.log, expected);
}

#[test]
fn extract_skipped_and_test() {
    let arc = open(sample());
    let r = extract(&arc, Some(&[0, 2]), false, &[0]);
    assert_eq!(r.hr, HRESULT(0));
    assert_eq!(r.files.keys().collect::<Vec<_>>(), ["dir/b.bin"]);
    // No PrepareOperation or result for the item 7-Zip declined.
    assert_eq!(
        r.log,
        [
            "total 8",
            "stream a.txt",
            "stream dir/b.bin",
            "prepare Extract",
            "result OK"
        ]
    );

    let r = extract(&arc, None, true, &[]);
    assert_eq!(r.hr, HRESULT(0));
    assert!(r.files.is_empty());
    assert_eq!(r.log.iter().filter(|l| *l == "result OK").count(), 3);
    assert_eq!(r.log.iter().filter(|l| *l == "prepare Test").count(), 3);

    assert_eq!(extract(&arc, Some(&[3]), false, &[]).hr, E_INVALIDARG);
}

fn try_open(bytes: Vec<u8>) -> (IInArchive, HRESULT) {
    let arc: IInArchive = create().unwrap();
    let hr = unsafe { arc.Open(InStream::new(Cursor::new(bytes)).as_com(), None, None) };
    (arc, hr)
}

fn error_flags(arc: &IInArchive) -> Option<ErrorFlags> {
    let mut v = PropVariant::default();
    unsafe { arc.GetArchiveProperty(PropId::ErrorFlags, &mut v) }.unwrap();
    (!v.is_empty()).then(|| ErrorFlags::from_bits_retain(v.get::<u32>().unwrap()))
}

#[test]
fn truncated_data_lists_what_was_read() {
    let mut bytes = sample();
    bytes.truncate(bytes.len() - 1);
    let (arc, hr) = try_open(bytes);
    assert_eq!(hr, HRESULT(0));
    assert_eq!(error_flags(&arc), Some(ErrorFlags::UnexpectedEnd));
    let mut n = 0;
    unsafe { arc.GetNumberOfItems(&mut n) }.unwrap();
    assert_eq!(n, 3);

    let r = extract(&arc, None, true, &[]);
    assert_eq!(r.hr, HRESULT(0));
    let results: Vec<_> = r.log.iter().filter(|l| l.starts_with("result")).collect();
    assert_eq!(results, ["result OK", "result OK", "result UnexpectedEnd"]);
}

#[test]
fn truncated_header_lists_what_was_read() {
    let bytes = sample();
    let (arc, hr) = try_open(bytes[..bytes.len() - 5].to_vec());
    assert_eq!(hr, HRESULT(0));
    assert_eq!(error_flags(&arc), Some(ErrorFlags::UnexpectedEnd));
    let mut n = 0;
    unsafe { arc.GetNumberOfItems(&mut n) }.unwrap();
    assert_eq!(n, 2);
}

#[test]
fn truncated_before_count_is_a_broken_archive() {
    // S_FALSE with flags that lack IsNotArc: 7-Zip reports a broken Toy archive and
    // doesn't try other formats.
    let (arc, hr) = try_open(b"TOY\x01\x02".to_vec());
    assert_eq!(hr, S_FALSE);
    assert_eq!(error_flags(&arc), Some(ErrorFlags::UnexpectedEnd));

    // The next Open starts clean.
    let (arc, hr) = (arc.clone(), unsafe {
        arc.Open(InStream::new(Cursor::new(sample())).as_com(), None, None)
    });
    assert_eq!(hr, HRESULT(0));
    assert_eq!(error_flags(&arc), None);
}

#[test]
fn not_an_archive() {
    let (arc, hr) = try_open(b"PK\x03\x04".to_vec());
    assert_eq!(hr, S_FALSE);
    assert_eq!(error_flags(&arc), None);
    let mut v = PropVariant::default();
    unsafe { arc.GetArchiveProperty(PropId::PhySize, &mut v) }.unwrap();
    assert!(v.is_empty());
    let mut n = 1;
    unsafe { arc.GetNumberOfItems(&mut n) }.unwrap();
    assert_eq!(n, 0);
}

#[test]
fn item_stream() {
    let arc = open(sample());
    let gs: IInArchiveGetStream = arc.cast().unwrap();
    let mut s: Option<ISequentialInStream> = None;
    unsafe { gs.GetStream(0, &mut s) }.unwrap();
    let s = s.unwrap();
    assert!(s.cast::<IInStream>().is_ok(), "a slice is served seekable");
    let mut text = String::new();
    SeqInStream::from_com(s).read_to_string(&mut text).unwrap();
    assert_eq!(text, "hello");
}

/// The path as 7-Zip reads it first: in place, through `IArchiveGetRawProps`.
fn raw_path(arc: &IInArchive, index: u32) -> Option<String> {
    let raw: IArchiveGetRawProps = arc.cast().unwrap();
    let (mut data, mut size, mut kind) = (core::ptr::null(), 0, 0);
    unsafe { raw.GetRawProp(index, PropId::Path, &mut data, &mut size, &mut kind) }.unwrap();
    if kind != PropDataType::Utf16Z.bits() {
        assert!(data.is_null() && size == 0);
        return None;
    }
    let s = unsafe { core::slice::from_raw_parts(data.cast::<u16>(), size as usize / 2) };
    let (nul, text) = s.split_last().unwrap();
    assert_eq!(*nul, 0);
    Some(String::from_utf16(text).unwrap())
}

#[test]
fn raw_props() {
    let arc = open(sample());
    assert_eq!(raw_path(&arc, 0).as_deref(), Some("a.txt"));
    assert_eq!(raw_path(&arc, 1).as_deref(), Some("dir"));
    assert_eq!(raw_path(&arc, 2).as_deref(), Some("dir/b.bin"));

    let raw: IArchiveGetRawProps = arc.cast().unwrap();
    let (mut data, mut size, mut kind) = (
        core::ptr::NonNull::<u8>::dangling()
            .as_ptr()
            .cast_const()
            .cast(),
        1,
        1,
    );
    unsafe { raw.GetRawProp(0, PropId::NtSecure, &mut data, &mut size, &mut kind) }.unwrap();
    assert!(data.is_null() && size == 0 && kind == PropDataType::NotDefined.bits());
    let mut v = 0;
    assert_eq!(
        unsafe { raw.GetRawProp(3, PropId::Path, &mut data, &mut size, &mut kind) },
        E_INVALIDARG
    );

    // Not a tree: every item is top-level.
    let (mut parent, mut parent_type) = (0, 9);
    unsafe { raw.GetParent(2, &mut parent, &mut parent_type) }.unwrap();
    assert_eq!((parent, parent_type), (u32::MAX, ParentType::Dir.0));
    unsafe { raw.GetNumRawProps(&mut v) }.unwrap();
    assert_eq!(v, 0);

    let root: IArchiveGetRootProps = arc.cast().unwrap();
    let mut pv = PropVariant::default();
    unsafe { root.GetRootProp(PropId::NtSecure, &mut pv) }.unwrap();
    assert!(pv.is_empty());
}

#[test]
fn set_properties() {
    let arc = open(sample());
    let sp: ISetProperties = arc.cast().unwrap();
    let set = |name: PCWSTR, value: PropVariant| unsafe { sp.SetProperties(&name, &value, 1) };
    assert_eq!(set(w!("x"), 9u32.into()), HRESULT(0));
    assert_eq!(set(w!("X"), PropVariant::default()), HRESULT(0));
    // The toy's check_option takes only `x`.
    assert_eq!(set(w!("mt"), PropVariant::default()), E_INVALIDARG);
    // VT_I4: 7-Zip never sends it.
    assert_eq!(set(w!("x"), (-1i32).into()), E_INVALIDARG);
}

/// A second format for checking the export properties: several extensions, one of
/// them wrapping, and several signatures. It reuses the toy's archive.
struct Multi;

impl Format for Multi {
    const NAME: &'static str = "Multi";
    const CLASS_ID: GUID = GUID::from_u128(2);
    const EXTENSIONS: &'static [Ext] = &[Ext::new("mlt"), Ext::wrapping("tmlt", ".toy")];
    const SIGNATURES: &'static [&'static [u8]] = &[b"AB", b"XYZ"];
    const SIGNATURE_OFFSET: u32 = 1;
    type Archive = toy::ToyArchive;

    fn open(stream: InStream, cb: &OpenCallback<'_>, options: &Options) -> Result<Self::Archive> {
        toy::Toy::open(stream, cb, options)
    }
}

#[test]
fn extensions_and_signatures() {
    let formats = Formats(&[Entry::read::<Multi>()]);
    let get = |id| {
        let mut v = PropVariant::default();
        let hr = formats.handler_property2(0, id, (&mut v).into());
        assert_eq!(hr, HRESULT(0));
        v
    };
    assert_eq!(
        get(HandlerPropId::Extension).get::<BSTR>().unwrap(),
        "mlt tmlt"
    );
    assert_eq!(
        get(HandlerPropId::AddExtension).get::<BSTR>().unwrap(),
        "* .toy"
    );
    assert!(get(HandlerPropId::Signature).is_empty());
    let flags = ArcInfoFlags::from_bits_retain(get(HandlerPropId::Flags).get::<u32>().unwrap());
    assert!(flags.contains(ArcInfoFlags::MultiSignature));

    let multi = get(HandlerPropId::MultiSignature);
    let p = unsafe { multi.Anonymous.Anonymous.Anonymous.bstrVal.as_ptr() }.cast::<u8>();
    let len = unsafe { p.cast::<u32>().sub(1).read_unaligned() } as usize;
    assert_eq!(
        unsafe { core::slice::from_raw_parts(p, len) },
        b"\x02AB\x03XYZ"
    );

    let is_arc = |head: &[u8]| Multi::is_arc(head);
    assert_eq!(is_arc(b"_ABc"), IsArcResult::Yes);
    assert_eq!(is_arc(b"_XYZ"), IsArcResult::Yes);
    assert_eq!(is_arc(b"_XY"), IsArcResult::NeedMore);
    assert_eq!(is_arc(b"_CDEF"), IsArcResult::No);

    // The toy has one extension and one signature, so neither list is used.
    let mut v = PropVariant::default();
    unsafe { toy::GetHandlerProperty2(0, HandlerPropId::AddExtension, (&mut v).into()) }.unwrap();
    assert!(v.is_empty());
    unsafe { toy::GetHandlerProperty2(0, HandlerPropId::Signature, (&mut v).into()) }.unwrap();
    assert!(!v.is_empty());
}

#[test]
fn module_version() {
    assert_eq!(export::version("1", "5"), 0x0001_0005);
    assert_eq!(export::version("12", "34"), 0x000C_0022);
    let mut v = PropVariant::default();
    unsafe { toy::GetModuleProp(1, (&mut v).into()) }.unwrap();
    let want = export::version(
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
    );
    assert_eq!(v.get::<u32>().unwrap(), want);
    unsafe { toy::GetModuleProp(0, (&mut v).into()) }.unwrap();
    assert_eq!(
        v.get::<u32>().unwrap(),
        0,
        "no virtual destructor in IUnknown"
    );
}

enum HostItem {
    Keep(u32),
    New(&'static str, &'static [u8]),
}

#[implement(IArchiveUpdateCallback, IArchiveUpdateCallbackFile)]
struct UpdateHost {
    arc: IInArchive,
    items: Vec<HostItem>,
    done: Rc<RefCell<u32>>,
    reports: Rc<RefCell<Vec<String>>>,
}

impl IArchiveUpdateCallbackFile_Impl for UpdateHost_Impl {
    unsafe fn GetStream2(
        &self,
        index: u32,
        s: OutRef<'_, ISequentialInStream>,
        _: UpdateNotifyOp,
    ) -> HRESULT {
        unsafe { IArchiveUpdateCallback_Impl::GetStream(self, index, s) }
    }
    unsafe fn ReportOperation(
        &self,
        kind: EventIndexType,
        index: u32,
        op: UpdateNotifyOp,
    ) -> HRESULT {
        // What 7-Zip does here: read the old item's path from the handler, re-entering it
        // while its UpdateItems is running.
        assert_eq!(kind, EventIndexType::InArcIndex);
        let name = raw_path(&self.arc, index).unwrap();
        self.reports.borrow_mut().push(format!("{op:?} {name}"));
        HRESULT(0)
    }
}

impl IProgress_Impl for UpdateHost_Impl {
    unsafe fn SetTotal(&self, _: u64) -> HRESULT {
        HRESULT(0)
    }
    unsafe fn SetCompleted(&self, _: Option<&u64>) -> HRESULT {
        HRESULT(0)
    }
}

impl IArchiveUpdateCallback_Impl for UpdateHost_Impl {
    unsafe fn GetUpdateItemInfo(
        &self,
        index: u32,
        new_data: OutRef<'_, i32>,
        new_props: OutRef<'_, i32>,
        index_in_archive: OutRef<'_, u32>,
    ) -> HRESULT {
        let (n, old) = match self.items[index as usize] {
            HostItem::Keep(old) => (0, old),
            HostItem::New(..) => (1, u32::MAX),
        };
        new_data.write(n).unwrap();
        new_props.write(n).unwrap();
        index_in_archive.write(old).unwrap();
        HRESULT(0)
    }
    unsafe fn GetProperty(
        &self,
        index: u32,
        id: PropId,
        value: OutRef<'_, PropVariant>,
    ) -> HRESULT {
        let HostItem::New(path, data) = self.items[index as usize] else {
            return E_INVALIDARG;
        };
        let v = match id {
            PropId::Path => path.into(),
            PropId::IsDir => false.into(),
            PropId::Size => (data.len() as u64).into(),
            _ => PropVariant::default(),
        };
        value.write(v).unwrap();
        HRESULT(0)
    }
    unsafe fn GetStream(&self, index: u32, s: OutRef<'_, ISequentialInStream>) -> HRESULT {
        let HostItem::New(_, data) = self.items[index as usize] else {
            return E_INVALIDARG;
        };
        let stream = InStream::new(Cursor::new(data));
        s.write(Some((**stream.as_com()).clone())).unwrap();
        HRESULT(0)
    }
    unsafe fn SetOperationResult(&self, _: i32) -> HRESULT {
        *self.done.borrow_mut() += 1;
        HRESULT(0)
    }
}

#[test]
fn update_existing() {
    let arc = open(sample());
    let out_archive: IOutArchive = arc.cast().unwrap();
    let mut t = 9;
    unsafe { out_archive.GetFileTimeType(&mut t) }.unwrap();
    assert_eq!(t, FileTimeType::Windows.0 as u32);

    // Keep a.txt and dir/b.bin, drop dir/, add new.txt.
    let done = Rc::new(RefCell::new(0));
    let reports = Rc::new(RefCell::new(Vec::new()));
    let host = UpdateHost {
        arc: arc.clone(),
        items: vec![
            HostItem::Keep(2),
            HostItem::New("sub\\new.txt", b"fresh"),
            HostItem::Keep(0),
        ],
        done: done.clone(),
        reports: reports.clone(),
    };
    let cb: IArchiveUpdateCallback = host.into();
    let out = Rc::new(RefCell::new(BTreeMap::new()));
    let sink = OutStream::new(Sink(out.clone(), String::new()));
    unsafe { out_archive.UpdateItems(sink.as_com(), 3, &cb) }.unwrap();
    assert_eq!(*done.borrow(), 1, "item_done once per new item");
    assert_eq!(
        *reports.borrow(),
        ["Replicate dir/b.bin", "Replicate a.txt"]
    );

    let bytes = out.borrow()[""].clone();
    assert_eq!(
        bytes,
        build(&[
            ("dir/b.bin", b"\0\x01\x02"),
            ("sub/new.txt", b"fresh"),
            ("a.txt", b"hello")
        ])
    );
    let new = open(bytes);
    assert_eq!(path(&new, 1), "sub/new.txt");
}

#[test]
fn exports() {
    let mut n = 0;
    unsafe { toy::GetNumberOfFormats(&mut n) }.unwrap();
    assert_eq!(n, 1);

    let get = |id| {
        let mut v = PropVariant::default();
        unsafe { toy::GetHandlerProperty2(0, id, (&mut v).into()) }.unwrap();
        v
    };
    assert_eq!(get(HandlerPropId::Name).get::<BSTR>().unwrap(), "Toy");
    assert_eq!(get(HandlerPropId::Extension).get::<BSTR>().unwrap(), "toy");
    assert!(get(HandlerPropId::Update).get::<bool>().unwrap());
    assert_eq!(get(HandlerPropId::Flags).get::<u32>().unwrap(), 0);
    assert!(get(HandlerPropId::TimeFlags).is_empty());

    let class_id = get(HandlerPropId::ClassID);
    let raw = unsafe { class_id.Anonymous.Anonymous.Anonymous.bstrVal.as_ptr() }.cast::<GUID>();
    assert_eq!(unsafe { raw.read_unaligned() }, toy::Toy::CLASS_ID);

    let mut is_arc = None;
    unsafe { toy::GetIsArc(0, &mut is_arc) }.unwrap();
    let is_arc = is_arc.unwrap();
    let head = sample();
    assert_eq!(
        unsafe { is_arc(head.as_ptr(), head.len()) },
        IsArcResult::Yes
    );
    assert_eq!(unsafe { is_arc(b"PK".as_ptr(), 2) }, IsArcResult::NeedMore);
    assert_eq!(
        unsafe { is_arc(b"PK\x03\x04".as_ptr(), 4) },
        IsArcResult::No
    );

    let mut p = core::ptr::null_mut();
    let other = GUID::from_u128(1);
    let hr = unsafe { toy::CreateObject(&other, &IInArchive::IID, &mut p) };
    assert!(hr.is_err() && p.is_null());
}

#[test]
fn read_only_handler_has_no_out_archive() {
    let formats = Formats(&[Entry::read::<toy::Toy>()]);
    let mut p = core::ptr::null_mut();
    let class_id = toy::Toy::CLASS_ID;
    let hr = unsafe { formats.create_object(&class_id, &IOutArchive::IID, &mut p) };
    assert_eq!(hr, E_NOINTERFACE);
    let hr = unsafe { formats.create_object(&class_id, &IInArchive::IID, &mut p) };
    assert_eq!(hr, HRESULT(0));
    drop(unsafe { IInArchive::from_raw(p) });

    let mut v = PropVariant::default();
    let hr = formats.handler_property2(0, HandlerPropId::Update, (&mut v).into());
    assert_eq!(hr, HRESULT(0));
    assert!(!v.get::<bool>().unwrap());
}

#[test]
fn close_forgets_archive() {
    let arc: IInArchive = create().unwrap();
    let stream = InStream::new(Cursor::new(sample()));
    assert_eq!(unsafe { arc.Open(stream.as_com(), None, None) }, HRESULT(0));
    assert_eq!(unsafe { arc.Close() }, HRESULT(0));
    let mut n = 1;
    unsafe { arc.GetNumberOfItems(&mut n) }.unwrap();
    assert_eq!(n, 0);
}
