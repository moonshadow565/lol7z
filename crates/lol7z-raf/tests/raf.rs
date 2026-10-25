//! Archives built by hand the way `RAF.cs` writes them, and the sample archive, read
//! through the `Format`/`Archive` traits with a volume callback standing in for 7-Zip.

use flate2::Compression;
use flate2::write::ZlibEncoder;
use lol7z_api::{
    Archive, Error, ErrorFlags, Format, InStream, OpenCallback, OpenReport, OperationResult,
    Options, PropId,
};
use lol7z_raf::toc::{self, MAGIC, path_hash};
use lol7z_raf::{Method, Raf, RafArchive, is_zlib};
use std::io::{Cursor, Write};
use std::path::PathBuf;

const NAME: &str = "Archive_1.raf";
const DATA_NAME: &str = "Archive_1.raf.dat";
const TEXT: &[u8] = b"the quick brown fox jumps over the lazy dog, again and again and again";
const BLOB: &[u8] = &[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0, 0, 0, 1];

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

struct File {
    path: &'static str,
    data: &'static [u8],
    compress: bool,
}

fn file(path: &'static str, data: &'static [u8], compress: bool) -> File {
    File {
        path,
        data,
        compress,
    }
}

/// `(index, data)` as the reference writer lays them out: entries sorted by hash, each
/// path listed once in the same order.
fn build(files: &[File]) -> (Vec<u8>, Vec<u8>) {
    let mut data = Vec::new();
    let mut records: Vec<(u32, u32, u32, &str)> = files
        .iter()
        .map(|f| {
            let offset = data.len() as u32;
            let stored = if f.compress {
                zlib(f.data)
            } else {
                f.data.to_vec()
            };
            data.extend(&stored);
            (path_hash(f.path), offset, stored.len() as u32, f.path)
        })
        .collect();
    records.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.3.cmp(b.3)));
    (build_index(&records), data)
}

/// `records`: `(hash, offset, length, path)`.
fn build_index(records: &[(u32, u32, u32, &str)]) -> Vec<u8> {
    let mut v = MAGIC.to_vec();
    v.extend(1i32.to_le_bytes());
    v.extend(0i32.to_le_bytes());
    v.extend(20u32.to_le_bytes());
    let paths_at = 24 + records.len() * 16;
    v.extend((paths_at as u32).to_le_bytes());
    v.extend((records.len() as u32).to_le_bytes());
    for (i, &(hash, offset, length, _)) in records.iter().enumerate() {
        v.extend(hash.to_le_bytes());
        v.extend(offset.to_le_bytes());
        v.extend(length.to_le_bytes());
        v.extend((i as u32).to_le_bytes());
    }
    let strings: usize = records.iter().map(|r| r.3.len() + 1).sum();
    let table = 8 + records.len() * 8;
    v.extend(((table + strings) as u32).to_le_bytes());
    v.extend((records.len() as u32).to_le_bytes());
    let mut at = table;
    for r in records {
        v.extend((at as u32).to_le_bytes());
        v.extend(((r.3.len() + 1) as u32).to_le_bytes());
        at += r.3.len() + 1;
    }
    for r in records {
        v.extend(r.3.as_bytes());
        v.push(0);
    }
    v
}

fn callback(data: Option<Vec<u8>>) -> OpenCallback<'static> {
    OpenCallback::with_volumes(NAME, move |name| {
        let d = data.as_ref()?;
        (name == DATA_NAME).then(|| InStream::new(Cursor::new(d.clone())))
    })
}

fn try_open(index: Vec<u8>, data: Option<Vec<u8>>) -> (Result<RafArchive, Error>, OpenReport) {
    let cb = callback(data);
    let r = Raf::open(InStream::new(Cursor::new(index)), &cb, &Options::default());
    (r, cb.report())
}

fn open(index: Vec<u8>, data: Option<Vec<u8>>) -> (RafArchive, OpenReport) {
    let (r, report) = try_open(index, data);
    (r.unwrap(), report)
}

/// What the default `Archive::extract` would report for the item, and its bytes.
fn extract(a: &RafArchive, index: u32) -> (OperationResult, Vec<u8>) {
    let mut out = Vec::new();
    let r = match a.extract_item(index, &mut out) {
        Ok(r) => r,
        Err(e) => e.operation_result().unwrap(),
    };
    (r, out)
}

fn text(a: &RafArchive, index: u32, id: PropId) -> String {
    a.item_prop(index, id).unwrap().into_string().unwrap()
}

