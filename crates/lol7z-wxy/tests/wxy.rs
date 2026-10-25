//! Packages built by hand in both layouts, and the sample packages, read through the
//! `Format`/`Archive` traits.

use flate2::Compression;
use flate2::write::{DeflateEncoder, ZlibEncoder};
use lol7z_api::{
    Archive, Error, ErrorFlags, Format, InStream, OpenCallback, OpenReport, OperationResult,
    Options, PropId,
};
use lol7z_wxy::toc::{Method, OINK_VERSION, deploy_hash};
use lol7z_wxy::{Item, Wxy, WxyArchive};
use std::io::{Cursor, Write};
use std::path::PathBuf;

const TEXT: &[u8] = b"the quick brown fox jumps over the lazy dog, again and again and again";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR not really a png";

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}

fn mask(bytes: &mut [u8], a: u32, b: u32) {
    let n = bytes.len() as u32;
    for (i, c) in (0u32..).zip(bytes.iter_mut()) {
        *c ^= (i * a + (n - i) * b) as u8;
    }
}

struct File {
    project: usize,
    path: &'static str,
    data: &'static [u8],
    method: Method,
}

fn file(project: usize, path: &'static str, data: &'static [u8], method: Method) -> File {
    File {
        project,
        path,
        data,
        method,
    }
}

/// An `OINK` package: version 6 or 7.
fn build_oink(version: i32, projects: &[&str], files: &[File], previews: &[&[u8]]) -> Vec<u8> {
    let mut v = b"WXYS".to_vec();
    v.extend(version.to_le_bytes());
    v.extend(b"OINK");
    let strings = ["Bard Malfurious", "someone", "1.0", "Champions", "Bard"];
    let packed: Vec<Vec<u8>> = strings
        .iter()
        .map(|s| {
            let mut p = deflate(s.as_bytes());
            if version > 6 {
                mask(&mut p, 42, 17);
            }
            p
        })
        .collect();
    for p in &packed {
        v.extend((p.len() as u16).to_le_bytes());
    }
    for p in &packed {
        v.extend(p);
    }
    v.extend((previews.len() as i32).to_le_bytes());
    for p in previews {
        let d = deflate(p);
        v.push(0);
        v.extend((d.len() as u32).to_le_bytes());
        v.extend(d);
    }
    v.extend((projects.len() as u16).to_le_bytes());
    for p in projects {
        v.extend((p.len() as u16).to_le_bytes());
        v.extend(p.as_bytes());
    }
    let mut data = Vec::new();
    for (pi, _) in projects.iter().enumerate() {
        let mut mine: Vec<&File> = files.iter().filter(|f| f.project == pi).collect();
        v.extend((mine.len() as i32).to_le_bytes());
        for f in &mine {
            let mut name = f.path.as_bytes().to_vec();
            if version > 6 {
                mask(&mut name, 17, 42);
            }
            v.extend((name.len() as u16).to_le_bytes());
            v.extend(name);
        }
        mine.sort_by_key(|f| deploy_hash(f.path.as_bytes()));
        for f in mine {
            let stored = match f.method {
                Method::Store => f.data.to_vec(),
                Method::Deflate => deflate(f.data),
                Method::Zlib => zlib(f.data),
                _ => unreachable!(),
            };
            let (size, packed) = (f.data.len() as u32, stored.len() as u32);
            // Big-endian on disk; the mask works on the little-endian word.
            let mut adler = adler32(f.data).swap_bytes();
            if version > 6 {
                adler ^= size ^ packed ^ 0x2A;
            }
            v.extend([0xAAu8; 32]);
            v.extend([0xBBu8; 16]);
            v.extend(adler.to_le_bytes());
            v.extend(size.to_le_bytes());
            v.extend(packed.to_le_bytes());
            v.push(f.method.0);
            data.extend(stored);
        }
    }
    v.extend(data);
    v
}

