//! WADs built by hand in every version, read through the `Format`/`Archive` traits, and
//! written back through 7-Zip's `IOutArchive` with a host written here.

use core::cell::{Cell, RefCell};
use lol7z_api::export::{Entry as FormatEntry, Formats};
use lol7z_api::sys::i_archive::{
    EventIndexType, IArchiveOpenCallback, IArchiveOpenCallback_Impl, IArchiveUpdateCallback,
    IArchiveUpdateCallback_Impl, IInArchive, IOutArchive, UpdateNotifyOp,
};
use lol7z_api::sys::i_progress::IProgress_Impl;
use lol7z_api::sys::i_stream::ISequentialInStream;
use lol7z_api::sys::propvar::PropVariant;
use lol7z_api::{
    Archive, Error, ErrorFlags, Format, InStream, OpenCallback, OperationResult, Options,
    OutStream, PropId,
};
use lol7z_wad::toc::{self, Entry, Kind};
use lol7z_wad::{Form, Names, PathHash, Resolver, Wad, WadArchive, display};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;
use windows_core::{HRESULT, Interface, OutRef, implement};
use xxhash_rust::xxh3::xxh3_64;

/// Stored bytes for one entry, by kind.
struct Item {
    hash: u64,
    kind: Kind,
    stored: Vec<u8>,
    size: u32,
}

fn item(hash: u64, kind: Kind, data: &[u8]) -> Item {
    let stored = match kind {
        Kind::Gzip => {
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            e.write_all(data).unwrap();
            e.finish().unwrap()
        }
        // Two frames back to back.
        Kind::Zstd => {
            let (a, b) = data.split_at(data.len() / 2);
            [
                zstd::bulk::compress(a, 1).unwrap(),
                zstd::bulk::compress(b, 1).unwrap(),
            ]
            .concat()
        }
        // Four subchunks: raw, frame, raw, frame.
        Kind::ZstdMulti => {
            let q = data.len() / 4;
            let z = |s: &[u8]| zstd::bulk::compress(s, 1).unwrap();
            [
                &data[..q],
                &z(&data[q..2 * q]),
                &data[2 * q..3 * q],
                &z(&data[3 * q..]),
            ]
            .concat()
        }
        Kind::Link => [&(data.len() as u32).to_le_bytes()[..], data].concat(),
        _ => data.to_vec(),
    };
    Item {
        hash,
        kind,
        stored,
        size: if kind == Kind::Link {
            4 + data.len() as u32
        } else {
            data.len() as u32
        },
    }
}

/// A WAD of `version` holding `items` in the order given.
fn build(version: (u8, u8), items: &[Item]) -> Vec<u8> {
    let (major, minor) = version;
    let (header_size, entry_size) = match major {
        1 => (12, 24),
        2 => (104, 32),
        _ => (272, 32),
    };
    let mut data_pos = header_size + entry_size * items.len();
    let mut v = vec![b'R', b'W', major, minor];
    match major {
        1 => {}
        2 => v.extend([0xAB; 84 + 8]),
        _ => v.extend([0xAB; 256 + 8]),
    }
    if major < 3 {
        v.extend((header_size as u16).to_le_bytes());
        v.extend((entry_size as u16).to_le_bytes());
    }
    v.extend((items.len() as u32).to_le_bytes());
    for it in items {
        v.extend(it.hash.to_le_bytes());
        v.extend((data_pos as u32).to_le_bytes());
        v.extend((it.stored.len() as u32).to_le_bytes());
        v.extend(it.size.to_le_bytes());
        let sub = if it.kind == Kind::ZstdMulti { 4 } else { 0 };
        v.push(sub << 4 | it.kind.0);
        v.extend([0; 3]);
        // 2.x and 3.0 hold a SHA-256 prefix instead, which the reader doesn't check.
        if major >= 2 {
            v.extend(xxh3_64(&it.stored).to_le_bytes());
        }
        data_pos += it.stored.len();
    }
    for it in items {
        v.extend(&it.stored);
    }
    v
}

fn open(bytes: Vec<u8>) -> (WadArchive, lol7z_api::OpenReport) {
    let cb = OpenCallback::none();
    let a = Wad::open(InStream::new(Cursor::new(bytes)), &cb, &Options::default()).unwrap();
    (a, cb.report())
}

fn extract(a: &WadArchive, index: u32) -> (OperationResult, Vec<u8>) {
    let mut out = Vec::new();
    let r = a.extract_item(index, &mut out).unwrap();
    (r, out)
}

fn text(a: &WadArchive, index: u32, id: PropId) -> String {
    a.item_prop(index, id).unwrap().into_string().unwrap()
}

const TEXT: &[u8] = b"the quick brown fox jumps over the lazy dog, again and again and again";

#[test]
fn reads_every_kind() {
    let items = [
        item(1, Kind::Raw, b"raw"),
        item(2, Kind::Gzip, TEXT),
        item(3, Kind::Zstd, TEXT),
        item(4, Kind::ZstdMulti, TEXT),
        item(5, Kind::Link, b"data/other.bin"),
    ];
    let (a, report) = open(build((3, 4), &items));
    assert_eq!(report.errors, ErrorFlags::empty());
    assert_eq!(report.warnings, ErrorFlags::empty());
    assert_eq!(a.item_count(), 5);
    assert_eq!(extract(&a, 0), (OperationResult::OK, b"raw".to_vec()));
    assert_eq!(extract(&a, 1), (OperationResult::OK, TEXT.to_vec()));
    assert_eq!(extract(&a, 2), (OperationResult::OK, TEXT.to_vec()));
    assert_eq!(extract(&a, 3), (OperationResult::OK, TEXT.to_vec()));
    assert_eq!(extract(&a, 4).1, items[4].stored);
    assert_eq!(text(&a, 0, PropId::Path), "«0000000000000001»");
    assert_eq!(text(&a, 1, PropId::Method), "GZip");
    assert_eq!(text(&a, 3, PropId::Method), "ZstdMulti");
    assert_eq!(text(&a, 4, PropId::Comment), "data/other.bin");
    assert_eq!(
        a.item_prop(2, PropId::Size).unwrap().as_u64(),
        Some(TEXT.len() as u64)
    );
    assert_eq!(
        a.item_prop(2, PropId::PackSize).unwrap().as_u64(),
        Some(items[2].stored.len() as u64)
    );
    assert_eq!(
        a.archive_prop(PropId::UnpackVer).unwrap().as_str(),
        Some("3.4")
    );
    assert_eq!(a.entries().nth(3).unwrap().subchunk_count, 4);
}