fn num(a: &RafArchive, index: u32, id: PropId) -> Option<u64> {
    a.item_prop(index, id).and_then(|p| p.as_u64())
}

fn paths(a: &RafArchive) -> Vec<String> {
    (0..a.item_count())
        .map(|i| text(a, i, PropId::Path))
        .collect()
}

fn at(a: &RafArchive, name: &str) -> u32 {
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

fn sample() -> Vec<File> {
    vec![
        file("DATA/Characters/Annie/Annie.skn", TEXT, true),
        file("DATA/Characters/Annie/Annie.dds", BLOB, false),
        file("LEVELS/Map1/Scene/room.nvr", TEXT, false),
        file("DATA/empty.txt", b"", false),
    ]
}

// ---------------------------------------------------------------- hash

#[test]
fn hash_is_case_insensitive_and_known() {
    assert_eq!(path_hash(""), 0);
    assert_eq!(
        path_hash("DATA/Characters/Annie/Annie.skn"),
        path_hash("data/characters/annie/annie.skn")
    );
    assert_ne!(path_hash("a"), path_hash("b"));
    // `a` is 0x61; the shift-xor over four or fewer characters is just the shifted sum.
    assert_eq!(path_hash("a"), 0x61);
    assert_eq!(path_hash("ab"), 0x61 << 4 | 0x62);
}

#[test]
fn sample_sorts_by_hash_like_the_writer() {
    let (index, _) = build(&sample());
    let toc = toc::read(&index).unwrap();
    let hashes: Vec<u32> = toc.entries.iter().map(|e| e.hash).collect();
    let mut sorted = hashes.clone();
    sorted.sort_unstable();
    assert_eq!(hashes, sorted);
    for e in &toc.entries {
        assert_eq!(e.hash, path_hash(&e.path), "{}", e.path);
    }
}

// ---------------------------------------------------------------- the sniff

/// The file types RADS release manifests ever deployed stored (mode 6) rather than
/// compressed (mode 22), by their magic. A stored one must never read as zlib.
#[test]
fn known_stored_magics_never_look_like_zlib() {
    let magics: &[&[u8]] = &[
        b"RW\x03\x00", // .wad.client
        b"RW\x01\x00",
        b"FSB4", // FMOD sample bank
        b"FSB5",
        b"FEV1",    // FMOD event file
        b"BKHD",    // Wwise bank
        b"CFX\x08", // Scaleform gfx, compressed
        b"GFX\x08", // Scaleform gfx, plain
        b"CWS\x0a", // SWF, in case
        b"FWS\x0a",
        b"<?xm",          // .xml
        b"\xEF\xBB\xBF<", // .xml with a BOM
    ];
    for m in magics {
        assert!(!is_zlib([m[0], m[1]]), "{:?}", String::from_utf8_lossy(m));
    }
    // What a zlib stream does start with, at every level.
    for head in [
        [0x78, 0x01],
        [0x78, 0x5E],
        [0x78, 0x9C],
        [0x78, 0xDA],
        [0x68, 0x81],
    ] {
        assert!(is_zlib(head), "{head:02x?}");
    }
    assert!(!is_zlib([0x78, 0x9D]), "check bits");
    assert!(!is_zlib([0x78, 0xBC]), "preset dictionary");
    assert!(!is_zlib([0x88, 0x98]), "window too large");
    assert!(!is_zlib([0x79, 0x9C]), "not deflate");
}

// ---------------------------------------------------------------- listing and data

#[test]
fn lists_and_extracts() {
    let (index, data) = build(&sample());
    let (a, report) = open(index, Some(data));
    clean(&report);
    assert_eq!(a.item_count(), 4);
    let mut names = paths(&a);
    names.sort();
    assert_eq!(
        names,
        [
            "DATA/Characters/Annie/Annie.dds",
            "DATA/Characters/Annie/Annie.skn",
            "DATA/empty.txt",
            "LEVELS/Map1/Scene/room.nvr",
        ]
    );

    let skn = at(&a, "DATA/Characters/Annie/Annie.skn");
    assert_eq!(a.methods()[skn as usize], Method::Zlib);
    assert_eq!(text(&a, skn, PropId::Method), "zlib");
    assert_eq!(num(&a, skn, PropId::Size), None, "zlib stores no size");
    assert_eq!(
        num(&a, skn, PropId::PackSize),
        Some(zlib(TEXT).len() as u64)
    );
    assert!(a.item_stream(skn).unwrap().is_none());
    assert_eq!(extract(&a, skn), (OperationResult::OK, TEXT.to_vec()));

    let dds = at(&a, "DATA/Characters/Annie/Annie.dds");
    assert_eq!(text(&a, dds, PropId::Method), "Store");
    assert_eq!(num(&a, dds, PropId::Size), Some(BLOB.len() as u64));
    assert_eq!(num(&a, dds, PropId::PackSize), Some(BLOB.len() as u64));
    assert!(a.item_stream(dds).unwrap().is_some());
    assert_eq!(extract(&a, dds), (OperationResult::OK, BLOB.to_vec()));

    let nvr = at(&a, "LEVELS/Map1/Scene/room.nvr");
    assert_eq!(extract(&a, nvr), (OperationResult::OK, TEXT.to_vec()));

    let empty = at(&a, "DATA/empty.txt");
    assert_eq!(text(&a, empty, PropId::Method), "Store");
    assert_eq!(num(&a, empty, PropId::Size), Some(0));
    assert_eq!(extract(&a, empty), (OperationResult::OK, Vec::new()));

    for i in 0..a.item_count() {
        assert_eq!(
            a.item_prop(i, PropId::IsDir).unwrap().as_bool(),
            Some(false)
        );
        let e = &a.toc().entries[i as usize];
        assert_eq!(num(&a, i, PropId::Offset), Some(u64::from(e.offset)));
    }
}

#[test]
fn archive_props() {
    let (index, data) = build(&sample());
    let (index_len, data_len) = (index.len() as u64, data.len() as u64);
    let (a, _) = open(index, Some(data));
    let prop = |id| a.archive_prop(id).unwrap();
    assert_eq!(prop(PropId::PhySize).as_u64(), Some(index_len));
    assert_eq!(
        prop(PropId::TotalPhySize).as_u64(),
        Some(index_len + data_len)
    );
    assert_eq!(prop(PropId::NumVolumes).as_u64(), Some(2));
    assert_eq!(prop(PropId::UnpackVer).into_string().unwrap(), "1");
    assert_eq!(
        prop(PropId::Comment).into_string().unwrap(),
        format!("data in {DATA_NAME}")
    );
    assert_eq!(a.data_name(), DATA_NAME);
}

#[test]
fn stored_data_shorter_than_zlib_overhead_is_stored() {
    // 5 bytes can't be a zlib stream, even starting with a valid header.
    let (index, data) = build(&[file("DATA/x.bin", &[0x78, 0x9C, 1, 2, 3], false)]);
    let (a, report) = open(index, Some(data));
    clean(&report);
    assert_eq!(text(&a, 0, PropId::Method), "Store");
    assert_eq!(
        extract(&a, 0),
        (OperationResult::OK, vec![0x78, 0x9C, 1, 2, 3])
    );
}

#[test]
fn bad_adler_is_a_crc_error() {
    let (index, mut data) = build(&[file("DATA/x.bin", TEXT, true)]);
    let n = data.len();
    data[n - 1] ^= 0xFF;
    let (a, report) = open(index, Some(data));
    clean(&report);
    let (r, out) = extract(&a, 0);
    assert_eq!(r, OperationResult::CRCError);
    assert_eq!(out, TEXT);
}

#[test]
fn corrupt_deflate_is_a_data_error() {
    let (index, mut data) = build(&[file("DATA/x.bin", TEXT, true)]);
    for b in &mut data[2..8] {
        *b = 0xFF;
    }
    let (a, report) = open(index, Some(data));
    clean(&report);
    assert_eq!(extract(&a, 0).0, OperationResult::DataError);
}

#[test]
fn cut_data_file() {
    let (index, mut data) = build(&sample());
    data.truncate(data.len() - 4);
    let (a, report) = open(index, Some(data));
    assert_eq!(report.errors, ErrorFlags::UnexpectedEnd);
    assert!(report.error.unwrap().contains(DATA_NAME));
    // The entry that reaches furthest is the cut one; the empty entry past the end still extracts.
    let last = a
        .toc()
        .entries
        .iter()
        .enumerate()
        .max_by_key(|(_, e)| (e.offset + e.length, e.length))
        .map(|(i, _)| i as u32)
        .unwrap();
    assert_eq!(extract(&a, last).0, OperationResult::UnexpectedEnd);
    let others: Vec<u32> = (0..a.item_count()).filter(|&i| i != last).collect();
    for i in others {
        assert_eq!(
            extract(&a, i).0,
            OperationResult::OK,
            "{}",
            text(&a, i, PropId::Path)
        );
    }
}

#[test]
fn missing_data_file_lists_but_cannot_extract() {
    let (index, _) = build(&sample());
    let (a, report) = open(index, None);
    assert_eq!(report.errors, ErrorFlags::UnexpectedEnd);
    assert_eq!(
        report.error.as_deref(),
        Some("can't open data file Archive_1.raf.dat")
    );
    assert_eq!(a.item_count(), 4);
    assert!(a.methods().iter().all(|&m| m == Method::Unknown));
    assert_eq!(a.item_prop(0, PropId::Method), None);
    assert_eq!(num(&a, 0, PropId::Size), None);
    assert!(num(&a, 0, PropId::PackSize).is_some());
    assert_eq!(extract(&a, 0).0, OperationResult::Unavailable);
    assert!(a.item_stream(0).unwrap().is_none());
    assert_eq!(
        a.archive_prop(PropId::NumVolumes).unwrap().as_u64(),
        Some(1)
    );
}

#[test]
fn opened_without_a_file_name() {
    let (index, _) = build(&sample());
    let cb = OpenCallback::none();
    let a = Raf::open(InStream::new(Cursor::new(index)), &cb, &Options::default()).unwrap();
    assert_eq!(cb.report().errors, ErrorFlags::UnexpectedEnd);
    assert_eq!(a.item_count(), 4);
    assert_eq!(extract(&a, 0).0, OperationResult::Unavailable);
}

// ---------------------------------------------------------------- bad indexes

#[test]
fn not_a_raf() {
    let (r, report) = try_open(b"RIFF".to_vec(), None);
    assert_eq!(r.err().unwrap(), Error::NotArchive);
    clean(&report);
    let (r, _) = try_open(b"\xF0\x0E".to_vec(), None);
    assert_eq!(r.err().unwrap(), Error::NotArchive);
}

#[test]
fn cut_index() {
    let (index, data) = build(&sample());
    for cut in [8, 20, 40, index.len() - 30, index.len() - 1] {
        let (r, _) = try_open(index[..cut].to_vec(), Some(data.clone()));
        assert_eq!(r.err().unwrap(), Error::UNEXPECTED_END, "cut at {cut}");
    }
}

#[test]
fn bad_path_index() {
    let (mut index, data) = build(&sample());
    // The first entry's path list index, at header + count.
    index[24 + 12..24 + 16].copy_from_slice(&99u32.to_le_bytes());
    let (r, _) = try_open(index, Some(data));
    assert_eq!(r.err().unwrap(), Error::HEADERS_ERROR);
}

#[test]
fn zero_length_path() {
    let (mut index, data) = build(&sample());
    let paths_at = 24 + 4 * 16;
    index[paths_at + 8 + 4..paths_at + 8 + 8].copy_from_slice(&0u32.to_le_bytes());
    let (r, _) = try_open(index, Some(data));
    assert_eq!(r.err().unwrap(), Error::HEADERS_ERROR);
}

#[test]
fn wrong_hash_is_a_warning() {
    let records = [(0xDEAD_BEEFu32, 0u32, BLOB.len() as u32, "DATA/x.bin")];
    let (a, report) = open(build_index(&records), Some(BLOB.to_vec()));
    assert_eq!(report.errors, ErrorFlags::empty());
    assert_eq!(report.warnings, ErrorFlags::HeadersError);
    assert!(report.warning.is_some());
    assert_eq!(extract(&a, 0), (OperationResult::OK, BLOB.to_vec()));
}

#[test]
fn leading_slash_and_backslash_are_shown_as_a_path() {
    let records = [(path_hash("/DATA\\a.bin"), 0u32, 4u32, "/DATA\\a.bin")];
    let (a, _) = open(build_index(&records), Some(BLOB.to_vec()));
    assert_eq!(paths(&a), ["DATA/a.bin"]);
    assert_eq!(a.toc().entries[0].path, "/DATA\\a.bin");
}

// ---------------------------------------------------------------- the sample archive

fn sample_archive() -> Option<PathBuf> {
    let path = std::env::var_os("RAF_SAMPLE").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(
            r"C:\Users\user\Desktop\League-of-Legends-4-20\RADS\projects\lol_game_client\filearchives\0.0.0.25\Archive_1.raf",
        )
    });
    path.is_file().then_some(path)
}

