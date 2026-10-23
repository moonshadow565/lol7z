//! Packages built by hand, the spec's worked example and the sample packages, read
//! through the `Format`/`Archive` traits, and written back through 7-Zip's
//! `IOutArchive` with a host written here.

use core::cell::{Cell, RefCell};
use lol7z_api::export::{Entry, Formats};
use lol7z_api::sys::i_archive::{
    IArchiveUpdateCallback, IArchiveUpdateCallback_Impl, IInArchive, IOutArchive,
};
use lol7z_api::sys::i_progress::IProgress_Impl;
use lol7z_api::sys::i_stream::ISequentialInStream;
use lol7z_api::sys::propvar::PropVariant;
use lol7z_api::{
    Archive, Error, ErrorFlags, Format, InStream, OpenCallback, OperationResult, Options,
    OutStream, PropId,
};
use lol7z_modpkg::config::{Config, ConfigAuthor, ConfigLicense};
use lol7z_modpkg::meta::{Author, License};
use lol7z_modpkg::path::{self, Chunk, Read};
use lol7z_modpkg::toc::{Compression, Layer, NONE, RECORD_SIZE, Record};
use lol7z_modpkg::{Item, Modpkg, ModpkgArchive};
use lol7z_wad::PathHash;
use serde_json::{Value, json};
use std::io::{self, Cursor, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::rc::Rc;
use windows_core::{HRESULT, Interface, OutRef, implement};
use xxhash_rust::xxh3::xxh3_64;
use xxhash_rust::xxh64::xxh64;

const TEXT: &[u8] = b"the quick brown fox jumps over the lazy dog, again and again and again";

fn game(path: &str) -> u64 {
    xxh64(path.to_ascii_lowercase().as_bytes(), 0)
}

/// One chunk of a package built by hand.
struct Spec {
    path: &'static str,
    layer: u32,
    wad: u32,
    data: Vec<u8>,
    zstd: bool,
    /// The record's hash, when not the path's.
    hash: Option<u64>,
}

fn chunk(path: &'static str, layer: u32, wad: u32, data: &[u8]) -> Spec {
    Spec {
        path,
        layer,
        wad,
        data: data.to_vec(),
        zstd: false,
        hash: None,
    }
}

fn meta(path: &'static str, data: &[u8]) -> Spec {
    chunk(path, NONE, NONE, data)
}

fn zstd(mut s: Spec) -> Spec {
    s.zstd = true;
    s
}

/// A package with the layers, WADs and chunks given, records in the order given, each
/// chunk's bytes stored once per record.
fn build(layers: &[(&str, i32)], wads: &[&str], chunks: &[Spec]) -> Vec<u8> {
    let mut v = b"_modpkg_".to_vec();
    v.extend(1u32.to_le_bytes());
    v.extend(0u32.to_le_bytes());
    v.extend((chunks.len() as u32).to_le_bytes());
    v.extend((layers.len() as u32).to_le_bytes());
    for (name, p) in layers {
        v.extend((name.len() as u32).to_le_bytes());
        v.extend(name.as_bytes());
        v.extend(p.to_le_bytes());
    }
    v.extend((chunks.len() as u32).to_le_bytes());
    for c in chunks {
        v.extend(c.path.as_bytes());
        v.push(0);
    }
    v.extend((wads.len() as u32).to_le_bytes());
    for w in wads {
        v.extend(w.as_bytes());
        v.push(0);
    }
    v.resize(v.len().div_ceil(8) * 8, 0);
    let mut data_pos = v.len() + RECORD_SIZE * chunks.len();
    let mut data = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        let stored = if c.zstd {
            zstd::bulk::compress(&c.data, 1).unwrap()
        } else {
            c.data.clone()
        };
        Record {
            path_hash: PathHash(c.hash.unwrap_or_else(|| game(c.path))),
            data_offset: data_pos as u64,
            compression: if c.zstd {
                Compression::Zstd
            } else {
                Compression::Raw
            },
            compressed_size: stored.len() as u64,
            uncompressed_size: c.data.len() as u64,
            compressed_checksum: xxh3_64(&stored),
            uncompressed_checksum: xxh3_64(&c.data),
            path_index: i as u32,
            layer_index: c.layer,
            wad_index: c.wad,
        }
        .write(&mut v)
        .unwrap();
        data_pos += stored.len();
        data.extend(stored);
    }
    v.extend(data);
    v
}

fn open(bytes: Vec<u8>) -> (ModpkgArchive, lol7z_api::OpenReport) {
    let cb = OpenCallback::none();
    let a = Modpkg::open(InStream::new(Cursor::new(bytes)), &cb, &Options::default()).unwrap();
    (a, cb.report())
}

fn try_open(bytes: Vec<u8>) -> (Result<ModpkgArchive, Error>, lol7z_api::OpenReport) {
    let cb = OpenCallback::none();
    let r = Modpkg::open(InStream::new(Cursor::new(bytes)), &cb, &Options::default());
    (r, cb.report())
}

fn extract(a: &ModpkgArchive, index: u32) -> (OperationResult, Vec<u8>) {
    let mut out = Vec::new();
    let r = a.extract_item(index, &mut out).unwrap();
    (r, out)
}

fn text(a: &ModpkgArchive, index: u32, id: PropId) -> String {
    a.item_prop(index, id).unwrap().into_string().unwrap()
}

fn paths(a: &ModpkgArchive) -> Vec<String> {
    (0..a.item_count())
        .map(|i| text(a, i, PropId::Path))
        .collect()
}

fn at(a: &ModpkgArchive, name: &str) -> u32 {
    paths(a)
        .iter()
        .position(|p| p == name)
        .unwrap_or_else(|| panic!("{name} not in {:?}", paths(a))) as u32
}

fn record<'a>(a: &'a ModpkgArchive, name: &str) -> &'a Record {
    let Item::Record(r) = a.items()[at(a, name) as usize] else {
        panic!("{name} is not a chunk")
    };
    &a.records()[r as usize]
}

/// The content of every file item, by name.
fn files(a: &ModpkgArchive) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = (0..a.item_count())
        .filter(|&i| !matches!(a.items()[i as usize], Item::Dir(_)))
        .map(|i| (text(a, i, PropId::Path), extract(a, i).1))
        .collect();
    v.sort();
    v
}

fn clean(report: &lol7z_api::OpenReport) {
    assert_eq!(
        report.errors | report.warnings,
        ErrorFlags::empty(),
        "{report:?}"
    );
}

// ---------------------------------------------------------------- reading