/// An old package: version 1 to 5. Strings are zlib streams without the first header
/// byte; data is a zlib stream without its first `cut` bytes, its packed size counting
/// them anyway.
fn build_old(version: i32, projects: &[&str], files: &[File], previews: &[&[u8]]) -> Vec<u8> {
    let cut = if version <= 4 { 1 } else { 2 };
    let counted = |v: &mut Vec<u8>, s: &str| {
        let z = zlib(s.as_bytes());
        v.extend((z.len() as i32 - 1).to_le_bytes());
        v.extend(&z[1..]);
    };
    let mut v = b"WXYS".to_vec();
    v.extend(version.to_le_bytes());
    counted(&mut v, "Magic Treeline");
    counted(&mut v, "Chewy");
    counted(&mut v, "v2");
    if version >= 3 {
        counted(&mut v, "Maps");
        counted(&mut v, "");
        v.extend((previews.len() as i32).to_le_bytes());
        for p in previews {
            // One byte before the stream and four after, which the reader skips.
            let d = deflate(p);
            v.extend((d.len() as u32 + 5).to_le_bytes());
            v.push(1);
            v.push(0);
            v.extend(d);
            v.extend([0; 4]);
        }
    } else {
        match previews {
            [] => v.extend(0u32.to_le_bytes()),
            [p, ..] => {
                let d = deflate(p);
                v.extend((d.len() as u32 + 5).to_le_bytes());
                v.push(0);
                v.extend(d);
                v.extend([0; 4]);
            }
        }
    }
    v.extend((projects.len() as i32).to_le_bytes());
    for p in projects {
        counted(&mut v, p);
    }
    if version >= 4 {
        v.extend(1i32.to_le_bytes());
        v.extend(0i32.to_le_bytes());
        counted(&mut v, "DATA/gone.bin");
    }
    v.extend((files.len() as i32).to_le_bytes());
    let mut data = Vec::new();
    for f in files {
        v.extend((f.project as i32).to_le_bytes());
        counted(&mut v, f.path);
        let stored = match f.method {
            Method::Store => f.data.to_vec(),
            Method::Zlib => zlib(f.data),
            _ => unreachable!(),
        };
        v.extend((f.data.len() as u32).to_le_bytes());
        v.extend((stored.len() as u32).to_le_bytes());
        if version != 1 {
            v.extend([0xBBu8; 16]);
        }
        match f.method {
            Method::Store => data.extend(stored),
            _ => data.extend(&stored[cut..]),
        }
    }
    v.extend(data);
    v
}

fn open(bytes: Vec<u8>) -> (WxyArchive, OpenReport) {
    let cb = OpenCallback::none();
    let a = Wxy::open(InStream::new(Cursor::new(bytes)), &cb, &Options::default()).unwrap();
    (a, cb.report())
}

fn try_open(bytes: Vec<u8>) -> (Result<WxyArchive, Error>, OpenReport) {
    let cb = OpenCallback::none();
    let r = Wxy::open(InStream::new(Cursor::new(bytes)), &cb, &Options::default());
    (r, cb.report())
}

/// What the default `Archive::extract` would report for the item, and its bytes.
fn extract(a: &WxyArchive, index: u32) -> (OperationResult, Vec<u8>) {
    let mut out = Vec::new();
    let r = match a.extract_item(index, &mut out) {
        Ok(r) => r,
        Err(e) => e.operation_result().unwrap(),
    };
    (r, out)
}

fn text(a: &WxyArchive, index: u32, id: PropId) -> String {
    a.item_prop(index, id).unwrap().into_string().unwrap()
}

fn paths(a: &WxyArchive) -> Vec<String> {
    (0..a.item_count())
        .map(|i| text(a, i, PropId::Path))
        .collect()
}

fn at(a: &WxyArchive, name: &str) -> u32 {
    paths(a)
        .iter()
        .position(|p| p == name)
        .unwrap_or_else(|| panic!("{name} not in {:?}", paths(a))) as u32
}

fn clean(report: &OpenReport) {
    assert_eq!(
        report.errors | report.warnings,
        ErrorFlags::empty(),
        "{report:?}"
    );
}

fn comment(a: &WxyArchive) -> String {
    a.archive_prop(PropId::Comment)
        .unwrap()
        .into_string()
        .unwrap()
}