#[test]
fn reads_sample_archive() {
    let Some(path) = sample_archive() else {
        eprintln!("no sample archive; set RAF_SAMPLE");
        return;
    };
    let index = std::fs::read(&path).unwrap();
    let mut data_path = path.clone().into_os_string();
    data_path.push(".dat");
    let data = std::fs::File::open(&data_path).unwrap();
    let data_len = data.metadata().unwrap().len();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let data_name = format!("{name}.dat");
    let data = std::cell::RefCell::new(Some(data));
    let cb = OpenCallback::with_volumes(&name, move |n| {
        (n == data_name).then(|| InStream::new(data.borrow_mut().take().unwrap()))
    });
    let a = Raf::open(InStream::new(Cursor::new(index)), &cb, &Options::default()).unwrap();
    clean(&cb.report());
    assert_eq!(
        a.archive_prop(PropId::TotalPhySize).unwrap().as_u64(),
        Some(a.archive_prop(PropId::PhySize).unwrap().as_u64().unwrap() + data_len)
    );

    let names = paths(&a);
    println!("{path:?}: {} entries", names.len());
    for n in names.iter().take(5) {
        println!("  {n}");
    }
    assert!(!names.is_empty());
    assert!(
        names
            .iter()
            .all(|n| n.starts_with("DATA/") || n.starts_with("LEVELS/")),
        "{names:?}"
    );
    let zlib = a.methods().iter().filter(|&&m| m == Method::Zlib).count();
    let stored = a.methods().len() - zlib;
    println!("  {zlib} zlib, {stored} stored");
    assert!(zlib > 0 && stored > 0);
    for i in 0..a.item_count() {
        let (r, out) = extract(&a, i);
        assert_eq!(r, OperationResult::OK, "{}", names[i as usize]);
        if a.methods()[i as usize] == Method::Store {
            assert_eq!(out.len() as u64, num(&a, i, PropId::Size).unwrap());
        }
    }
}

