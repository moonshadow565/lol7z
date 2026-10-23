//! Opens each WAD given and decodes every entry, reporting what failed:
//! `cargo run --release -p lol7z_wad --example check -- [--hashes] <wad>...`
//!
//! `--hashes` prints each item's hash and name instead.

use lol7z_api::{Archive, Format, InStream, OpenCallback, OperationResult, Options};
use lol7z_wad::Wad;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader};
use std::time::Instant;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let hashes = args.first().is_some_and(|a| a == "--hashes");
    if hashes {
        args.remove(0);
    }
    for path in args {
        let t = Instant::now();
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                println!("{path}: {e}");
                continue;
            }
        };
        let cb = OpenCallback::none();
        let a = match Wad::open(
            InStream::new(BufReader::new(file)),
            &cb,
            &Options::default(),
        ) {
            Ok(a) => a,
            Err(e) => {
                println!("{path}: open failed: {e}");
                continue;
            }
        };
        let opened = t.elapsed();
        if hashes {
            for (i, e) in a.entries().enumerate() {
                println!("{:016x} {}", e.hash, a.names().string(i as u32));
            }
            eprintln!("{path}: open {opened:.1?}; {}", a.names().comment());
            continue;
        }
        let r = cb.report();
        let h = a.header();
        let s = a.names().stats();
        let mut kinds: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
        let mut bytes = 0u64;
        for (i, e) in a.entries().enumerate() {
            let mut out = io::sink();
            let res = a.extract_item(i as u32, &mut out);
            let k = kinds.entry(e.kind.name()).or_default();
            k.0 += 1;
            bytes += u64::from(e.size_decompressed);
            match res {
                Ok(OperationResult::OK) => {}
                Ok(other) => {
                    k.1 += 1;
                    println!("  {:016x} {}: {other:?}", e.hash, e.kind.name());
                }
                Err(err) => {
                    k.1 += 1;
                    println!("  {:016x} {}: {err}", e.hash, e.kind.name());
                }
            }
        }
        println!(
            "{path}: v{}.{} {} items, {} MiB out, errors {:?} warnings {:?} {:?}, {:.1?}",
            h.major,
            h.minor,
            a.item_count(),
            bytes >> 20,
            r.errors,
            r.warnings,
            r.warning,
            t.elapsed()
        );
        println!(
            "  open {opened:.1?}; {}; named {} (folded {} hashed {} clash {}), «H»: unknown {} \
             unverified {}",
            a.names().comment(),
            s.named,
            s.folded,
            s.hashed,
            s.clash,
            s.unknown,
            s.unverified,
        );
        for (name, (n, bad)) in kinds {
            println!("  {name}: {n} ({bad} failed)");
        }
    }
}