/// Appendix A of the format specification, as the spec prints it.
#[test]
fn reads_the_worked_example() {
    let rows = [
        "5f 6d 6f 64 70 6b 67 5f 01 00 00 00 00 00 00 00",
        "02 00 00 00 01 00 00 00 04 00 00 00 62 61 73 65",
        "00 00 00 00 02 00 00 00 5f 6d 65 74 61 5f 2f 69",
        "6e 66 6f 2e 6d 73 67 70 61 63 6b 00 44 41 54 41",
        "2f 43 68 61 72 61 63 74 65 72 73 2f 41 61 74 72",
        "6f 78 2f 53 6b 69 6e 73 2f 53 6b 69 6e 30 2f 53",
        "6b 69 6e 30 2e 62 69 6e 00 01 00 00 00 61 61 74",
        "72 6f 78 2e 77 61 64 2e 63 6c 69 65 6e 74 00 00",
        "de 5a 05 40 51 19 6f 73 fa 00 00 00 00 00 00 00",
        "00 a1 00 00 00 00 00 00 00 a1 00 00 00 00 00 00",
        "00 3a 0b 5d 1b 81 8f 8a c0 3a 0b 5d 1b 81 8f 8a",
        "c0 00 00 00 00 ff ff ff ff ff ff ff ff 35 c3 b5",
        "98 34 03 fa 35 9b 01 00 00 00 00 00 00 00 05 00",
        "00 00 00 00 00 00 05 00 00 00 00 00 00 00 fd dc",
        "62 5c 55 e8 55 95 fd dc 62 5c 55 e8 55 95 01 00",
        "00 00 00 00 00 00 00 00 00 00 88 ae 73 63 68 65",
        "6d 61 5f 76 65 72 73 69 6f 6e 03 a4 6e 61 6d 65",
        "a7 65 78 61 6d 70 6c 65 ac 64 69 73 70 6c 61 79",
        "5f 6e 61 6d 65 a7 45 78 61 6d 70 6c 65 ab 64 65",
        "73 63 72 69 70 74 69 6f 6e aa 41 6e 20 65 78 61",
        "6d 70 6c 65 a7 76 65 72 73 69 6f 6e a5 31 2e 30",
        "2e 30 ab 64 69 73 74 72 69 62 75 74 6f 72 c0 a7",
        "61 75 74 68 6f 72 73 91 82 a4 6e 61 6d 65 a7 43",
        "72 61 75 7a 65 72 a4 72 6f 6c 65 c0 a7 6c 69 63",
        "65 6e 73 65 82 a4 74 79 70 65 a4 73 70 64 78 a7",
        "73 70 64 78 5f 69 64 a3 4d 49 54 68 65 6c 6c 6f",
    ];
    let bytes: Vec<u8> = rows
        .iter()
        .flat_map(|r| r.split(' ').map(|h| u8::from_str_radix(h, 16).unwrap()))
        .collect();
    assert_eq!(bytes.len(), 0x19b + 5);
    let (a, report) = open(bytes);
    clean(&report);
    assert_eq!(
        paths(&a),
        [
            "content/base/aatrox.wad.client/DATA/Characters/Aatrox/Skins/Skin0/Skin0.bin",
            "mod.config.json",
            "content/base",
        ]
    );
    let r = &a.records()[1];
    assert_eq!(r.path_hash.0, 0x35fa033498b5c335);
    assert_eq!(r.data_offset, 411);
    assert_eq!(a.records()[0].path_hash.0, 0x736f195140055ade);
    assert_eq!(extract(&a, 0), (OperationResult::OK, b"hello".to_vec()));
    assert_eq!(a.item_prop(2, PropId::IsDir).unwrap().as_bool(), Some(true));
    assert_eq!(
        a.item_prop(0, PropId::IsDir).unwrap().as_bool(),
        Some(false)
    );
    assert_eq!(text(&a, 0, PropId::Method), "Store");
    assert_eq!(a.item_prop(0, PropId::Size).unwrap().as_u64(), Some(5));
    assert_eq!(a.archive_prop(PropId::PhySize).unwrap().as_u64(), Some(416));
    assert_eq!(
        a.archive_prop(PropId::Comment)
            .unwrap()
            .into_string()
            .unwrap(),
        "Example 1.0.0 by Crauzer; layers: base (0)"
    );
    let m = a.metadata().unwrap();
    assert_eq!(m.schema_version, Some(3));
    assert_eq!(m.name, "example");
    assert_eq!(m.description.as_deref(), Some("An example"));
    assert_eq!(
        m.license,
        License::Spdx {
            spdx_id: "MIT".into()
        }
    );
    assert_eq!(
        m.authors,
        [Author {
            name: "Crauzer".into(),
            role: None
        }]
    );
    assert!(m.layers.is_empty() && m.generator.is_none() && m.extra.is_empty());
    // The config: the document's fields in the project's shapes, the layer table's
    // layers.
    let (result, json) = extract(&a, 1);
    assert_eq!(result, OperationResult::OK);
    assert_eq!(
        a.item_prop(1, PropId::Size).unwrap().as_u64(),
        Some(json.len() as u64)
    );
    let c: Config = serde_json::from_slice(&json).unwrap();
    assert_eq!(a.config(), Some(&c));
    assert_eq!(c.name, "example");
    assert_eq!(c.display_name, "Example");
    assert_eq!(c.version, "1.0.0");
    assert_eq!(c.description, "An example");
    assert_eq!(c.authors, [ConfigAuthor::Name("Crauzer".into())]);
    assert_eq!(c.license, Some(ConfigLicense::Spdx("MIT".into())));
    assert_eq!(c.layers.len(), 1);
    assert_eq!(
        (c.layers[0].name.as_str(), c.layers[0].priority),
        ("base", 0)
    );
    assert!(c.hashtables.is_empty() && c.extra.is_empty());
    let v: Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(v["license"], json!("MIT"));
    assert_eq!(v["authors"], json!(["Crauzer"]));
    assert!(v.get("tags").is_none() && v.get("thumbnail").is_none());
}

fn three_layers() -> Vec<u8> {
    build(
        &[("base", 0), ("voice", 1), ("hud", -1)],
        &["Aatrox.wad.client", "map11.wad.client"],
        &[
            meta("_meta_/info.msgpack", b"\x80"),
            zstd(chunk("DATA/a.bin", 0, 0, TEXT)),
            chunk("overrides/a.ptch", 1, NONE, b"patch"),
            chunk("0123456789abcdef.dds", 0, 1, b"dds"),
            zstd(chunk("DATA/a.bin", 2, 0, TEXT)),
            meta("_meta_/hashes/game.hashes.txt", b""),
            meta("_meta_/readme.md", b"# hi"),
            meta("_meta_/notes.txt", b"?"),
        ],
    )
}