#[test]
fn reads_old_versions() {
    for version in [(1, 0), (2, 0), (2, 1), (3, 0), (3, 1), (3, 3)] {
        let items = [item(7, Kind::Gzip, TEXT), item(9, Kind::Raw, b"x")];
        let (a, report) = open(build(version, &items));
        assert_eq!(report.errors, ErrorFlags::empty(), "{version:?}");
        assert_eq!(a.item_count(), 2);
        let checksum = if version.0 >= 2 {
            xxh3_64(&items[0].stored)
        } else {
            0
        };
        assert_eq!(
            a.entries().next().unwrap().checksum,
            checksum,
            "{version:?}"
        );
        assert_eq!(extract(&a, 0), (OperationResult::OK, TEXT.to_vec()));
        assert_eq!(extract(&a, 1), (OperationResult::OK, b"x".to_vec()));
        let want = format!("{}.{}", version.0, version.1);
        assert_eq!(
            a.archive_prop(PropId::UnpackVer).unwrap().as_str(),
            Some(&*want)
        );
    }
}

/// Only stored entries are handed over as streams; 7-Zip extracts the rest.
#[test]
fn item_stream_for_stored_only() {
    let items = [item(1, Kind::Raw, TEXT), item(2, Kind::Zstd, TEXT)];
    let (a, _) = open(build((3, 4), &items));
    assert!(a.item_stream(0).unwrap().is_some());
    assert!(a.item_stream(1).unwrap().is_none());
}

#[test]
fn not_a_wad() {
    let cb = OpenCallback::none();
    let err = |bytes: &[u8]| {
        Wad::open(
            InStream::new(Cursor::new(bytes.to_vec())),
            &cb,
            &Options::default(),
        )
        .err()
    };
    assert_eq!(err(b"PK\x03\x04"), Some(Error::NotArchive));
    assert_eq!(err(b"\0\0\0\0\0\0\0\0"), Some(Error::NotArchive));
    assert_eq!(err(b"RW"), Some(Error::NotArchive));
    assert_eq!(
        err(b"RW\x09\x00"),
        Some(Error::Damaged(ErrorFlags::Unsupported))
    );
    assert_eq!(err(b"RW\x03\x00abc"), Some(Error::UNEXPECTED_END));
}

#[test]
fn truncated_lists_what_fits() {
    let items = [item(1, Kind::Raw, b"aaaa"), item(2, Kind::Raw, b"bbbb")];
    let mut bytes = build((3, 4), &items);
    // Cut inside the second item's data.
    bytes.truncate(bytes.len() - 2);
    let (a, report) = open(bytes.clone());
    assert_eq!(a.item_count(), 2);
    assert_eq!(report.errors, ErrorFlags::UnexpectedEnd);
    assert_eq!(extract(&a, 0), (OperationResult::OK, b"aaaa".to_vec()));
    assert_eq!(
        extract(&a, 1),
        (OperationResult::UnexpectedEnd, b"bb".to_vec())
    );

    // Cut inside the table: only whole entries are listed.
    bytes.truncate(272 + 32 + 10);
    let (a, report) = open(bytes);
    assert_eq!(a.item_count(), 1);
    assert_eq!(report.errors, ErrorFlags::UnexpectedEnd);
}

#[test]
fn unsorted_is_a_warning() {
    let items = [item(2, Kind::Raw, b"b"), item(1, Kind::Raw, b"a")];
    let (a, report) = open(build((3, 4), &items));
    assert_eq!(a.item_count(), 2);
    assert_eq!(report.errors, ErrorFlags::empty());
    assert_eq!(report.warnings, ErrorFlags::HeadersError);
    assert_eq!(
        report.warning.as_deref(),
        Some("entries not sorted by hash")
    );
}

#[test]
fn duplicate_is_a_warning() {
    let items = [item(1, Kind::Raw, b"b"), item(1, Kind::Link, b"a")];
    let (a, report) = open(build((3, 0), &items));
    assert_eq!(a.item_count(), 2);
    assert_eq!(report.errors, ErrorFlags::empty());
    assert_eq!(report.warnings, ErrorFlags::HeadersError);
    assert_eq!(report.warning.as_deref(), Some("duplicate entry hashes"));

    let items = [
        item(1, Kind::Raw, b"b"),
        item(1, Kind::Raw, b"a"),
        item(0, Kind::Raw, b"c"),
    ];
    let (_, report) = open(build((3, 4), &items));
    assert_eq!(
        report.warning.as_deref(),
        Some("duplicate entry hashes; entries not sorted by hash")
    );
}

#[test]
fn unknown_entry_type_lists_but_does_not_extract() {
    let items = [item(1, Kind(7), b"a"), item(2, Kind::Raw, b"b")];
    let (a, report) = open(build((3, 4), &items));
    assert_eq!(report.errors | report.warnings, ErrorFlags::empty());
    assert_eq!(a.item_count(), 2);
    assert_eq!(text(&a, 0, PropId::Method), "Unknown");
    assert_eq!(
        a.extract_item(0, &mut Vec::new()),
        Err(Error::Damaged(ErrorFlags::UnsupportedMethod))
    );
    assert_eq!(extract(&a, 1), (OperationResult::OK, b"b".to_vec()));
}

/// More entries than one table read.
fn many(n: u32) -> Vec<Item> {
    (0..n)
        .map(|i| item(u64::from(i) + 1, Kind::Raw, &[i as u8]))
        .collect()
}

#[test]
fn table_read_in_chunks() {
    let (a, report) = open(build((3, 4), &many(10_000)));
    assert_eq!(report.errors | report.warnings, ErrorFlags::empty());
    assert_eq!(a.item_count(), 10_000);
    let entries: Vec<&Entry> = a.entries().collect();
    for i in [0, 4095, 4096, 8191, 8192, 9999] {
        assert_eq!(entries[i].hash.0, i as u64 + 1);
        assert_eq!(extract(&a, i as u32), (OperationResult::OK, vec![i as u8]));
    }
}