// ---------------------------------------------------------------- hostile indexes

#[test]
fn entries_sharing_one_long_path_cannot_outgrow_the_index() {
    // 4096 entries all naming one 64 KiB path: more string than the index holds.
    let long = "a".repeat(1 << 16);
    let mut v = MAGIC.to_vec();
    v.extend(1i32.to_le_bytes());
    v.extend(0i32.to_le_bytes());
    v.extend(20u32.to_le_bytes());
    let n = 4096u32;
    v.extend((24 + n * 16).to_le_bytes());
    v.extend(n.to_le_bytes());
    for _ in 0..n {
        v.extend([0u32, 0, 4, 0].iter().flat_map(|x| x.to_le_bytes()));
    }
    v.extend(0u32.to_le_bytes());
    v.extend(1u32.to_le_bytes());
    v.extend(16u32.to_le_bytes());
    v.extend((long.len() as u32 + 1).to_le_bytes());
    v.extend(long.as_bytes());
    v.push(0);
    let (r, _) = try_open(v, Some(BLOB.to_vec()));
    assert_eq!(r.err().unwrap(), Error::HEADERS_ERROR);
}

#[test]
fn non_utf8_path_lists_with_a_hash_warning() {
    let records = [(0u32, 0u32, BLOB.len() as u32, "DATA/caf\u{E9}.bin")];
    let mut index = build_index(&records);
    // Latin-1 in place of the two UTF-8 bytes: one byte shorter, so patch the length too.
    let at = index
        .windows(3)
        .position(|w| w == "\u{E9}.".as_bytes()[..3].to_vec())
        .unwrap();
    index.splice(at..at + 2, [0xE9]);
    let paths_at = 24 + 16;
    let len = u32::from_le_bytes(index[paths_at + 12..paths_at + 16].try_into().unwrap());
    index[paths_at + 12..paths_at + 16].copy_from_slice(&(len - 1).to_le_bytes());
    let (a, report) = open(index, Some(BLOB.to_vec()));
    assert_eq!(report.warnings, ErrorFlags::HeadersError);
    assert_eq!(paths(&a), ["DATA/caf\u{FFFD}.bin"]);
    assert_eq!(extract(&a, 0), (OperationResult::OK, BLOB.to_vec()));
}

#[test]
fn padding_after_the_zlib_stream_is_ignored() {
    let mut stored = zlib(TEXT);
    stored.extend([0u8; 7]);
    let records = [(
        path_hash("DATA/x.bin"),
        0u32,
        stored.len() as u32,
        "DATA/x.bin",
    )];
    let (a, report) = open(build_index(&records), Some(stored));
    clean(&report);
    assert_eq!(text(&a, 0, PropId::Method), "zlib");
    assert_eq!(extract(&a, 0), (OperationResult::OK, TEXT.to_vec()));
}