#[test]
fn lists_the_project_layout() {
    let (a, report) = open(three_layers());
    clean(&report);
    assert_eq!(
        paths(&a),
        [
            "content/base/Aatrox.wad.client/DATA/a.bin",
            "content/voice/overrides/a.ptch",
            "content/base/map11.wad.client/0123456789abcdef.dds",
            "content/hud/Aatrox.wad.client/DATA/a.bin",
            "hashes/game.hashes.txt",
            "README.md",
            "_meta_/notes.txt",
            "mod.config.json",
            "content/base",
            "content/voice",
            "content/hud",
        ]
    );
    assert_eq!(text(&a, 0, PropId::Method), "Zstd");
    assert_eq!(
        a.item_prop(0, PropId::PackSize).unwrap().as_u64(),
        Some(a.records()[1].compressed_size)
    );
    for i in 0..8 {
        assert_eq!(extract(&a, i).0, OperationResult::OK, "{i}");
    }
    assert_eq!(extract(&a, 0).1, TEXT);
    assert_eq!(extract(&a, 1).1, b"patch");
    assert_eq!(extract(&a, 4).1, b"");
    assert_eq!(extract(&a, 5).1, b"# hi");
    assert_eq!(
        a.archive_prop(PropId::Comment)
            .unwrap()
            .into_string()
            .unwrap(),
        "layers: base (0), voice (1), hud (-1)"
    );
    // An empty document is a config with nothing in it but the layers, base first.
    let c = a.config().unwrap();
    assert_eq!(
        c.layers
            .iter()
            .map(|l| (l.name.as_str(), l.priority))
            .collect::<Vec<_>>(),
        [("base", 0), ("hud", -1), ("voice", 1)]
    );
}

/// Without a document that reads, the package lists as it is.
#[test]
fn unreadable_document_lists_raw() {
    let (a, _) = open(build(
        &[("base", 0)],
        &["w.wad"],
        &[meta("_meta_/info.msgpack", b"\xc0"), chunk("x", 0, 0, b"1")],
    ));
    assert!(a.config().is_none());
    assert_eq!(
        paths(&a),
        [
            "_meta_/info.msgpack",
            "content/base/w.wad/x",
            "content/base"
        ]
    );
}

#[test]
fn item_stream_for_stored_only() {
    let (a, _) = open(three_layers());
    assert!(a.item_stream(0).unwrap().is_none());
    assert!(a.item_stream(1).unwrap().is_some());
    assert!(a.item_stream(7).unwrap().is_none());
    assert!(a.item_stream(8).unwrap().is_none());
}

#[test]
fn checks_what_comes_out() {
    let chunks = vec![
        zstd(chunk("a", 0, 0, TEXT)),
        chunk("b", 0, 0, TEXT),
        zstd(chunk("c", 0, 0, TEXT)),
    ];
    let mut bytes = build(&[("base", 0)], &["w"], &chunks);
    // Record 0 says one byte more than its frame decodes to; record 1's checksum is
    // wrong; record 2's frame is cut short.
    let (a, _) = open(bytes.clone());
    let toc = a.records()[0].data_offset as usize - 3 * RECORD_SIZE;
    drop(a);
    bytes[toc + 25] += 1;
    bytes[toc + RECORD_SIZE + 41] ^= 1;
    bytes[toc + 2 * RECORD_SIZE + 17] -= 4;
    let (a, _) = open(bytes);
    assert_eq!(extract(&a, 0).0, OperationResult::UnexpectedEnd);
    assert_eq!(extract(&a, 1).0, OperationResult::CRCError);
    let mut out = Vec::new();
    assert!(matches!(
        a.extract_item(2, &mut out),
        Ok(OperationResult::UnexpectedEnd | OperationResult::DataError) | Err(Error::Damaged(_))
    ));
}

#[test]
fn not_a_modpkg() {
    let (r, report) = try_open(b"RW\x03\x04 not a package at all".to_vec());
    assert_eq!(r.err(), Some(Error::NotArchive));
    assert_eq!(report.errors, ErrorFlags::empty());
    let (r, _) = try_open(b"_modp".to_vec());
    assert_eq!(r.err(), Some(Error::NotArchive));
}

#[test]
fn other_version_unsupported() {
    let mut bytes = three_layers();
    bytes[8] = 2;
    let (r, _) = try_open(bytes);
    assert_eq!(r.err(), Some(Error::Damaged(ErrorFlags::Unsupported)));
}

#[test]
fn truncated_lists_what_fits() {
    let whole = three_layers();
    let (a, _) = open(whole.clone());
    let data = a.records()[0].data_offset as usize;
    let count = a.records().len();
    drop(a);
    // Inside the records: those that fit are listed.
    let (a, report) = open(whole[..data - RECORD_SIZE - 1].to_vec());
    assert_eq!(a.records().len(), count - 2);
    assert_eq!(report.errors, ErrorFlags::UnexpectedEnd);
    // Inside the data: everything is listed, the cut chunk fails to extract.
    let (a, report) = open(whole[..whole.len() - 2].to_vec());
    assert_eq!(a.records().len(), count);
    assert_eq!(report.errors, ErrorFlags::UnexpectedEnd);
    assert_eq!(
        a.archive_prop(PropId::PhySize).unwrap().as_u64(),
        Some(whole.len() as u64 - 2)
    );
    // Inside the tables: nothing to list.
    let (r, _) = try_open(whole[..40].to_vec());
    assert_eq!(r.err(), Some(Error::UNEXPECTED_END));
}

#[test]
fn bad_records_refused() {
    let (a, _) = open(three_layers());
    let count = a.records().len();
    let toc = a.records()[0].data_offset as usize - count * RECORD_SIZE;
    drop(a);
    for (at, value, why) in [
        (16, 7u8, "unknown compression"),
        (49, 9, "path index past the path table"),
        (53, 3, "layer index past the layer table"),
        (57, 2, "WAD index past the WAD table"),
    ] {
        let mut bytes = three_layers();
        bytes[toc + RECORD_SIZE + at] = value;
        let (r, report) = try_open(bytes);
        assert_eq!(r.err(), Some(Error::HEADERS_ERROR), "{why}");
        assert_eq!(report.error.as_deref(), Some(why));
    }
}