#[test]
fn link_length_past_entry_is_an_error() {
    let mut link = item(1, Kind::Link, b"data/other.bin");
    link.stored[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut short = item(2, Kind::Link, b"data/other.bin");
    short.stored[..4].copy_from_slice(&15u32.to_le_bytes());
    let (a, _) = open(build((3, 4), &[link, short]));
    assert!(a.item_prop(0, PropId::Comment).is_none());
    assert!(a.item_prop(1, PropId::Comment).is_none());
}

#[test]
fn path_hashes() {
    let h = |p: &str| PathHash::from_path(p).unwrap().0;
    assert_eq!(h("0123456789abcdef"), 0x0123456789abcdef);
    assert_eq!(h("some\\dir\\0123456789ABCDEF"), 0x0123456789abcdef);
    assert_eq!(
        h("DATA\\Final\\X.bin"),
        xxhash_rust::xxh64::xxh64(b"data/final/x.bin", 0)
    );
    assert_ne!(h("+123456789abcdef"), 0x123456789abcdef);
}

// ---------------------------------------------------------------- names

fn xxh(path: &str) -> u64 {
    xxhash_rust::xxh64::xxh64(path.to_ascii_lowercase().as_bytes(), 0)
}

fn hash(path: &str) -> PathHash {
    PathHash(xxh(path))
}

/// What `path` shows as in 7-Zip, and in what form.
fn shown(path: &str) -> (String, Option<Form>) {
    let mut out = Vec::new();
    let form = display(path, hash(path), &mut out).unwrap();
    (String::from_utf16(&out).unwrap(), form)
}

#[test]
fn from_path_reads_names_back() {
    let h = |p: &str| PathHash::from_path(p).unwrap().0;
    assert_eq!(h("0123456789abcdef"), 0x0123456789abcdef);
    assert_eq!(h("dir/0123456789abcdef.bin"), 0x0123456789abcdef);
    assert_eq!(h("«0123456789abcdef»"), 0x0123456789abcdef);
    assert_eq!(h("any/Dir\\«0123456789ABCDEF».bin"), 0x0123456789abcdef);
    assert_eq!(h("d/ahri«_s{1…3}».bin"), xxh("d/ahri_s1_s2_s3.bin"));
    assert_eq!(h("d/a«_s{1,2}».bin\u{F02F}"), xxh("d/a_s1_s2.bin"));
    // Not ours: hashed as it is.
    assert_eq!(h("d/a«b».bin"), xxh("d/a«b».bin"));
    assert_eq!(h("d/a»b"), xxh("d/a»b"));
    for bad in ["d/a«_s{9…1}».bin", "d/a«{x»", "d/a«_s{1}»«_s{2}»"] {
        assert_eq!(PathHash::from_path(bad), Err(Error::INVALID_ARG), "{bad}");
    }
    assert_eq!(h("DIR\\File.BIN"), xxh("dir/file.bin"));
    assert_eq!(h("a/b\u{F02F}/c"), xxh("a/b//c"));
    assert_eq!(h("a/\u{F02F}/b"), xxh("a//b"));
    assert_eq!(h("dir\\a\u{F03A}b.bin"), xxh("dir/a:b.bin"));
    assert_eq!(h("a/b\u{F02F}"), xxh("a/b"));
    assert_eq!(
        h("A/\u{C9}.bin"),
        xxhash_rust::xxh64::xxh64("a/\u{C9}.bin".as_bytes(), 0)
    );
}

#[test]
fn display_maps_what_windows_refuses() {
    let plain = Some(Form::Plain);
    assert_eq!(shown("a//b"), ("a/\u{F02F}/b".into(), plain));
    assert_eq!(shown("a:b"), ("a\u{F03A}b".into(), plain));
    assert_eq!(shown("a/b\u{1}?"), ("a/b\u{F001}\u{F03F}".into(), plain));
    assert_eq!(shown("data/x.bin"), ("data/x.bin".into(), plain));
    assert_eq!(shown("a/"), ("a/\u{F02F}".into(), plain));

    // Nothing to fold and no `_` to cut at: all of it hidden.
    let long = format!("dir/{}", "x".repeat(256));
    let want = format!("dir/«{}-{}»", hash(&long), hash("dir/"));
    assert_eq!(shown(&long), (want, Some(Form::Hashed)));
    assert_eq!(shown(&format!("dir/{}", "x".repeat(255))).1, plain);
    let long_dir = format!("{}/f", "x".repeat(256));
    assert_eq!(shown(&long_dir), (String::new(), None));
}

/// `ahri_multi_skins_skin0_skins_skin1_skins_skin10…`, in the order the game writes it.
fn ahri() -> String {
    let mut n: Vec<String> = (0..=84).map(|n| n.to_string()).collect();
    n.sort();
    let skins: String = n.iter().map(|n| format!("_skins_skin{n}")).collect();
    format!("characters/ahri/Ahri_multi{skins}.bin")
}

#[test]
fn display_folds_long_names() {
    let path = ahri();
    let (name, form) = shown(&path);
    assert_eq!(name, "characters/ahri/Ahri_multi«_skins_skin{0…84}».bin");
    assert_eq!(form, Some(Form::Folded));
    // With a mapped character in the preamble.
    let path = path.replace("Ahri_", "Ah:ri_");
    let (name, _) = shown(&path);
    assert!(
        name.starts_with("characters/ahri/Ah\u{F03A}ri_multi«"),
        "{name}"
    );
    assert_eq!(PathHash::from_path(&name).unwrap(), hash(&path));
}

#[test]
fn display_round_trips() {
    let long = format!("dir/{}", "x".repeat(256));
    let ahri = ahri();
    for path in [
        "a//b",
        "a:b",
        "Dir/A<b>.bin",
        "a/b\u{1f}",
        "a/",
        &long,
        &ahri,
    ] {
        assert_eq!(
            PathHash::from_path(&shown(path).0),
            Ok(hash(path)),
            "{path}"
        );
    }
}

#[test]
fn names_mark_files_that_are_folders() {
    let wrong = hash("not/this.bin");
    let names = Names::from_paths(&[
        (hash("a/b"), Some("a/b")),
        (hash("a/b.bin"), Some("a/b.bin")),
        (hash("a/b/c"), Some("a/b/c")),
        (wrong, Some("something/else.bin")),
        (PathHash(0xabc), None),
    ])
    .unwrap();
    let all: Vec<String> = (0..5).map(|i| names.string(i)).collect();
    assert_eq!(
        all,
        [
            "a/b\u{F02F}".to_string(),
            "a/b.bin".into(),
            "a/b/c".into(),
            format!("«{wrong}»"),
            "«0000000000000abc»".into(),
        ]
    );
    assert_eq!(names.utf16z(0).last(), Some(&0));
    assert_eq!(PathHash::from_path(&all[0]), Ok(hash("a/b")));
    let s = names.stats();
    assert_eq!(
        (
            s.named,
            s.clash,
            s.unverified,
            s.unknown,
            s.folded,
            s.hashed
        ),
        (3, 1, 1, 1, 0, 0)
    );
}

/// Windows doesn't tell `A/B` from `a/b`.
#[test]
fn clash_ignores_case() {
    let names = Names::from_paths(&[
        (hash("A/B"), Some("A/B")),
        (hash("a/b/c"), Some("a/b/c")),
        (hash("a/bc"), Some("a/bc")),
    ])
    .unwrap();
    assert_eq!(names.string(0), "A/B\u{F02F}");
    assert_eq!(names.string(2), "a/bc");
    assert_eq!(names.stats().clash, 1);
}

/// A file of 255 units that is also a folder has no room for the mark: it's shown again
/// a unit shorter.
#[test]
fn clash_at_the_limit() {
    let file = format!("d/{}", "y".repeat(255));
    let under = format!("{file}/z");
    let names =
        Names::from_paths(&[(hash(&file), Some(&file)), (hash(&under), Some(&under))]).unwrap();
    let want = format!("d/«{}-{}»\u{F02F}", hash(&file), hash("d/"));
    assert_eq!(names.string(0), want);
    assert_eq!(names.string(1), under);
    assert_eq!(PathHash::from_path(&want), Ok(hash(&file)));
    let s = names.stats();
    assert_eq!((s.named, s.clash, s.hashed), (2, 1, 1));
}

/// Paths whose display wouldn't read back are refused, and shown by hash.
#[test]
fn unshowable_table_paths() {
    let long_dir = format!("{}/f", "x".repeat(256));
    let paths = [
        "a\\b.bin",
        "a/b\u{F03A}.bin",
        "a/«b».bin",
        "a/b».bin",
        "dir/0123456789abcdef.bin",
        &long_dir,
    ];
    let entries: Vec<(PathHash, Option<&str>)> =
        paths.iter().map(|p| (hash(p), Some(*p))).collect();
    let names = Names::from_paths(&entries).unwrap();
    for (i, p) in paths.iter().enumerate() {
        assert_eq!(names.string(i as u32), format!("«{}»", hash(p)), "{p}");
    }
    assert_eq!(names.stats().unverified, paths.len() as u32);
}

#[test]
fn unknown_reads_back_anywhere() {
    let names = Names::from_paths(&[(PathHash(0xab), None)]).unwrap();
    let name = names.string(0);
    assert_eq!(name, "«00000000000000ab»");
    for p in [
        name.clone(),
        format!("moved/to/{name}"),
        format!("x\\{name}.bin"),
    ] {
        assert_eq!(PathHash::from_path(&p), Ok(PathHash(0xab)), "{p}");
    }
}

#[test]
fn names_fold_long_names() {
    let path = ahri();
    let names = Names::from_paths(&[(hash(&path), Some(&path))]).unwrap();
    let name = names.string(0);
    assert_eq!(name, "characters/ahri/Ahri_multi«_skins_skin{0…84}».bin");
    assert_eq!(PathHash::from_path(&name), Ok(hash(&path)));
    let s = names.stats();
    assert_eq!((s.named, s.folded, s.hashed), (1, 1, 0));
}

/// 306 units, every token different, so nothing folds.
const AVATAR_DIR: &str = "assets/ux/tft/pettftavatar/";

fn avatar() -> (String, String, String) {
    let tail: String = (0..75).map(|k| format!("_s{k}")).collect();
    let name = format!("pettftavatar{tail}.bin");
    assert!(name.len() > 300);
    let path = format!("{AVATAR_DIR}{name}");
    (path, name, tail)
}

#[test]
fn hashed_names() {
    let (path, name, tail) = avatar();
    assert_eq!(lol7z_fold::encode(&name), None);
    let full = hash(&path);
    let names = Names::from_paths(&[(full, Some(&path))]).unwrap();
    let shown = names.string(0);
    let s = names.stats();
    assert_eq!((s.named, s.folded, s.hashed), (1, 0, 1));

    // The preamble keeps whole tokens and leaves room for the block and a clash mark.
    let pre = shown
        .strip_prefix(AVATAR_DIR)
        .and_then(|n| n.split_once('«'))
        .unwrap()
        .0;
    assert!(pre.starts_with("pettftavatar_s0_s1") && pre.len() <= 219 - 4);
    assert!(tail.starts_with(&pre["pettftavatar".len()..]));
    let outer = hash(&format!("{AVATAR_DIR}{pre}.bin"));
    assert_eq!(shown, format!("{AVATAR_DIR}{pre}«{full}-{outer}».bin"));
    assert!(shown.rsplit('/').next().unwrap().encode_utf16().count() <= 254);
    assert_eq!(PathHash::from_path(&shown), Ok(full));
    assert_eq!(PathHash::from_path(&shown.to_uppercase()), Ok(full));

    let middle = &name[pre.len()..name.len() - 4];
    let moved = shown.replace(AVATAR_DIR, "other/");
    let renamed = shown.replace("pettftavatar_s0", "renamed_s0");
    let retyped = shown.replace("».bin", "».txt");
    for p in [&moved, &renamed, &retyped] {
        assert_eq!(PathHash::from_path(p), Err(Error::INVALID_ARG), "{p}");
    }

    let dir = cache(
        "hashed",
        ltk_hashdb::Casing::AsciiInsensitive,
        &[&path],
        &[],
    );
    let mut r = Resolver::with_dir(&dir);
    assert_eq!(r.hash(&shown), Ok(full));
    assert_eq!(r.hash(&moved), Ok(hash(&format!("other/{name}"))));
    let renamed_pre = pre.replace("pettftavatar", "renamed");
    assert_eq!(
        r.hash(&renamed),
        Ok(hash(&format!("{AVATAR_DIR}{renamed_pre}{middle}.bin")))
    );
    assert_eq!(
        r.hash(&retyped),
        Ok(hash(&format!("{AVATAR_DIR}{pre}{middle}.txt")))
    );
    assert_eq!(r.hash("plain/name.bin"), Ok(hash("plain/name.bin")));
    // FULL not in the tables.
    let unknown = moved.replace(&full.to_string(), "0000000000000001");
    assert_eq!(r.hash(&unknown), Err(Error::INVALID_ARG));
    let _ = std::fs::remove_dir_all(&dir);

    let mut none = Resolver::with_dir(&dir);
    assert_eq!(none.hash(&moved), Err(Error::INVALID_ARG));
}

/// An extension too long to show is hidden with the middle, and spliced back with it.
#[test]
fn hashed_hidden_extension() {
    let ext = format!(".{}", "e".repeat(300));
    let path = format!("d/a_b{ext}");
    let full = hash(&path);
    let (shown, form) = shown(&path);
    assert_eq!(form, Some(Form::Hashed));
    assert_eq!(shown, format!("d/a_b«{full}-{}»", hash("d/a_b")));
    assert_eq!(PathHash::from_path(&shown), Ok(full));

    let dir = cache(
        "hidden_ext",
        ltk_hashdb::Casing::AsciiInsensitive,
        &[&path],
        &[],
    );
    let mut r = Resolver::with_dir(&dir);
    assert_eq!(r.hash(&shown), Ok(full));
    let moved = shown.replace("d/", "other/");
    assert_eq!(r.hash(&moved), Ok(hash(&format!("other/a_b{ext}"))));
    let renamed = shown.replace("a_b«", "z_b«");
    assert_eq!(r.hash(&renamed), Ok(hash(&format!("d/z_b{ext}"))));
    let retyped = format!("{shown}.txt");
    assert_eq!(r.hash(&retyped), Ok(hash(&format!("d/a_b{ext}.txt"))));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mapped_character_inside_fold() {
    let path = ahri().replace("_skins_", "_sk:ins_");
    let (name, form) = shown(&path);
    assert_eq!(form, Some(Form::Folded));
    let block = &name[name.find('«').unwrap()..];
    assert!(block.contains('\u{F03A}'), "{name}");
    assert_eq!(PathHash::from_path(&name), Ok(hash(&path)));
}

/// No display has a folded component this long, so it isn't decoded.
#[test]
fn long_fold_refused() {
    let name = format!("a«_s{{0…9}}{}».bin", "{a}".repeat(100));
    assert!(name.encode_utf16().count() > 255);
    assert!(lol7z_fold::decode(&name).is_ok());
    assert_eq!(
        PathHash::from_path(&format!("d/{name}")),
        Err(Error::INVALID_ARG)
    );
}

#[test]
fn resolver_without_tables_reads_plain_names() {
    let dir = std::env::temp_dir().join(format!("lol7z_wad_none_{}", std::process::id()));
    let mut r = Resolver::with_dir(&dir);
    assert_eq!(r.hash("plain/name.bin"), Ok(hash("plain/name.bin")));
    assert_eq!(r.hash("Dir\\Name.BIN"), Ok(hash("dir/name.bin")));
    assert!(!dir.exists());
}

/// Every name component over 255 units in the tables: `target/` isn't in git, so this
/// only runs where the census was taken.
#[test]
fn long_names_round_trip() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/test-long/long-component.tsv"
    );
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let rows: Vec<(PathHash, &str)> = text
        .lines()
        .map(|l| {
            let (h, p) = l.split_once('\t').unwrap();
            (PathHash(u64::from_str_radix(h, 16).unwrap()), p)
        })
        .collect();
    let entries: Vec<(PathHash, Option<&str>)> = rows.iter().map(|&(h, p)| (h, Some(p))).collect();
    let names = Names::from_paths(&entries).unwrap();
    for (i, &(h, p)) in rows.iter().enumerate() {
        let name = names.string(i as u32);
        assert_eq!(PathHash::from_path(&name), Ok(h), "{p}");
        assert!(
            name.split('/').all(|c| c.encode_utf16().count() <= 255),
            "{name}"
        );
    }
    let s = names.stats();
    println!("{s:?}");
    assert_eq!((s.named, s.folded, s.hashed), (4379, 4361, 18));
}

