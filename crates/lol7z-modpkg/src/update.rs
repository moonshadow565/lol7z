//! Writing a package from 7-Zip's update callback: the project layout read back.
//!
//! A pack needs `mod.config.json`, from the host or kept from the package being
//! updated: it names the mod and declares the layers. The document is written from
//! it, so a kept package's unknown keys survive through the config's `extra`.

use crate::archive::{Item, ModpkgArchive};
use crate::config::{self, Config, MAX_CONFIG};
use crate::format::Modpkg;
use crate::meta::MAX_INFO;
use crate::path::{self, Chunk, HASHES_PREFIX, INFO, Read};
use crate::toc::{Compression, Layer, NONE, Record, Tables};
use crate::writer::{At, Writer};
use indexmap::map::Entry;
use indexmap::{IndexMap, IndexSet};
use lol7z_api::{Error, Options, OutStream, Result, Update, UpdateCallback};
use lol7z_wad::encode;
use lol7z_wad::toc::Kind;
use lol7z_wad::{PathHash, zstd_level};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{self, Read as _, Write};
use xxhash_rust::xxh3::{Xxh3, xxh3_64};
use xxhash_rust::xxh64::xxh64;

/// Where a new package's chunk comes from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// Data from the host, by update index.
    New(u32),
    /// Copied as stored, by index in the existing package.
    Old(u32),
    /// The metadata document, written from the config.
    Info,
}

/// A chunk as planned, before the tables are known.
struct Pending {
    chunk: Chunk,
    source: Source,
    /// A copy's stored bytes: offset and size. `None` for new data.
    run: Option<(u64, u64)>,
    size: u64,
}

/// One chunk of the new package.
#[derive(Clone, Copy)]
struct Step {
    at: At,
    source: Source,
    run: Option<(u64, u64)>,
    size: u64,
}

/// The tables being built: the layer table as declared, each path and WAD once by
/// canonical name in the order first seen.
struct Intern {
    layers: IndexMap<String, Layer>,
    /// By ASCII-lowercased path, to the path as first spelled.
    paths: IndexMap<String, String>,
    /// Lowercased, as the reference writer stores them.
    wads: IndexSet<String>,
}

impl Intern {
    fn new(table: &[Layer]) -> Self {
        Self {
            layers: table.iter().map(|l| (l.name.clone(), l.clone())).collect(),
            paths: IndexMap::new(),
            wads: IndexSet::new(),
        }
    }

    /// A layer the config declares; `INVALID_ARG` for any other.
    fn layer(&self, name: &str) -> Result<u32> {
        index(self.layers.get_index_of(name).ok_or(Error::INVALID_ARG)?)
    }

    fn path(&mut self, p: &str) -> Result<u32> {
        if path::escapes(p) {
            return Err(Error::INVALID_ARG);
        }
        index(match self.paths.entry(p.to_ascii_lowercase()) {
            Entry::Occupied(e) => e.index(),
            Entry::Vacant(e) => {
                let i = e.index();
                e.insert(p.to_owned());
                i
            }
        })
    }

    fn wad(&mut self, w: &str) -> Result<u32> {
        if path::escapes(w) || w.contains('/') {
            return Err(Error::INVALID_ARG);
        }
        index(self.wads.insert_full(w.to_ascii_lowercase()).0)
    }

    fn at(&mut self, c: &Chunk) -> Result<At> {
        Ok(At {
            hash: c.hash,
            path_index: self.path(&c.path)?,
            layer_index: match &c.layer {
                Some(l) => self.layer(l)?,
                None => NONE,
            },
            wad_index: match &c.wad {
                Some(w) => self.wad(w)?,
                None => NONE,
            },
        })
    }

    fn into_tables(self) -> Tables {
        Tables {
            layers: self.layers.into_values().collect(),
            paths: self.paths.into_values().collect(),
            wads: self.wads.into_iter().collect(),
        }
    }
}

fn index(n: usize) -> Result<u32> {
    u32::try_from(n)
        .ok()
        .filter(|&i| i != NONE)
        .ok_or(Error::INVALID_ARG)
}

/// An existing record, under its own names.
fn chunk_of(t: &Tables, r: &Record) -> Chunk {
    let name = |table: &[String], i: u32| (i != NONE).then(|| table[i as usize].clone());
    Chunk {
        path: t.paths[r.path_index as usize].clone(),
        hash: r.path_hash,
        layer: (r.layer_index != NONE).then(|| t.layers[r.layer_index as usize].name.clone()),
        wad: name(&t.wads, r.wad_index),
    }
}