#[test]
fn odd_packages_warn() {
    let cases: [(Vec<u8>, &str); 5] = [
        (
            build(&[("main", 0)], &["w"], &[chunk("a", 0, 0, b"1")]),
            "no base layer",
        ),
        (
            build(&[("base", 0), ("Bad Name", 1)], &["w"], &[]),
            "layer name not a slug",
        ),
        (
            build(&[("base", 0)], &["w"], &[chunk("../a", 0, 0, b"1")]),
            "name escapes the output directory",
        ),
        (
            build(&[("base", 0)], &["w"], &[chunk("_meta_/x", 0, 0, b"1")]),
            "chunk under _meta_ with a layer or WAD",
        ),
        (
            build(
                &[("base", 0)],
                &["w", "v"],
                &[chunk("a", 0, 0, b"1"), chunk("A", 0, 1, b"22")],
            ),
            "chunks of one path and layer differ",
        ),
    ];
    for (bytes, why) in cases {
        let (a, report) = open(bytes);
        assert_eq!(report.warnings, ErrorFlags::HeadersError, "{why}");
        assert_eq!(report.warning.as_deref(), Some(why));
        assert!(a.item_count() > 0);
    }
    let (_, report) = open(build(
        &[("base", 0)],
        &["w"],
        &[chunk("a", 0, 0, b"1"), chunk("a", 0, 0, b"1")],
    ));
    assert_eq!(report.warning.as_deref(), Some("duplicate chunk records"));
    let (_, report) = open(build(&[("base", 0)], &[], &[chunk("a", NONE, NONE, b"1")]));
    assert_eq!(
        report.warning.as_deref(),
        Some("chunk without a layer outside _meta_")
    );
}

#[test]
fn long_names_fold() {
    let long = format!(
        "data/multi_{}.bin",
        (0..60)
            .map(|i| format!("_skins_skin{i}"))
            .collect::<String>()
    );
    let long: &'static str = Box::leak(long.into_boxed_str());
    let (a, report) = open(build(
        &[("base", 0)],
        &["w.wad"],
        &[chunk(long, 0, 0, b"1")],
    ));
    clean(&report);
    let shown = paths(&a)[0].clone();
    assert!(
        shown.starts_with("content/base/w.wad/data/multi_«"),
        "{shown}"
    );
    assert!(shown.encode_utf16().count() < 300);
    match path::read(&shown).unwrap() {
        Read::Chunk(c) => assert_eq!(c.path, long),
        other => panic!("{other:?}"),
    }
}

#[test]
fn mapped_characters_read_back() {
    let (a, report) = open(build(
        &[("base", 0)],
        &["w.wad"],
        &[chunk("a?b", 0, 0, b"1")],
    ));
    assert_eq!(report.warnings, ErrorFlags::empty());
    let shown = paths(&a)[0].clone();
    assert_eq!(shown, "content/base/w.wad/a\u{F03F}b");
    match path::read(&shown).unwrap() {
        Read::Chunk(c) => assert_eq!(c.path, "a?b"),
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------- names

#[test]
fn reads_names_back() {
    let c = |path: &str, layer: Option<&str>, wad: Option<&str>| {
        Read::Chunk(Chunk {
            path: path.into(),
            hash: PathHash(game(path)),
            layer: layer.map(Into::into),
            wad: wad.map(Into::into),
        })
    };
    let cases = [
        (
            "content/base/Aatrox.wad.client/DATA/A.bin",
            c("DATA/A.bin", Some("base"), Some("aatrox.wad.client")),
        ),
        (
            "content\\voice\\yone.wad\\x",
            c("x", Some("voice"), Some("yone.wad")),
        ),
        (
            "content/hi-res_2/overrides/a.ptch",
            c("overrides/a.ptch", Some("hi-res_2"), None),
        ),
        (
            "content/base/a.wad.client/dir/0123456789abcdef.dds",
            c(
                "dir/0123456789abcdef.dds",
                Some("base"),
                Some("a.wad.client"),
            ),
        ),
        ("README.md", c("_meta_/readme.md", None, None)),
        ("readme.MD", c("_meta_/readme.md", None, None)),
        ("LICENSE", c("_meta_/license", None, None)),
        ("LICENSE.txt", c("_meta_/license", None, None)),
        ("thumbnail.webp", c("_meta_/thumbnail.webp", None, None)),
        (
            "hashes/game.hashes.txt",
            c("_meta_/hashes/game.hashes.txt", None, None),
        ),
        ("_meta_/notes.txt", c("_meta_/notes.txt", None, None)),
        ("_meta_/info.msgpack", c("_meta_/info.msgpack", None, None)),
        ("mod.config.json", Read::Config),
        (".modignore", Read::Ignored),
        ("content", Read::Folder),
        ("content/", Read::Folder),
        ("_meta_", Read::Folder),
        ("hashes", Read::Folder),
        ("content/x", Read::Layer("x".into())),
        ("content/x/", Read::Layer("x".into())),
        ("content/x/a.wad.client", Read::Folder),
        ("content/x/game_data.json", Read::GameData("x".into())),
    ];
    for (name, want) in cases {
        assert_eq!(path::read(name).unwrap(), want, "{name}");
    }
    // A hex name at a WAD root is its hash, lowercased.
    let Read::Chunk(c) = path::read("content/base/w.wad/0123456789ABCDEF.DDS").unwrap() else {
        panic!()
    };
    assert_eq!(c.hash.0, 0x0123456789abcdef);
    assert_eq!(c.path, "0123456789abcdef.dds");
    // Blocks the listing writes.
    let Read::Chunk(c) = path::read("content/base/w.wad/«00000000000000ab».bin").unwrap() else {
        panic!()
    };
    assert_eq!((c.hash.0, c.path.as_str()), (0xab, "00000000000000ab.bin"));
    let Read::Chunk(c) =
        path::read("content/base/w.wad/pre«00000000000000ab-00000000000000cd».bin").unwrap()
    else {
        panic!()
    };
    assert_eq!((c.hash.0, c.path.as_str()), (0xab, "00000000000000ab.bin"));
    let Read::Chunk(c) = path::read("content/base/w.wad/a\u{F02F}").unwrap() else {
        panic!()
    };
    assert_eq!(c.path, "a");
    let Read::Chunk(c) = path::read("content/base/w.wad/a/\u{F02F}/b").unwrap() else {
        panic!()
    };
    assert_eq!(c.path, "a//b");
}

#[test]
fn bad_names_refused() {
    for name in [
        "elsewhere/x",
        "mod.config.toml",
        "content/Base/x",
        "content/-x/x",
        "content/base/_meta_/x",
        "content/base/w.wad/../x",
        "content/base/w.wad/a:b",
        "content/base/w.wad/a«_s{9…1}».bin",
        "hashes/a/b",
        "",
    ] {
        assert_eq!(path::read(name).err(), Some(Error::INVALID_ARG), "{name}");
    }
}

// ---------------------------------------------------------------- samples

fn samples() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/sample-modpkg");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "modpkg"))
        .collect();
    v.sort();
    v
}

#[test]
fn reads_samples() {
    for sample in samples() {
        let bytes = std::fs::read(&sample).unwrap();
        let (a, report) = open(bytes);
        clean(&report);
        let names = paths(&a);
        assert!(names.contains(&"mod.config.json".to_string()));
        assert!(names.contains(&"README.md".to_string()));
        assert!(names.contains(&"hashes/game.hashes.txt".to_string()));
        assert!(names.contains(&"content/base/game_data.json".to_string()));
        assert!(!names.iter().any(|n| n.starts_with("_meta_/")), "{names:?}");
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("content/base/") && n.contains(".wad.client/"))
        );
        let m = a.metadata().unwrap();
        assert_eq!(m.schema_version, Some(5));
        assert!(!m.display_name.is_empty() && !m.authors.is_empty(), "{m:?}");
        assert_eq!(m.layers.len(), a.tables().layers.len());
        let c = a.config().unwrap();
        assert!(
            c.hashtables
                .iter()
                .all(|h| h.path.starts_with("hashes/") && h.bits > 0)
        );
        assert_eq!(c.layers.len(), a.tables().layers.len());
        let comment = a
            .archive_prop(PropId::Comment)
            .unwrap()
            .into_string()
            .unwrap();
        assert!(comment.starts_with(&m.display_name), "{comment}");
        for i in 0..a.item_count() {
            if !matches!(a.items()[i as usize], Item::Dir(_)) {
                assert_eq!(
                    extract(&a, i).0,
                    OperationResult::OK,
                    "{sample:?} {}",
                    names[i as usize]
                );
            }
        }
    }
}

