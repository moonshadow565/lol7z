# lol7z

A 7-Zip plugin for League of Legends files. One DLL, `lol7z.dll`, adds two formats to
7-Zip's file manager and command line:

- **WAD** (`*.wad`, `*.wad.client`): the game's archives, read in every version from 1
  to 3.4 and written as 3.4. Entries get their real paths from the Mimir hash tables.
- **modpkg** (`*.modpkg`): the LeagueToolkit mod package. A package lists as the mod
  project it was packed from, and a mod project packs back into a package.

## Why

**WADs** have been served by many tools over the years, and most of them are abandoned
now. The ones that remain handle the archive's long names poorly: a path hash the
tables don't know, or a name longer than Windows allows, gets mangled, truncated or
silently dropped, and what you extracted can't be put back. This plugin shows every
entry under a name that reads back to the same hash, folds a too-long name into a
notation that expands again, and refuses rather than guesses. The rules are in
[docs/names.md](docs/names.md).

**modpkg** is a binary container with a MessagePack document inside, which makes it an
opaque blob when you are writing a tool against it or debugging one: nothing on the
machine will show you what is in it. Opening it in 7-Zip makes it a folder tree again:
`mod.config.json`, `README.md`, `hashes/`, `content/<layer>/...`, exactly the project
layout the reference importer writes, with the document shown as the config it came
from. The layout and the pack rules are in [docs/modpkg.md](docs/modpkg.md).

## Install

Build the DLL and drop it in a `Formats` folder next to `7z.dll` (create the folder
if there isn't one; 7-Zip looks for it at startup):

```bash
cargo build --release -p lol7z
```

```
C:\Program Files\7-Zip\Formats\lol7z.dll
```

`7z i` then lists `WAD` and `modpkg` among the formats.

## Use

Anything 7-Zip does with an archive. The switches that matter:

```bash
7z l Aatrox.wad.client
7z x -o out my-mod_1.0.0.modpkg
7z a -tmodpkg my-mod_1.0.0.modpkg ./my-mod/*
7z a -twad -mx=9 Aatrox.wad.client ./assets/*
```

`-mx=N` picks the zstd level for both formats, 0 stores. Packing a modpkg needs a
`mod.config.json` in the tree.

## Layout

| Crate | What it is |
|---|---|
| `crates/lol7z-sys` | Bindings to 7-Zip's COM interfaces, generated from its headers |
| `crates/lol7z-api` | A safe API for writing a 7-Zip format on top of the bindings |
| `crates/lol7z-fold` | The notation that folds an over-long file name and expands it again |
| `crates/lol7z-wad` | The WAD format |
| `crates/lol7z-modpkg` | The modpkg format |
| `crates/lol7z` | The DLL: the formats above, exported |

License: MIT.

## 🤖 Robots inside

Much of this code was written with Claude. I reviewed the `lol7z-sys` bindings, which
hold the `unsafe` that talks to 7-Zip, and did a lighter review of `lol7z-api`, which
also holds `unsafe` and builds on them. The format crates are covered by tests against
real archives, the format specification and 7-Zip itself.
