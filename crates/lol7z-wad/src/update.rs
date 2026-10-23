//! Writing a 3.4 WAD from 7-Zip's update callback.

use crate::archive::WadArchive;
use crate::encode;
use crate::format::Wad;
use crate::hash::PathHash;
use crate::resolver::Resolver;
use crate::toc::Entry;
use crate::writer::Writer;
use lol7z_api::{Error, Options, OutStream, Result, Update, UpdateCallback};
use std::io;

/// Where a new archive's item comes from.
#[derive(Clone, Copy)]
enum Source {
    /// Data from the host, by update index.
    New(u32),
    /// Copied as stored, by index in the existing archive.
    Old(u32),
}

/// One item of the new archive.
#[derive(Clone, Copy)]
struct Step {
    hash: PathHash,
    source: Source,
    /// Where a copy's stored bytes are; `None` for new data.
    group: Option<Stored>,
    size: u64,
}

/// Checksum, offset, size, kind, subchunk count and subchunk index.
type Stored = (u64, u32, u32, u8, u8, u32);

fn stored(e: &Entry) -> Stored {
    (
        e.checksum,
        e.offset,
        e.size,
        e.kind.0,
        e.subchunk_count,
        e.subchunk_index,
    )
}

impl Update for Wad {
    fn update(
        existing: Option<&WadArchive>,
        out: OutStream,
        cb: &UpdateCallback<'_>,
        options: &Options,
    ) -> Result<()> {
        // The table goes in last, as 7-Zip's 7z handler does it.
        if !out.is_seekable() {
            return Err(Error::NOT_IMPL);
        }
        let level = zstd_level(options);
        let mut names = Resolver::new();
        let mut plan: Vec<Step> = Vec::new();
        plan.try_reserve_exact(cb.len() as usize)?;
        for item in cb.items() {
            let item = item?;
            let path = || cb.path(item.index)?.ok_or(Error::INVALID_ARG);
            let step = match (item.new_data, item.old_index) {
                (false, Some(i)) => {
                    let a = existing.ok_or(Error::INVALID_ARG)?;
                    let e = &a.items.get(i as usize).ok_or(Error::INVALID_ARG)?.entry;
                    Step {
                        hash: if item.new_props {
                            names.hash(&path()?)?
                        } else {
                            e.hash
                        },
                        source: Source::Old(i),
                        group: Some(stored(e)),
                        size: e.size_decompressed.into(),
                    }
                }
                _ => {
                    if cb.is_dir(item.index)? {
                        continue;
                    }
                    Step {
                        hash: names.hash(&path()?)?,
                        source: Source::New(item.index),
                        group: None,
                        size: cb.size(item.index)?.unwrap_or(0),
                    }
                }
            };
            plan.push(step);
        }
        // New data for a hash replaces copies of it; anything else is two items for one.
        plan.sort_unstable_by_key(|s| (s.hash, matches!(s.source, Source::Old(_))));
        let mut twice = false;
        plan.dedup_by(|later, kept| {
            if later.hash != kept.hash {
                return false;
            }
            twice |= !matches!(
                (kept.source, later.source),
                (Source::New(_), Source::Old(_))
            );
            true
        });
        if twice {
            return Err(Error::INVALID_ARG);
        }
        plan.sort_unstable_by_key(|s| (s.group, s.hash));
        cb.set_total(plan.iter().fold(0, |t, s| t.saturating_add(s.size)))?;

        let signature = existing.map_or([0; 256], |a| a.header.signature);
        let job = Job {
            plan: &plan,
            existing,
            cb,
            level,
            signature,
        };
        job.run(out)
    }
}

/// One `update` call's inputs.
struct Job<'a> {
    /// Copies of the same stored bytes next to each other.
    plan: &'a [Step],
    existing: Option<&'a WadArchive>,
    cb: &'a UpdateCallback<'a>,
    level: Option<i32>,
    signature: [u8; 256],
}

impl Job<'_> {
    /// Each entry streams straight into the file after room for the table; the table goes
    /// in last.
    fn run(&self, out: OutStream) -> Result<()> {
        let mut writer = Writer::new(out, self.plan.len())?;
        let mut done = 0u64;
        // The last copy written, by its group.
        let mut last: Option<(Option<Stored>, Entry)> = None;
        for step in self.plan {
            let added = match step.source {
                Source::New(i) => {
                    let Some(s) = self.cb.stream(i)? else {
                        // Couldn't be opened; 7-Zip has said so.
                        continue;
                    };
                    let src = self.cb.progress().reader(s, done);
                    let added = writer.add(step.hash, |out| {
                        let (kind, n) = encode::encode(src, out, self.level)?;
                        Ok((kind, n, 0, 0))
                    })?;
                    self.cb.item_done()?;
                    added
                }
                Source::Old(i) => {
                    let added = match last {
                        Some((group, same)) if group == step.group => writer.alias(step.hash, same),
                        _ => self.copy(&mut writer, step.hash, i, done)?,
                    };
                    last = Some((step.group, added));
                    self.cb.copied(i)?;
                    added
                }
            };
            done = done.saturating_add(u64::from(added.size_decompressed));
            self.cb.set_completed(done)?;
        }
        writer.finish(&self.signature)
    }

    /// Copies old entry `i` as stored.
    fn copy(&self, writer: &mut Writer, hash: PathHash, i: u32, done: u64) -> Result<Entry> {
        let a = self.existing.ok_or(Error::INVALID_ARG)?;
        let e = &a.items.get(i as usize).ok_or(Error::INVALID_ARG)?.entry;
        writer.add(hash, |out| {
            let src = a.stream.slice(e.offset.into(), e.size.into());
            let mut src = self.cb.progress().reader(src, done);
            if io::copy(&mut src, out)? != u64::from(e.size) {
                return Err(Error::UNEXPECTED_END);
            }
            Ok((
                e.kind,
                e.size_decompressed.into(),
                e.subchunk_count,
                e.subchunk_index,
            ))
        })
    }
}

/// `-mx`: 0 stores, 1 to 9 map onto zstd levels 1 to 19; absent is zstd 3.
pub fn zstd_level(options: &Options) -> Option<i32> {
    let x = match options.get("x") {
        None => return Some(3),
        Some(o) => match &o.value {
            None => 9,
            Some(v) => v
                .as_u64()
                .or_else(|| v.as_str()?.trim().parse().ok())
                .unwrap_or(9),
        },
    };
    match x {
        0 => None,
        1..=4 => Some(x as i32),
        5 => Some(6),
        6 => Some(9),
        7 => Some(12),
        8 => Some(16),
        _ => Some(19),
    }
}