/// What the reference importer wrote for one sample, when that checkout is around.
#[test]
fn matches_the_reference_import() {
    let project = PathBuf::from(r"C:\DevLtk\ltk-reforge\data\workspace\EvanesciaYone-s0jaku");
    let sample = samples().into_iter().find(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("EvanesciaYone"))
    });
    let (Some(sample), true) = (sample, project.join("mod.config.json").is_file()) else {
        return;
    };
    let (a, _) = open(std::fs::read(sample).unwrap());
    let theirs: Value =
        serde_json::from_slice(&std::fs::read(project.join("mod.config.json")).unwrap()).unwrap();
    let ours: Value = serde_json::from_slice(&extract(&a, at(&a, "mod.config.json")).1).unwrap();
    assert_eq!(ours, theirs);
    for layer in ["base", "voice", "hud"] {
        let file = project.join("content").join(layer).join("game_data.json");
        let theirs: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        let name = format!("content/{layer}/game_data.json");
        let ours: Value = serde_json::from_slice(&extract(&a, at(&a, &name)).1).unwrap();
        // The reference's manifest writer prints an empty `links`; the document omits it.
        let trim = |mut v: Value| {
            for m in v["modules"].as_array_mut().unwrap() {
                for e in m["edits"].as_array_mut().unwrap() {
                    if e["links"] == json!([]) {
                        e.as_object_mut().unwrap().remove("links");
                    }
                }
            }
            v
        };
        assert_eq!(trim(ours), trim(theirs), "{name}");
    }
    for name in ["README.md", "thumbnail.webp", "hashes/game.hashes.txt"] {
        let theirs = std::fs::read(project.join(name)).unwrap();
        assert_eq!(extract(&a, at(&a, name)).1, theirs, "{name}");
    }
}