/// Every path in the census reads back from its display, and only over-long components
/// change: `cargo test --release -p lol7z_wad -- --ignored census`.
#[test]
#[ignore]
fn census() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/test-long/all-paths.tsv"
    );
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let rows: Vec<(PathHash, &str)> = text
        .lines()
        .map(|l| {
            let (h, p) = l.split_once('\t').unwrap();
            (PathHash(u64::from_str_radix(h, 16).unwrap()), p)
        })
        .collect();
    let (mut named, mut refused) = (0, 0);
    for chunk in rows.chunks(1 << 18) {
        let entries: Vec<(PathHash, Option<&str>)> =
            chunk.iter().map(|&(h, p)| (h, Some(p))).collect();
        let names = Names::from_paths(&entries).unwrap();
        for (i, &(h, p)) in chunk.iter().enumerate() {
            let name = names.string(i as u32);
            assert_eq!(PathHash::from_path(&name), Ok(h), "{p}");
            if name.starts_with('«') {
                refused += 1;
                continue;
            }
            named += 1;
            let mut plain = Vec::new();
            let shown: Vec<&str> = name.split('/').collect();
            let orig: Vec<&str> = p.split('/').collect();
            assert_eq!(shown.len(), orig.len(), "{p}");
            for (k, (s, o)) in shown.iter().zip(&orig).enumerate() {
                let s = s
                    .strip_suffix('\u{F02F}')
                    .filter(|s| !s.is_empty())
                    .unwrap_or(s);
                if k + 1 == orig.len() && o.encode_utf16().count() > 254 {
                    continue;
                }
                plain.clear();
                assert_eq!(display(o, h, &mut plain).unwrap(), Some(Form::Plain), "{p}");
                assert_eq!(String::from_utf16(&plain).unwrap(), *s, "{p}");
            }
        }
    }
    println!("{named} named, {refused} refused");
}