/// A new item's whole content, at most `max` bytes.
fn slurp(cb: &UpdateCallback<'_>, index: u32, max: u64) -> Result<Vec<u8>> {
    let Some(s) = cb.stream(index)? else {
        return Err(Error::INVALID_ARG);
    };
    let mut buf = Vec::new();
    buf.try_reserve_exact(cb.size(index)?.unwrap_or(0).min(max) as usize)?;
    s.take(max + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max {
        return Err(Error::INVALID_ARG);
    }
    cb.item_done()?;
    Ok(buf)
}

impl Update for Modpkg {
    fn update(
        existing: Option<&ModpkgArchive>,
        out: OutStream,
        cb: &UpdateCallback<'_>,
        options: &Options,
    ) -> Result<()> {
        // The tables go in first and the records after the data, which is seeked back to.
        if !out.is_seekable() {
            return Err(Error::NOT_IMPL);
        }
        let level = zstd_level(options);
        let mut pending: Vec<Pending> = Vec::new();
        pending.try_reserve_exact(cb.len() as usize)?;
        let mut config: Option<Config> = None;
        let mut game_data: IndexMap<String, Value> = IndexMap::new();
        let mut folders: IndexSet<String> = IndexSet::new();
        for item in cb.items() {
            let item = item?;
            let read = || -> Result<Read> {
                let path = cb.path(item.index)?.ok_or(Error::INVALID_ARG)?;
                path::read(&path)
            };
            match (item.new_data, item.old_index) {
                (false, Some(i)) => {
                    let a = existing.ok_or(Error::INVALID_ARG)?;
                    match *a.items().get(i as usize).ok_or(Error::INVALID_ARG)? {
                        Item::Record(r) => {
                            let r = &a.records[r as usize];
                            let chunk = if item.new_props {
                                match read()? {
                                    Read::Chunk(c) => c,
                                    _ => return Err(Error::INVALID_ARG),
                                }
                            } else {
                                chunk_of(&a.tables, r)
                            };
                            pending.push(Pending {
                                chunk,
                                source: Source::Old(i),
                                run: Some((r.data_offset, r.compressed_size)),
                                size: r.uncompressed_size,
                            });
                        }
                        Item::Config => {
                            let c = a.config().cloned().ok_or(Error::INVALID_ARG)?;
                            c.check()?;
                            config = Some(c);
                        }
                        Item::GameData(l) => {
                            let name = &a.tables.layers[l as usize].name;
                            if let Some(v) = a
                                .metadata()
                                .and_then(|m| m.layer(name))
                                .and_then(|l| l.game_data.clone())
                            {
                                game_data.insert(name.clone(), v);
                            }
                        }
                        Item::Dir(l) => {
                            folders.insert(a.tables.layers[l as usize].name.clone());
                        }
                    }
                }
                _ => {
                    let read = read()?;
                    let dir = cb.is_dir(item.index)?;
                    match read {
                        Read::Ignored | Read::Folder => {}
                        Read::Layer(l) => {
                            if !dir {
                                return Err(Error::INVALID_ARG);
                            }
                            folders.insert(l);
                        }
                        Read::Config => {
                            if dir {
                                return Err(Error::INVALID_ARG);
                            }
                            config = Some(Config::parse(&slurp(cb, item.index, MAX_CONFIG)?)?);
                        }
                        Read::GameData(l) => {
                            if dir {
                                return Err(Error::INVALID_ARG);
                            }
                            let bytes = slurp(cb, item.index, MAX_INFO)?;
                            let v =
                                serde_json::from_slice(&bytes).map_err(|_| Error::INVALID_ARG)?;
                            game_data.insert(l, config::game_data_document(v));
                        }
                        Read::Chunk(chunk) => {
                            if dir {
                                continue;
                            }
                            pending.push(Pending {
                                chunk,
                                source: Source::New(item.index),
                                run: None,
                                size: cb.size(item.index)?.unwrap_or(0),
                            });
                        }
                    }
                }
            }
        }
        let config = config.ok_or(Error::INVALID_ARG)?;
        let table = config.layer_table();
        let mut intern = Intern::new(&table);
        for f in &folders {
            intern.layer(f)?;
        }
        // Every table the config declares is there, and every table there is declared.
        let declared: IndexSet<String> = config
            .hashtables
            .iter()
            .map(|h| {
                let file = h.path.rsplit('/').next().unwrap_or(&h.path);
                format!("{HASHES_PREFIX}{file}").to_ascii_lowercase()
            })
            .collect();
        let present: IndexSet<String> = pending
            .iter()
            .filter(|p| p.chunk.layer.is_none() && p.chunk.path.starts_with(HASHES_PREFIX))
            .map(|p| p.chunk.path.to_ascii_lowercase())
            .collect();
        if declared != present {
            return Err(Error::INVALID_ARG);
        }
        let info = config.to_metadata(&table, &game_data).to_msgpack();

        let mut plan: Vec<Step> = Vec::new();
        plan.try_reserve_exact(pending.len() + 1)?;
        for p in &pending {
            plan.push(Step {
                at: intern.at(&p.chunk)?,
                source: p.source,
                run: p.run,
                size: p.size,
            });
        }
        plan.push(Step {
            at: At {
                hash: PathHash(xxh64(INFO.as_bytes(), 0)),
                path_index: intern.path(INFO)?,
                layer_index: NONE,
                wad_index: NONE,
            },
            source: Source::Info,
            run: None,
            size: info.len() as u64,
        });
        drop(pending);
        let tables = intern.into_tables();

        // New data for a chunk replaces copies of it; anything else is two chunks for one.
        let old = |s: &Step| matches!(s.source, Source::Old(_));
        plan.sort_unstable_by_key(|s| (key(&s.at), s.at.wad_index, old(s)));
        let mut twice = false;
        plan.dedup_by(|later, kept| {
            if key(&later.at) != key(&kept.at) || later.at.wad_index != kept.at.wad_index {
                return false;
            }
            twice |= old(kept) || !old(later);
            true
        });
        if twice {
            return Err(Error::INVALID_ARG);
        }
        // New data first, so a copy of a chunk it replaces under another WAD can share it;
        // then copies of the same stored bytes next to each other.
        plan.sort_unstable_by_key(|s| (old(s), s.run, s.at.hash));
        cb.set_total(plan.iter().fold(0, |t, s| t.saturating_add(s.size)))?;

        let job = Job {
            plan: &plan,
            existing,
            cb,
            level,
            info: &info,
            tables: &tables,
        };
        job.run(out)
    }
}

/// A chunk's identity: path hash and layer.
fn key(at: &At) -> (PathHash, u32) {
    (at.hash, at.layer_index)
}

/// One `update` call's inputs.
struct Job<'a> {
    plan: &'a [Step],
    existing: Option<&'a ModpkgArchive>,
    cb: &'a UpdateCallback<'a>,
    level: Option<i32>,
    info: &'a [u8],
    tables: &'a Tables,
}