#[test]
fn round_trips_samples() {
    for sample in samples() {
        let old = std::fs::read(&sample).unwrap();
        let (a, _) = open(old.clone());
        let count = a.item_count();
        let expect = files(&a);
        let layers = a.tables().layers.clone();
        let mut metadata = a.metadata().unwrap().clone();
        drop(a);
        let bytes = update(Some(old), (0..count).map(HostItem::Keep).collect(), None);
        let (b, report) = open(bytes);
        clean(&report);
        assert_eq!(b.tables().layers, layers);
        assert_eq!(files(&b), expect, "{sample:?}");
        // The document is written again from the config: the same but for the
        // generator.
        metadata.generator = b.metadata().unwrap().generator.clone();
        assert_eq!(b.metadata(), Some(&metadata), "{sample:?}");
        // Meta chunks first, then by WAD, layer and hash.
        let key = |r: &Record| {
            (
                r.wad_index.wrapping_add(1),
                r.layer_index.wrapping_add(1),
                r.path_hash,
            )
        };
        assert!(b.records().windows(2).all(|w| key(&w[0]) <= key(&w[1])));
    }
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
    /// Kept data under a new name.
    Rename(u32, &'static str),
}

/// A config declaring `layers`, with hashtable declarations `hashes/<file>` of
/// category `game`.
fn config(layers: &[(&str, i32)], hashes: &[&str]) -> HostItem {
    let layers: Vec<Value> = layers
        .iter()
        .map(|(n, p)| json!({"name": n, "priority": p}))
        .collect();
    let hashes: Vec<Value> = hashes
        .iter()
        .map(|f| json!({"path": format!("hashes/{f}"), "category": "game", "algorithm": "xxh64", "bits": 64}))
        .collect();
    let mut c = json!({
        "name": "test-mod",
        "display_name": "Test Mod",
        "version": "1.2.3",
        "description": "made by the tests",
        "authors": ["tester", {"name": "other", "role": "Artist"}],
        "license": "MIT",
        "champions": ["Aatrox"],
    });
    if !layers.is_empty() {
        c["layers"] = Value::Array(layers);
    }
    if !hashes.is_empty() {
        c["hashtables"] = Value::Array(hashes);
    }
    HostItem::New("mod.config.json", serde_json::to_vec_pretty(&c).unwrap())
}

#[implement(IArchiveUpdateCallback)]
struct Host {
    items: Vec<HostItem>,
    done: Rc<RefCell<u32>>,
    calls: Cell<u32>,
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
        let (data, props, old) = match &self.items[index as usize] {
            HostItem::Keep(old) => (0, 0, *old),
            HostItem::New(..) => (1, 1, u32::MAX),
            HostItem::Rename(old, _) => (0, 1, *old),
        };
        new_data.write(data).unwrap();
        new_props.write(props).unwrap();
        index_in_archive.write(old).unwrap();
        HRESULT(0)
    }
    unsafe fn GetProperty(
        &self,
        index: u32,
        id: PropId,
        value: OutRef<'_, PropVariant>,
    ) -> HRESULT {
        let (path, data) = match &self.items[index as usize] {
            HostItem::New(path, data) => (*path, Some(data)),
            HostItem::Rename(_, path) => (*path, None),
            HostItem::Keep(_) => return Error::INVALID_ARG.code(),
        };
        let v = match id {
            PropId::Path => path.into(),
            PropId::IsDir => path.ends_with('/').into(),
            PropId::Size => (data.map_or(0, Vec::len) as u64).into(),
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
static FORMATS: Formats<'static> = Formats(&[Entry::update::<Modpkg>()]);

fn handler(bytes: Option<Vec<u8>>) -> IInArchive {
    let mut p = core::ptr::null_mut();
    unsafe { FORMATS.create_object(&Modpkg::CLASS_ID, &IInArchive::IID, &mut p) }.unwrap();
    let arc = unsafe { IInArchive::from_raw(p) };
    if let Some(bytes) = bytes {
        let stream = InStream::new(Cursor::new(bytes));
        unsafe { arc.Open(stream.as_com(), None, None) }.unwrap();
    }
    arc
}

/// Runs `UpdateItems` with `items` against `existing`, with `-mx=level` when given.
fn update(existing: Option<Vec<u8>>, items: Vec<HostItem>, level: Option<u32>) -> Vec<u8> {
    let (hr, bytes) = try_update(existing, Host::new(items), level, true);
    hr.unwrap();
    bytes
}

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

fn refused(existing: Option<Vec<u8>>, items: Vec<HostItem>) {
    let (hr, _) = try_update(existing, Host::new(items), None, true);
    assert_eq!(hr, Error::INVALID_ARG.code());
}

#[test]
fn writes_a_package() {
    let game_data = json!({"version": 1, "modules": [{"target": "data/x.bin", "edits": []}]});
    // Stored with the origin the reference packer gives a module; shown without it.
    let stored = json!({"version": 1, "modules": [{"target": "data/x.bin", "edits": [],
        "origin": {"manifest": "game_data.json", "source": null, "module": 0}}]});
    let bytes = update(
        None,
        vec![
            config(
                &[("base", 0), ("voice", 2), ("hud", 1)],
                &["game.hashes.txt"],
            ),
            HostItem::New("content/base/Aatrox.wad.client/DATA/x.bin", TEXT.repeat(20)),
            HostItem::New("content/base/Aatrox.wad.client/DATA/", Vec::new()),
            HostItem::New("content/voice/", Vec::new()),
            HostItem::New("content/base/overrides/a.ptch", b"patch".to_vec()),
            HostItem::New("content/hud/aatrox.wad.client/DATA/x.bin", TEXT.repeat(20)),
            HostItem::New(
                "content/base/aatrox.wad.client/0123456789abcdef.dds",
                b"dds".to_vec(),
            ),
            HostItem::New("content/base/aatrox.wad.client/sfx/a.bnk", TEXT.repeat(21)),
            HostItem::New(
                "content/base/game_data.json",
                serde_json::to_vec(&game_data).unwrap(),
            ),
            HostItem::New("README.md", b"# hi".to_vec()),
            HostItem::New("LICENSE.txt", b"MIT".to_vec()),
            HostItem::New("hashes/game.hashes.txt", b"data/x.bin\n".to_vec()),
            HostItem::New(".modignore", b"*.bak".to_vec()),
        ],
        None,
    );
    let (a, report) = open(bytes.clone());
    clean(&report);
    assert_eq!(a.header().version, 1);
    assert_eq!(a.header().signature_size, 0);
    let t = a.tables();
    assert_eq!(
        t.layers,
        [("base", 0), ("hud", 1), ("voice", 2)].map(|(n, p)| Layer {
            name: n.into(),
            priority: p
        })
    );
    assert_eq!(t.wads, ["aatrox.wad.client"]);
    assert_eq!(a.records().len(), 9);
    // Meta first, then by WAD, layer, hash.
    let names = paths(&a);
    let mut metas = names[..3].to_vec();
    metas.sort();
    assert_eq!(metas, ["LICENSE", "README.md", "hashes/game.hashes.txt"]);
    assert_eq!(names[3], "content/base/overrides/a.ptch");
    assert!(
        names[4..8]
            .iter()
            .all(|n| n.starts_with("content/") && n.contains("aatrox.wad.client/")),
        "{names:?}"
    );
    assert_eq!(
        names[8..],
        [
            "mod.config.json",
            "content/base/game_data.json",
            "content/base",
            "content/hud",
            "content/voice"
        ]
    );
    // Equal content under two layers is one run; a bank stays raw; a hex name is its hash.
    let x = record(&a, "content/base/aatrox.wad.client/DATA/x.bin");
    let y = record(&a, "content/hud/aatrox.wad.client/DATA/x.bin");
    assert_eq!(x.compression, Compression::Zstd);
    assert_eq!(x.data_offset, y.data_offset);
    assert_eq!(x.path_index, y.path_index);
    assert_eq!(
        x.compressed_checksum,
        xxh3_64(&bytes[x.data_offset as usize..][..x.compressed_size as usize])
    );
    assert_eq!(x.uncompressed_checksum, xxh3_64(&TEXT.repeat(20)));
    let bnk = record(&a, "content/base/aatrox.wad.client/sfx/a.bnk");
    assert_eq!(bnk.compression, Compression::Raw);
    assert_eq!(bnk.compressed_size, TEXT.len() as u64 * 21);
    let hex = record(&a, "content/base/aatrox.wad.client/0123456789abcdef.dds");
    assert_eq!(hex.path_hash.0, 0x0123456789abcdef);
    // The document, written from the config.
    let m = a.metadata().unwrap();
    assert_eq!(m.schema_version, Some(5));
    assert_eq!(m.name, "test-mod");
    assert_eq!(m.description.as_deref(), Some("made by the tests"));
    assert_eq!(
        m.license,
        License::Spdx {
            spdx_id: "MIT".into()
        }
    );
    assert_eq!(m.authors[1].role.as_deref(), Some("Artist"));
    assert_eq!(
        m.layers
            .iter()
            .map(|l| (l.name.as_str(), l.priority))
            .collect::<Vec<_>>(),
        [("base", 0), ("hud", 1), ("voice", 2)]
    );
    assert_eq!(m.layers[0].game_data, Some(stored));
    assert_eq!(m.layers[1].game_data, None);
    assert_eq!(m.hashtables.len(), 1);
    assert_eq!(m.hashtables[0].path, "_meta_/hashes/game.hashes.txt");
    assert!(m.generator.as_deref().unwrap().starts_with("lol7z_modpkg "));
    let info = a
        .records()
        .iter()
        .find(|r| r.is_meta() && a.tables().paths[r.path_index as usize] == "_meta_/info.msgpack")
        .unwrap();
    assert_eq!(info.compression, Compression::Raw);
    // Nil for an absent distributor, as the reference writer writes.
    let doc = &bytes[info.data_offset as usize..][..info.compressed_size as usize];
    assert!(doc.windows(12).any(|w| w == b"distributor\xc0"));
    // The config shown is the one given, plus what the package adds.
    let shown: Value = serde_json::from_slice(&extract(&a, at(&a, "mod.config.json")).1).unwrap();
    assert_eq!(shown["name"], json!("test-mod"));
    assert_eq!(
        shown["authors"],
        json!(["tester", {"name": "other", "role": "Artist"}])
    );
    assert_eq!(shown["license"], json!("MIT"));
    assert_eq!(
        shown["hashtables"][0]["path"],
        json!("hashes/game.hashes.txt")
    );
    let shown_data: Value =
        serde_json::from_slice(&extract(&a, at(&a, "content/base/game_data.json")).1).unwrap();
    assert_eq!(shown_data, game_data);
    for i in 0..10 {
        assert_eq!(
            extract(&a, i).0,
            OperationResult::OK,
            "{}",
            names[i as usize]
        );
    }
    assert_eq!(
        extract(&a, at(&a, "content/base/overrides/a.ptch")).1,
        b"patch"
    );
    assert_eq!(extract(&a, at(&a, "LICENSE")).1, b"MIT");
    // Packed: data starts right after the records.
    let first = a.records().iter().map(|r| r.data_offset).min().unwrap();
    assert_eq!(
        first as usize,
        (20 + t.size() as usize).div_ceil(8) * 8 + 9 * RECORD_SIZE
    );
}

#[test]
fn config_required_and_checked() {
    let item = || HostItem::New("content/base/w.wad/a", b"a".to_vec());
    refused(None, vec![item()]);
    for bad in [
        json!({"display_name": "X", "version": "1"}),
        json!({"name": "x", "display_name": "", "version": "1"}),
        json!({"name": "x", "display_name": "X"}),
        json!({"name": "x", "display_name": "X", "version": "1", "layers": [{"name": "Bad"}]}),
        json!({"name": "x", "display_name": "X", "version": "1", "layers": [{"name": "base", "priority": 3}]}),
        json!({"name": "x", "display_name": "X", "version": "1", "hashtables": [{"path": "elsewhere/x.txt"}]}),
        json!([]),
    ] {
        let config = HostItem::New("mod.config.json", serde_json::to_vec(&bad).unwrap());
        refused(None, vec![config, item()]);
    }
    let minimal = json!({"name": "x", "display_name": "X", "version": "1"});
    let config = HostItem::New("mod.config.json", serde_json::to_vec(&minimal).unwrap());
    let bytes = update(None, vec![config, item()], Some(0));
    let (a, report) = open(bytes);
    clean(&report);
    assert_eq!(
        a.tables().layers,
        [Layer {
            name: "base".into(),
            priority: 0
        }]
    );
    let m = a.metadata().unwrap();
    assert_eq!((m.name.as_str(), m.version.as_str()), ("x", "1"));
    assert_eq!(m.description.as_deref(), Some(""));
    assert!(m.authors.is_empty() && m.license == License::None);
}

/// Without a layer list, `base` is the only layer; with one, every folder must be in it.
#[test]
fn layers_must_be_declared() {
    let minimal = || {
        HostItem::New(
            "mod.config.json",
            serde_json::to_vec(&json!({"name": "x", "display_name": "X", "version": "1"})).unwrap(),
        )
    };
    refused(
        None,
        vec![
            minimal(),
            HostItem::New("content/extra/w.wad/a", b"a".to_vec()),
        ],
    );
    refused(
        None,
        vec![minimal(), HostItem::New("content/extra/", Vec::new())],
    );
    refused(
        None,
        vec![
            config(&[("base", 0)], &[]),
            HostItem::New("content/other/", Vec::new()),
        ],
    );
    // A layer folder as a file, or a config as a folder.
    refused(
        None,
        vec![minimal(), HostItem::New("content/base", b"?".to_vec())],
    );
    refused(None, vec![HostItem::New("mod.config.json/", Vec::new())]);
}

#[test]
fn hashtables_must_match_the_config() {
    let table = || HostItem::New("hashes/game.hashes.txt", b"x\n".to_vec());
    refused(None, vec![config(&[], &[]), table()]);
    refused(None, vec![config(&[], &["game.hashes.txt"])]);
    refused(None, vec![config(&[], &["other.hashes.txt"]), table()]);
    let bytes = update(
        None,
        vec![config(&[], &["game.hashes.txt"]), table()],
        Some(0),
    );
    let (a, _) = open(bytes);
    assert_eq!(
        a.metadata().unwrap().hashtables[0].path,
        "_meta_/hashes/game.hashes.txt"
    );
    assert_eq!(
        a.config().unwrap().hashtables[0].path,
        "hashes/game.hashes.txt"
    );
}

#[test]
fn unknown_keys_survive() {
    let doc = json!({
        "name": "x", "display_name": "X", "version": "1",
        "future": {"a": [1, 2]},
        "layers": [{"name": "base", "priority": 0, "later": true}],
        "distributor": {"site_id": "s", "site_name": "S", "site_url": "u", "mod_id": "m"},
    });
    let config = HostItem::New("mod.config.json", serde_json::to_vec(&doc).unwrap());
    let bytes = update(None, vec![config], Some(0));
    let (a, _) = open(bytes.clone());
    let m = a.metadata().unwrap().clone();
    assert_eq!(m.extra.get("future"), Some(&json!({"a": [1, 2]})));
    assert_eq!(m.layers[0].extra.get("later"), Some(&json!(true)));
    assert_eq!(
        m.distributor.as_ref().map(|d| d.site_id.as_str()),
        Some("s")
    );
    let shown: Value = serde_json::from_slice(&extract(&a, at(&a, "mod.config.json")).1).unwrap();
    assert_eq!(shown["future"], json!({"a": [1, 2]}));
    assert_eq!(shown["distributor"]["mod_id"], json!("m"));
    assert_eq!(shown["layers"][0]["later"], json!(true));
    // And again through a plain update.
    let n = a.item_count();
    drop(a);
    let (b, _) = open(update(
        Some(bytes),
        (0..n).map(HostItem::Keep).collect(),
        None,
    ));
    assert_eq!(b.metadata().unwrap().extra, m.extra);
    assert_eq!(b.metadata().unwrap().distributor, m.distributor);
}

#[test]
fn store_level_zero() {
    let bytes = update(
        None,
        vec![
            config(&[], &[]),
            HostItem::New("content/base/w.wad.client/a", TEXT.to_vec()),
        ],
        Some(0),
    );
    let (a, _) = open(bytes);
    assert_eq!(
        record(&a, "content/base/w.wad.client/a").compression,
        Compression::Raw
    );
}

#[test]
fn updates_existing_copying_stored_bytes() {
    let old = three_layers();
    let (a, _) = open(old.clone());
    let n = a.item_count();
    let gone = at(&a, "content/voice/overrides/a.ptch");
    // The package's own document is empty, so the config comes from the host.
    let old_config = at(&a, "mod.config.json");
    drop(a);
    let mut items: Vec<HostItem> = (0..n)
        .filter(|&i| i != gone && i != old_config)
        .map(HostItem::Keep)
        .collect();
    items.push(config(&[("voice", 1), ("hud", -1)], &["game.hashes.txt"]));
    items.push(HostItem::New(
        "content/voice/map11.wad.client/new.bin",
        b"new".to_vec(),
    ));
    let bytes = update(Some(old), items, None);
    let (b, report) = open(bytes);
    clean(&report);
    assert_eq!(b.tables().layers.len(), 3);
    assert_eq!(b.records().len(), 8);
    let x = record(&b, "content/base/aatrox.wad.client/DATA/a.bin");
    assert_eq!(x.compression, Compression::Zstd);
    assert_eq!(
        extract(&b, at(&b, "content/base/aatrox.wad.client/DATA/a.bin")),
        (OperationResult::OK, TEXT.to_vec())
    );
    assert_eq!(
        extract(&b, at(&b, "content/voice/map11.wad.client/new.bin")),
        (OperationResult::OK, b"new".to_vec())
    );
    assert!(!paths(&b).contains(&"content/voice/overrides/a.ptch".to_string()));
    // Copies of one run in the source share one run here.
    let y = record(&b, "content/hud/aatrox.wad.client/DATA/a.bin");
    assert_eq!(x.data_offset, y.data_offset);
    assert_eq!(b.signature(), b"");
    // The config declared the three layers, so the one left empty survives.
    assert!(b.config().unwrap().layers.iter().any(|l| l.name == "voice"));
}

/// A kept config is checked like a new one: a package with an empty document can't
/// be updated without a config from the host.
#[test]
fn kept_config_is_checked() {
    let old = three_layers();
    let (a, _) = open(old.clone());
    let n = a.item_count();
    drop(a);
    refused(Some(old), (0..n).map(HostItem::Keep).collect());
}

#[test]
fn empty_layer_survives() {
    let bytes = update(
        None,
        vec![
            config(&[("extra", 5)], &[]),
            HostItem::New("content/extra/", Vec::new()),
        ],
        None,
    );
    let (a, _) = open(bytes.clone());
    assert_eq!(
        a.tables().layers[1],
        Layer {
            name: "extra".into(),
            priority: 5
        }
    );
    assert_eq!(paths(&a)[1..], ["content/base", "content/extra"]);
    let n = a.item_count();
    drop(a);
    let bytes = update(Some(bytes), (0..n).map(HostItem::Keep).collect(), None);
    let (a, _) = open(bytes);
    assert_eq!(a.tables().layers.len(), 2);
}

#[test]
fn renamed_copy_moves_layer() {
    let old = build(&[("base", 0)], &["w.wad"], &[zstd(chunk("a", 0, 0, TEXT))]);
    let bytes = update(
        Some(old),
        vec![
            config(&[("alt", 1)], &[]),
            HostItem::Rename(0, "content/alt/w.wad/b"),
        ],
        None,
    );
    let (a, report) = open(bytes);
    clean(&report);
    assert_eq!(
        paths(&a),
        [
            "content/alt/w.wad/b",
            "mod.config.json",
            "content/base",
            "content/alt"
        ]
    );
    let r = record(&a, "content/alt/w.wad/b");
    assert_eq!(r.path_hash.0, game("b"));
    assert_eq!(r.compression, Compression::Zstd);
    assert_eq!(extract(&a, 0), (OperationResult::OK, TEXT.to_vec()));
}

#[test]
fn new_data_replaces_copies_under_every_wad() {
    let old = build(
        &[("base", 0)],
        &["w.wad", "v.wad"],
        &[
            chunk("a", 0, 0, b"old"),
            chunk("a", 0, 1, b"old"),
            chunk("b", 0, 0, b"bee"),
        ],
    );
    let bytes = update(
        Some(old),
        vec![
            config(&[], &[]),
            HostItem::Keep(0),
            HostItem::Keep(1),
            HostItem::Keep(2),
            HostItem::New("content/base/w.wad/a", b"new".to_vec()),
        ],
        Some(0),
    );
    let (a, report) = open(bytes);
    clean(&report);
    assert_eq!(a.records().len(), 4);
    let w = record(&a, "content/base/w.wad/a");
    let v = record(&a, "content/base/v.wad/a");
    assert_eq!(w.data_offset, v.data_offset);
    assert_eq!(extract(&a, at(&a, "content/base/v.wad/a")).1, b"new");
}

#[test]
fn conflicts_refused() {
    refused(
        None,
        vec![
            config(&[], &[]),
            HostItem::New("content/base/w.wad/a", b"1".to_vec()),
            HostItem::New("content/base/w.wad/A", b"2".to_vec()),
        ],
    );
    for name in ["elsewhere", "content/base/w.wad/..", "mod.config.toml"] {
        refused(
            None,
            vec![config(&[], &[]), HostItem::New(name, b"1".to_vec())],
        );
    }
    let old = build(&[("base", 0)], &["w.wad"], &[chunk("a", 0, 0, b"1")]);
    refused(
        Some(old),
        vec![config(&[], &[]), HostItem::Keep(0), HostItem::Keep(5)],
    );
}

#[test]
fn update_cancels_inside_item() {
    let mut host = Host::new(vec![
        config(&[], &[]),
        HostItem::New("content/base/w.wad/big", vec![7; 10 << 20]),
    ]);
    host.abort_at = 2;
    let done = Rc::clone(&host.done);
    let (hr, _) = try_update(None, host, None, true);
    assert_eq!(hr, Error::ABORT.code());
    // Only the config was read.
    assert_eq!(*done.borrow(), 1);
}

#[test]
fn non_seekable_output_not_implemented() {
    let host = Host::new(vec![
        config(&[], &[]),
        HostItem::New("content/base/w.wad/a", TEXT.to_vec()),
    ]);
    let (hr, bytes) = try_update(None, host, None, false);
    assert_eq!(hr, Error::NOT_IMPL.code());
    assert!(bytes.is_empty());
}