fn files() -> Vec<File> {
    vec![
        file(
            0,
            "DATA\\Characters\\Bard\\Skins\\Base\\Bard.bin",
            TEXT,
            Method::Zlib,
        ),
        file(
            0,
            "ASSETS/Characters/Bard/Skins/Base/Bard.dds",
            TEXT,
            Method::Deflate,
        ),
        file(
            1,
            "DATA/Characters/Bard/Animations/Bard.anm",
            b"raw",
            Method::Store,
        ),
        file(
            1,
            "DATA/Characters/Bard/Skins/Base/Bard.skn",
            b"",
            Method::Deflate,
        ),
    ]
}

#[test]
fn reads_oink() {
    for version in [6, 7] {
        let files = files();
        let (a, report) = open(build_oink(
            version,
            &["Bard", "Bard2"],
            &files,
            &[PNG, b"BM!"],
        ));
        clean(&report);
        // Entries list per project in the order of their `deploy/` hashes.
        let mut expected: Vec<String> = Vec::new();
        for p in 0..2 {
            let mut mine: Vec<&File> = files.iter().filter(|f| f.project == p).collect();
            mine.sort_by_key(|f| deploy_hash(f.path.as_bytes()));
            expected.extend(
                mine.iter()
                    .map(|f| format!("RAW/{}", f.path.replace('\\', "/"))),
            );
        }
        expected.extend(["META/info.json", "META/image.png", "META/image0.bmp"].map(String::from));
        assert_eq!(paths(&a), expected, "version {version}");
        for f in &files {
            let name = format!("RAW/{}", f.path.replace('\\', "/"));
            let i = at(&a, &name);
            assert_eq!(
                extract(&a, i),
                (OperationResult::OK, f.data.to_vec()),
                "{name}"
            );
            assert_eq!(text(&a, i, PropId::Method), f.method.name());
            assert_eq!(
                a.item_prop(i, PropId::Size).unwrap().as_u64(),
                Some(f.data.len() as u64)
            );
            assert_eq!(
                a.item_stream(i).unwrap().is_some(),
                f.method == Method::Store
            );
        }
        let info = extract(&a, at(&a, "META/info.json")).1;
        let j: serde_json::Value = serde_json::from_slice(&info).unwrap();
        assert_eq!(j["Name"], "Bard Malfurious");
        assert_eq!(j["Author"], "someone");
        assert_eq!(j["Version"], "1.0");
        assert_eq!(extract(&a, at(&a, "META/image.png")).1, PNG);
        assert_eq!(extract(&a, at(&a, "META/image0.bmp")).1, b"BM!");
        assert_eq!(
            comment(&a),
            "Bard Malfurious 1.0 by someone; Champions/Bard; projects: Bard, Bard2"
        );
        assert_eq!(
            a.archive_prop(PropId::UnpackVer).unwrap().as_str(),
            Some(version.to_string().as_str())
        );
        assert!(a.toc().version >= OINK_VERSION);
    }
}

#[test]
fn reads_old() {
    let files = vec![
        file(
            0,
            "DATA/Maps/MapGeometry/Map1/Map1.wgeo",
            TEXT,
            Method::Zlib,
        ),
        file(1, "LEVELS/Map1/Scene/room.dsc", b"stored", Method::Store),
    ];
    for version in [1, 2, 3, 4, 5] {
        let bytes = build_old(version, &["Map1", "Map1b"], &files, &[PNG]);
        let (a, report) = open(bytes);
        clean(&report);
        assert_eq!(
            paths(&a),
            [
                "RAW/DATA/Maps/MapGeometry/Map1/Map1.wgeo",
                "RAW/LEVELS/Map1/Scene/room.dsc",
                "META/info.json",
                "META/image.png",
            ],
            "version {version}"
        );
        assert_eq!(
            extract(&a, 0),
            (OperationResult::OK, TEXT.to_vec()),
            "version {version}"
        );
        assert_eq!(extract(&a, 1), (OperationResult::OK, b"stored".to_vec()));
        assert_eq!(extract(&a, 3).1, PNG, "version {version}");
        assert_eq!(text(&a, 0, PropId::Method), "zlib");
        let c = comment(&a);
        assert!(c.starts_with("Magic Treeline v2 by Chewy"), "{c}");
        if version >= 4 {
            assert!(c.ends_with("deletes: 1"), "{c}");
            assert_eq!(
                a.toc().deletes,
                [("Map1".to_string(), "DATA/gone.bin".to_string())]
            );
        }
        if version >= 3 {
            assert!(c.contains("; Maps;"), "{c}");
        }
    }
}