impl Job<'_> {
    fn run(&self, out: OutStream) -> Result<()> {
        let signature = self.existing.map_or(&[][..], ModpkgArchive::signature);
        let mut writer = Writer::new(out, self.tables, signature, self.plan.len())?;
        let mut done = 0u64;
        // New data by chunk identity, which a copy under another WAD shares.
        let mut fresh: BTreeMap<(PathHash, u32), Record> = BTreeMap::new();
        // The last copy written, by its run.
        let mut last: Option<(Option<(u64, u64)>, Record)> = None;
        for step in self.plan {
            let added = match step.source {
                Source::New(i) => {
                    let Some(s) = self.cb.stream(i)? else {
                        // Couldn't be opened; 7-Zip has said so.
                        continue;
                    };
                    let level = self
                        .level
                        .filter(|_| !audio(&self.tables.paths[step.at.path_index as usize]));
                    let mut src = Hashing::new(self.cb.progress().reader(s, done));
                    let added = writer.add(step.at, |out| {
                        let (kind, n) = encode::encode(&mut src, out, level)?;
                        Ok((compression(kind), n, src.finish()))
                    })?;
                    self.cb.item_done()?;
                    fresh.insert(key(&step.at), added);
                    added
                }
                Source::Info => {
                    let added = writer.add(step.at, |out| {
                        out.write_all(self.info)?;
                        Ok((Compression::Raw, self.info.len() as u64, xxh3_64(self.info)))
                    })?;
                    fresh.insert(key(&step.at), added);
                    added
                }
                Source::Old(i) => {
                    let added = match (fresh.get(&key(&step.at)), last) {
                        (Some(&same), _) => writer.alias(step.at, same),
                        (None, Some((run, same))) if run == step.run => writer.alias(step.at, same),
                        _ => self.copy(&mut writer, step.at, i, done)?,
                    };
                    last = Some((step.run, added));
                    self.cb.copied(i)?;
                    added
                }
            };
            done = done.saturating_add(added.uncompressed_size);
            self.cb.set_completed(done)?;
        }
        writer.finish()
    }

    /// Copies old item `i`, a record, as stored.
    fn copy(&self, writer: &mut Writer, at: At, i: u32, done: u64) -> Result<Record> {
        let a = self.existing.ok_or(Error::INVALID_ARG)?;
        let Some(&Item::Record(r)) = a.items().get(i as usize) else {
            return Err(Error::INVALID_ARG);
        };
        let r = &a.records[r as usize];
        writer.add(at, |out| {
            let src = a.stream.slice(r.data_offset, r.compressed_size);
            let mut src = self.cb.progress().reader(src, done);
            if io::copy(&mut src, out)? != r.compressed_size {
                return Err(Error::UNEXPECTED_END);
            }
            Ok((r.compression, r.uncompressed_size, r.uncompressed_checksum))
        })
    }
}

fn compression(kind: Kind) -> Compression {
    if kind == Kind::Zstd {
        Compression::Zstd
    } else {
        Compression::Raw
    }
}

/// A Wwise audio container, which the game stores raw.
fn audio(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bnk") || ext.eq_ignore_ascii_case("wpk"))
}

/// A reader that hashes what passes through: a chunk's content checksum.
struct Hashing<R> {
    r: R,
    hash: Xxh3,
}

impl<R> Hashing<R> {
    fn new(r: R) -> Self {
        Self {
            r,
            hash: Xxh3::new(),
        }
    }

    fn finish(&self) -> u64 {
        self.hash.digest()
    }
}

impl<R: io::Read> io::Read for Hashing<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.r.read(buf)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }
}