/// A cache directory holding `game` and `lcu` tables.
fn cache(
    name: &str,
    casing: ltk_hashdb::Casing,
    game: &[&str],
    lcu: &[&str],
) -> std::path::PathBuf {
    use ltk_hashdb::{Compression, HashDbWriter, HashKind, KeyConfig, KeyWidth};
    let dir = std::env::temp_dir().join(format!("lol7z_wad_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config = KeyConfig::new(KeyWidth::U64, HashKind::Xxh64, casing);
    let mut tables = Vec::new();
    for (id, paths) in [("game", game), ("lcu", lcu)] {
        let mut w = HashDbWriter::with_key_config(config, Compression::default());
        for p in paths {
            // A row whose path doesn't hash to its key.
            let key = p
                .strip_prefix("wrong:")
                .map_or_else(|| xxh(p), |p| xxh(p) ^ 1);
            w.insert(key, p.strip_prefix("wrong:").unwrap_or(p));
        }
        let file = format!("{id}-2026-01-02.lhdb");
        w.build(std::fs::File::create(dir.join(&file)).unwrap())
            .unwrap();
        tables.push(format!(
            r#""{id}":{{"file":"{file}","sha256":"00","entries":{},"key_width":8,"version":"2026-01-02","format_version":1}}"#,
            paths.len()
        ));
    }
    let manifest = format!(
        r#"{{"schema":1,"generated_at":"2026-01-02T00:00:00Z","tables":{{{}}}}}"#,
        tables.join(",")
    );
    std::fs::write(dir.join("manifest.json"), manifest).unwrap();
    dir
}

#[test]
fn names_from_tables() {
    let dir = cache(
        "tables",
        ltk_hashdb::Casing::AsciiInsensitive,
        &["data/a.bin", "data/a.bin/b.bin", "wrong:data/w.bin"],
        &["plugins/X.json"],
    );
    let hashes = [
        hash("data/a.bin"),
        hash("data/a.bin/b.bin"),
        PathHash(xxh("data/w.bin") ^ 1),
        hash("plugins/x.json"),
        PathHash(7),
    ];
    let names = Names::with_dir(&dir, hashes.iter().copied()).unwrap();
    assert_eq!(names.comment(), "names: game-2026-01-02, lcu-2026-01-02");
    let all: Vec<String> = (0..5).map(|i| names.string(i)).collect();
    assert_eq!(
        all,
        [
            "data/a.bin\u{F02F}".to_string(),
            "data/a.bin/b.bin".into(),
            format!("«{}»", hashes[2]),
            "plugins/X.json".into(),
            "«0000000000000007»".into(),
        ]
    );
    for (name, h) in all.iter().zip(hashes) {
        assert_eq!(PathHash::from_path(name), Ok(h), "{name}");
    }
    let s = names.stats();
    assert_eq!((s.named, s.clash, s.unverified, s.unknown), (3, 1, 1, 1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn names_without_tables_are_hashes() {
    let dir = cache("keyed", ltk_hashdb::Casing::Sensitive, &["data/a.bin"], &[]);
    let names = Names::with_dir(&dir, [hash("data/a.bin")].into_iter()).unwrap();
    assert_eq!(names.string(0), format!("«{}»", hash("data/a.bin")));
    assert!(
        names.comment().starts_with("names: none (game: keyed"),
        "{}",
        names.comment()
    );
    let _ = std::fs::remove_dir_all(&dir);

    let names = Names::with_dir(&dir, [PathHash(1)].into_iter()).unwrap();
    assert_eq!(names.string(0), "«0000000000000001»");
    assert!(
        names.comment().starts_with("names: none (no manifest"),
        "{}",
        names.comment()
    );
}

// ---------------------------------------------------------------- writing

/// A `Vec` the test can read after the handler wrote to it.
#[derive(Clone, Default)]
struct Shared(Rc<RefCell<Cursor<Vec<u8>>>>);

impl Write for Shared {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for Shared {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().seek(pos)
    }
}

enum HostItem {
    Keep(u32),
    New(&'static str, Vec<u8>),
}

#[implement(IArchiveUpdateCallback)]
struct Host {
    items: Vec<HostItem>,
    done: Rc<RefCell<u32>>,
    /// `SetCompleted` calls so far.
    calls: Cell<u32>,
    /// The `SetCompleted` call, counting from 1, from which on the user has cancelled.
    abort_at: u32,
}

impl Host {
    fn new(items: Vec<HostItem>) -> Self {
        Self {
            items,
            done: Rc::default(),
            calls: Cell::new(0),
            abort_at: u32::MAX,
        }
    }
}

impl IProgress_Impl for Host_Impl {
    unsafe fn SetTotal(&self, _: u64) -> HRESULT {
        HRESULT(0)
    }
    unsafe fn SetCompleted(&self, _: Option<&u64>) -> HRESULT {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() >= self.abort_at {
            return Error::ABORT.code();
        }
        HRESULT(0)
    }
}

impl IArchiveUpdateCallback_Impl for Host_Impl {
    unsafe fn GetUpdateItemInfo(
        &self,
        index: u32,
        new_data: OutRef<'_, i32>,
        new_props: OutRef<'_, i32>,
        index_in_archive: OutRef<'_, u32>,
    ) -> HRESULT {
        let (n, old) = match &self.items[index as usize] {
            HostItem::Keep(old) => (0, *old),
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
        let HostItem::New(path, data) = &self.items[index as usize] else {
            return Error::INVALID_ARG.code();
        };
        let v = match id {
            PropId::Path => (*path).into(),
            PropId::IsDir => path.ends_with('/').into(),
            PropId::Size => (data.len() as u64).into(),
            _ => PropVariant::default(),
        };
        value.write(v).unwrap();
        HRESULT(0)
    }
    unsafe fn GetStream(&self, index: u32, s: OutRef<'_, ISequentialInStream>) -> HRESULT {
        let HostItem::New(_, data) = &self.items[index as usize] else {
            return Error::INVALID_ARG.code();
        };
        let stream = InStream::new(Cursor::new(data.clone()));
        s.write(Some((**stream.as_com()).clone())).unwrap();
        HRESULT(0)
    }
    unsafe fn SetOperationResult(&self, _: i32) -> HRESULT {
        *self.done.borrow_mut() += 1;
        HRESULT(0)
    }
}

/// What the DLL would export for this format.
static FORMATS: Formats<'static> = Formats(&[FormatEntry::update::<Wad>()]);

fn handler(bytes: Option<Vec<u8>>) -> IInArchive {
    let mut p = core::ptr::null_mut();
    unsafe { FORMATS.create_object(&Wad::CLASS_ID, &IInArchive::IID, &mut p) }.unwrap();
    let arc = unsafe { IInArchive::from_raw(p) };
    if let Some(bytes) = bytes {
        let stream = InStream::new(Cursor::new(bytes));
        unsafe { arc.Open(stream.as_com(), None, None) }.unwrap();
    }
    arc
}

/// An open callback whose user cancels at the first progress report.
#[implement(IArchiveOpenCallback)]
struct Cancel {
    at: Rc<Cell<Option<u64>>>,
}

impl IArchiveOpenCallback_Impl for Cancel_Impl {
    unsafe fn SetTotal(&self, _: Option<&u64>, _: Option<&u64>) -> HRESULT {
        HRESULT(0)
    }
    unsafe fn SetCompleted(&self, files: Option<&u64>, _: Option<&u64>) -> HRESULT {
        self.at.set(files.copied());
        Error::ABORT.code()
    }
}

#[test]
fn open_cancels_during_table_read() {
    let arc = handler(None);
    let at = Rc::default();
    let cb: IArchiveOpenCallback = Cancel { at: Rc::clone(&at) }.into();
    let stream = InStream::new(Cursor::new(build((3, 4), &many(10_000))));
    let hr = unsafe { arc.Open(stream.as_com(), None, &cb) };
    assert_eq!(hr, Error::ABORT.code());
    assert_eq!(at.get(), Some(4096));
}

/// Runs `UpdateItems` with `items` against `existing`, with `-mx=level` when given.
fn update(existing: Option<Vec<u8>>, items: Vec<HostItem>, level: Option<u32>) -> Vec<u8> {
    let (hr, bytes) = try_update(existing, Host::new(items), level, true);
    hr.unwrap();
    bytes
}

/// [`update`] with `host`, into a seekable output or not: the result and what was written.
fn try_update(
    existing: Option<Vec<u8>>,
    host: Host,
    level: Option<u32>,
    seekable: bool,
) -> (HRESULT, Vec<u8>) {
    let arc = handler(existing);
    if let Some(level) = level {
        use lol7z_api::sys::i_archive::ISetProperties;
        use windows_core::{PCWSTR, w};
        let props: ISetProperties = arc.cast().unwrap();
        let names: [PCWSTR; 1] = [w!("x")];
        let values = [PropVariant::from(level)];
        unsafe { props.SetProperties(names.as_ptr(), values.as_ptr(), 1) }.unwrap();
    }
    let out_archive: IOutArchive = arc.cast().unwrap();
    let count = host.items.len() as u32;
    let cb: IArchiveUpdateCallback = host.into();
    let sink = Shared::default();
    let out = if seekable {
        OutStream::seekable(sink.clone())
    } else {
        OutStream::new(sink.clone())
    };
    let hr = unsafe { out_archive.UpdateItems(out.as_com(), count, &cb) };
    drop(out);
    let bytes = sink.0.borrow().get_ref().clone();
    (hr, bytes)
}

#[test]
fn writes_sorted_deduplicated_3_4() {
    let bytes = update(
        None,
        vec![
            HostItem::New("0000000000000bbb", TEXT.repeat(20)),
            HostItem::New("dir/", Vec::new()),
            HostItem::New("0000000000000aaa", TEXT.repeat(20)),
            HostItem::New("data/raw.bnk", b"BKHDxxxx".to_vec()),
        ],
        None,
    );
    let (a, report) = open(bytes.clone());
    assert_eq!(report.errors | report.warnings, ErrorFlags::empty());
    assert_eq!(a.header().major, 3);
    assert_eq!(a.header().minor, 4);
    let entries: Vec<&Entry> = a.entries().collect();
    assert_eq!(entries.len(), 3);
    assert!(entries.windows(2).all(|w| w[0].hash.0 < w[1].hash.0));
    assert_eq!(entries[0].hash.0, 0xaaa);
    assert_eq!(entries[1].hash.0, 0xbbb);
    // Same bytes, one copy.
    assert_eq!(entries[0].offset, entries[1].offset);
    assert_eq!(entries[0].kind, Kind::Zstd);
    assert_eq!(
        entries[0].checksum,
        xxh3_64(&bytes[entries[0].offset as usize..][..entries[0].size as usize])
    );
    // Audio banks stay raw.
    let bnk = entries
        .iter()
        .find(|e| e.hash == hash("data/raw.bnk"))
        .unwrap();
    assert_eq!(bnk.kind, Kind::Raw);
    for i in 0..3 {
        assert_eq!(extract(&a, i).0, OperationResult::OK);
    }
    assert_eq!(extract(&a, 0).1, TEXT.repeat(20));
    // The table and data are packed: nothing between the header and the first entry.
    assert_eq!(
        entries[0].offset as usize,
        toc::HEADER_V3_SIZE + 3 * toc::ENTRY_V3_SIZE
    );
}

#[test]
fn store_level_zero() {
    let bytes = update(None, vec![HostItem::New("a", TEXT.to_vec())], Some(0));
    let (a, _) = open(bytes);
    assert_eq!(a.entries().next().unwrap().kind, Kind::Raw);
}

#[test]
fn updates_existing_copying_stored_bytes() {
    let items = [
        item(1, Kind::Gzip, TEXT),
        item(2, Kind::ZstdMulti, TEXT),
        item(3, Kind::Raw, b"gone"),
    ];
    let old = build((3, 1), &items);
    let bytes = update(
        Some(old),
        vec![
            HostItem::Keep(1),
            HostItem::New("0000000000000009", b"new".to_vec()),
            HostItem::Keep(0),
        ],
        None,
    );
    let (a, report) = open(bytes);
    assert_eq!(report.errors | report.warnings, ErrorFlags::empty());
    let entries: Vec<&Entry> = a.entries().collect();
    assert_eq!(
        entries.iter().map(|e| e.hash.0).collect::<Vec<_>>(),
        [1, 2, 9]
    );
    // Copied as stored, including the kind and subchunk count.
    assert_eq!(entries[0].kind, Kind::Gzip);
    assert_eq!(entries[1].kind, Kind::ZstdMulti);
    assert_eq!(entries[1].subchunk_count, 4);
    assert_eq!(extract(&a, 0), (OperationResult::OK, TEXT.to_vec()));
    assert_eq!(extract(&a, 1), (OperationResult::OK, TEXT.to_vec()));
    assert_eq!(extract(&a, 2), (OperationResult::OK, b"new".to_vec()));
    // Signature carried over.
    assert_eq!(a.header().signature, [0xAB; 256]);
}

#[test]
fn duplicate_hashes_refused() {
    let host = Host::new(vec![
        HostItem::New("0000000000000001", b"a".to_vec()),
        HostItem::New("x/0000000000000001", b"b".to_vec()),
    ]);
    let (hr, _) = try_update(None, host, None, true);
    assert_eq!(hr, Error::INVALID_ARG.code());
}

/// An item added under another name for a hash already there replaces the old one.
#[test]
fn new_data_replaces_copy_of_same_hash() {
    let items = [item(1, Kind::Raw, b"old"), item(2, Kind::Raw, b"two")];
    let bytes = update(
        Some(build((3, 4), &items)),
        vec![
            HostItem::Keep(0),
            HostItem::Keep(1),
            HostItem::New("moved/«0000000000000001».bin", b"new".to_vec()),
        ],
        None,
    );
    let (a, report) = open(bytes);
    assert_eq!(report.errors | report.warnings, ErrorFlags::empty());
    let hashes: Vec<u64> = a.entries().map(|e| e.hash.0).collect();
    assert_eq!(hashes, [1, 2]);
    assert_eq!(extract(&a, 0), (OperationResult::OK, b"new".to_vec()));
    assert_eq!(extract(&a, 1), (OperationResult::OK, b"two".to_vec()));
}

#[test]
fn bad_fold_refused() {
    let host = Host::new(vec![HostItem::New("d/a«_s{9…1}».bin", b"a".to_vec())]);
    let (hr, _) = try_update(None, host, None, true);
    assert_eq!(hr, Error::INVALID_ARG.code());
}

/// Entries sharing stored bytes in the source are one copy of them.
#[test]
fn copies_grouped_by_source_bytes() {
    let items = [
        item(1, Kind::Gzip, TEXT),
        item(2, Kind::Raw, b"other"),
        item(3, Kind::Gzip, TEXT),
    ];
    assert_eq!(items[0].stored, items[2].stored);
    let mut old = build((3, 4), &items);
    // The third entry points at the first one's bytes.
    let at = |i: usize| toc::HEADER_V3_SIZE + i * toc::ENTRY_V3_SIZE + 8;
    let first: [u8; 4] = old[at(0)..at(0) + 4].try_into().unwrap();
    old[at(2)..at(2) + 4].copy_from_slice(&first);
    let bytes = update(
        Some(old),
        vec![HostItem::Keep(2), HostItem::Keep(1), HostItem::Keep(0)],
        None,
    );
    let data = items[0].stored.len() + items[1].stored.len();
    assert_eq!(bytes.len(), toc::Toc::size(3) + data);
    let (a, report) = open(bytes.clone());
    assert_eq!(report.errors | report.warnings, ErrorFlags::empty());
    assert_eq!(
        a.archive_prop(PropId::PhySize).unwrap().as_u64(),
        Some(bytes.len() as u64)
    );
    let entries: Vec<&Entry> = a.entries().collect();
    assert_eq!(
        entries.iter().map(|e| e.hash.0).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(entries[0].offset, entries[2].offset);
    assert_ne!(entries[0].offset, entries[1].offset);
    assert_eq!(entries[2].kind, Kind::Gzip);
    for i in 0..3 {
        assert_eq!(extract(&a, i).0, OperationResult::OK);
    }
    assert_eq!(extract(&a, 2).1, TEXT);
}

/// A shared checksum alone doesn't make two entries one copy.
#[test]
fn copies_with_equal_checksums_keep_their_data() {
    let items = [item(1, Kind::Raw, b"aaaa"), item(2, Kind::Raw, b"bbbb")];
    let mut old = build((3, 4), &items);
    let at = |i: usize| toc::HEADER_V3_SIZE + i * toc::ENTRY_V3_SIZE + 24;
    let first: [u8; 8] = old[at(0)..at(0) + 8].try_into().unwrap();
    old[at(1)..at(1) + 8].copy_from_slice(&first);
    let bytes = update(Some(old), vec![HostItem::Keep(0), HostItem::Keep(1)], None);
    let (a, report) = open(bytes);
    assert_eq!(report.errors | report.warnings, ErrorFlags::empty());
    let entries: Vec<&Entry> = a.entries().collect();
    assert_ne!(entries[0].offset, entries[1].offset);
    assert_eq!(extract(&a, 0), (OperationResult::OK, b"aaaa".to_vec()));
    assert_eq!(extract(&a, 1), (OperationResult::OK, b"bbbb".to_vec()));
}

#[test]
fn old_index_out_of_range_refused() {
    let old = build(
        (3, 4),
        &[item(1, Kind::Raw, b"a"), item(2, Kind::Raw, b"b")],
    );
    let host = Host::new(vec![HostItem::Keep(0), HostItem::Keep(2)]);
    let (hr, _) = try_update(Some(old), host, None, true);
    assert_eq!(hr, Error::INVALID_ARG.code());
    // A copy with nothing to copy from.
    let host = Host::new(vec![HostItem::Keep(0)]);
    let (hr, _) = try_update(None, host, None, true);
    assert_eq!(hr, Error::INVALID_ARG.code());
}

#[test]
fn new_data_replaces_every_copy_of_its_hash() {
    let items = [
        item(1, Kind::Raw, b"one"),
        item(1, Kind::Raw, b"uno"),
        item(2, Kind::Raw, b"two"),
    ];
    let old = build((3, 4), &items);
    let bytes = update(
        Some(old.clone()),
        vec![
            HostItem::Keep(0),
            HostItem::Keep(1),
            HostItem::Keep(2),
            HostItem::New("«0000000000000001»", b"new".to_vec()),
        ],
        None,
    );
    let (a, report) = open(bytes);
    assert_eq!(report.errors | report.warnings, ErrorFlags::empty());
    let hashes: Vec<u64> = a.entries().map(|e| e.hash.0).collect();
    assert_eq!(hashes, [1, 2]);
    assert_eq!(extract(&a, 0), (OperationResult::OK, b"new".to_vec()));
    assert_eq!(extract(&a, 1), (OperationResult::OK, b"two".to_vec()));
    // Without the new data, two copies of one hash.
    let host = Host::new(vec![HostItem::Keep(0), HostItem::Keep(1)]);
    let (hr, _) = try_update(Some(old), host, None, true);
    assert_eq!(hr, Error::INVALID_ARG.code());
}

/// New data equal to a copied entry's stored bytes is not stored again.
#[test]
fn new_deduplicated_against_copy() {
    let items = [item(1, Kind::Raw, TEXT), item(2, Kind::Raw, b"x")];
    let bytes = update(
        Some(build((3, 4), &items)),
        vec![
            HostItem::Keep(0),
            HostItem::Keep(1),
            HostItem::New("0000000000000005", TEXT.to_vec()),
        ],
        Some(0),
    );
    let (a, report) = open(bytes);
    assert_eq!(report.errors | report.warnings, ErrorFlags::empty());
    // The test's output can't be cut, so a duplicate written last stays behind as zeroes.
    assert_eq!(
        a.archive_prop(PropId::PhySize).unwrap().as_u64(),
        Some((toc::Toc::size(3) + TEXT.len() + 1) as u64)
    );
    let entries: Vec<&Entry> = a.entries().collect();
    assert_eq!(entries[2].hash.0, 5);
    assert_eq!(entries[2].offset, entries[0].offset);
    assert_eq!(entries[2].kind, Kind::Raw);
    assert_eq!(extract(&a, 2), (OperationResult::OK, TEXT.to_vec()));
}

/// Data goes out in group order; the table still comes out sorted by hash.
#[test]
fn table_sorted_data_in_group_order() {
    let items: Vec<Item> = (1..=8)
        .map(|h| item(h, Kind::Raw, format!("data {h}").as_bytes()))
        .collect();
    let bytes = update(
        Some(build((3, 4), &items)),
        (0..8).map(HostItem::Keep).collect(),
        None,
    );
    let (a, report) = open(bytes);
    assert_eq!(report.errors | report.warnings, ErrorFlags::empty());
    let mut entries: Vec<Entry> = a.entries().copied().collect();
    assert!(entries.windows(2).all(|w| w[0].hash < w[1].hash));
    assert!(!entries.windows(2).all(|w| w[0].offset < w[1].offset));
    entries.sort_by_key(|e| e.offset);
    assert!(entries.windows(2).all(|w| w[0].checksum < w[1].checksum));
}

/// The user cancels while one large item is being read.
#[test]
fn update_cancels_inside_item() {
    let mut host = Host::new(vec![HostItem::New("big", vec![7; 10 << 20])]);
    host.abort_at = 2;
    let done = Rc::clone(&host.done);
    let (hr, _) = try_update(None, host, None, true);
    assert_eq!(hr, Error::ABORT.code());
    assert_eq!(*done.borrow(), 0);
}

/// The table goes in last, so the output must be seekable.
#[test]
fn non_seekable_output_not_implemented() {
    let host = Host::new(vec![HostItem::New("a", TEXT.to_vec())]);
    let (hr, bytes) = try_update(None, host, None, false);
    assert_eq!(hr, Error::NOT_IMPL.code());
    assert!(bytes.is_empty());
}

#[test]
fn reports_as_7zip_sees_them() {
    let _ = EventIndexType::NoIndex;
    let _ = UpdateNotifyOp::Add;
    let mut data = Vec::new();
    let mut s = InStream::new(Cursor::new(b"abc".to_vec()));
    s.read_to_end(&mut data).unwrap();
    assert_eq!(data, b"abc");
}