#[test]
fn checks_what_comes_out() {
    let mut bytes = build_oink(
        7,
        &["p"],
        &[file(0, "DATA/a.bin", TEXT, Method::Store)],
        &[],
    );
    let n = bytes.len();
    // Flip a byte of the stored data: the Adler-32 catches it.
    bytes[n - 1] ^= 1;
    let (a, _) = open(bytes);
    assert_eq!(extract(&a, 0).0, OperationResult::CRCError);
}

#[test]
fn not_a_wxy() {
    for bytes in [
        b"".to_vec(),
        b"WX".to_vec(),
        b"RW\x03\0".to_vec(),
        b"WXYS".to_vec(),
    ] {
        let (r, _) = try_open(bytes.clone());
        let expected = if bytes == b"WXYS" {
            Error::UNEXPECTED_END
        } else {
            Error::NotArchive
        };
        assert_eq!(r.err(), Some(expected), "{bytes:?}");
    }
    let mut bytes = build_oink(7, &["p"], &[], &[]);
    bytes[8..12].copy_from_slice(b"MOO!");
    assert_eq!(try_open(bytes).0.err(), Some(Error::HEADERS_ERROR));
}

#[test]
fn truncated() {
    let whole = build_oink(
        7,
        &["p"],
        &[file(0, "DATA/a.bin", TEXT, Method::Deflate)],
        &[],
    );
    // Inside the tables: nothing to list.
    let (r, _) = try_open(whole[..whole.len() - 60].to_vec());
    assert_eq!(r.err(), Some(Error::UNEXPECTED_END));
    // Inside the data: listed, flagged, and extraction says so.
    let (a, report) = open(whole[..whole.len() - 3].to_vec());
    assert_eq!(report.errors, ErrorFlags::UnexpectedEnd);
    assert_eq!(a.item_count(), 2);
    assert_ne!(extract(&a, 0).0, OperationResult::OK);
}

#[test]
fn hostile_counts() {
    let mut bytes = build_oink(7, &["p"], &[], &[]);
    // Preview count far past the file.
    let at = bytes.len() - 2 - 1 - 4;
    bytes[at..at + 4].copy_from_slice(&i32::MAX.to_le_bytes());
    assert_eq!(try_open(bytes).0.err(), Some(Error::UNEXPECTED_END));
    let mut bytes = build_old(5, &["p"], &[], &[]);
    let at = bytes.len() - 4;
    bytes[at..].copy_from_slice(&i32::MAX.to_le_bytes());
    assert_eq!(try_open(bytes).0.err(), Some(Error::UNEXPECTED_END));
}

// ---------------------------------------------------------------- samples

fn samples() -> Vec<PathBuf> {
    let dir = std::env::var_os("WXY_SAMPLES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"X:\Archive\skins\wxy-samples"));
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "wxy"))
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
        println!("{sample:?}: version {}, {}", a.toc().version, comment(&a));
        for n in names.iter().take(8) {
            println!("  {n}");
        }
        assert!(names.contains(&"META/info.json".to_string()));
        assert!(names.iter().any(|n| n.starts_with("RAW/")), "{names:?}");
        assert!(!a.toc().meta.name.is_empty());
        for i in 0..a.item_count() {
            let (r, data) = extract(&a, i);
            assert_eq!(r, OperationResult::OK, "{sample:?} {}", names[i as usize]);
            if let Item::Preview(_) = a.items()[i as usize] {
                assert!(!data.is_empty(), "{}", names[i as usize]);
            }
        }
    }
}
