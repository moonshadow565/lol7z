# Item names in modpkg packages

How the 7-Zip plugin lists a `.modpkg` (the LeagueToolkit mod package, specified in
league-mod's `docs/design/modpkg.md`), and how it reads a listing back when files are
added. The package stores every path, layer name and WAD name, so no hash tables are
involved. The listing is the mod project layout the reference importer
(`ltk_mod_project`) unpacks a package to, so an extraction is a project and a project
packs.

## Listing

| Chunk | Lists as |
|---|---|
| `_meta_/info.msgpack` | `mod.config.json`, the document as the project's config |
| `_meta_/readme.md`, `_meta_/license`, `_meta_/thumbnail.webp` | `README.md`, `LICENSE`, `thumbnail.webp` |
| `_meta_/hashes/<file>` | `hashes/<file>` |
| a layer's `game_data` in the document | `content/<layer>/game_data.json` |
| content, with a WAD target | `content/<layer>/<WAD>/<stored path>` |
| content, no WAD target | `content/<layer>/<stored path>` |
| every layer | a folder `content/<layer>`, so an empty layer survives a round trip |
| any other meta chunk | its stored path, `_meta_/...` |

`mod.config.json` and each `game_data.json` are made at open and held in memory. The
config follows the reference conversion: the layer table's names and priorities (the
document's copy is informational), the document's display name, description and string
overrides per layer, authors as a string or a name-role object, the license as an SPDX
string or a name-url object, hashtable paths rewritten to `hashes/<file>`. A key the
plugin doesn't model is carried into the config as it is, and a package's `distributor`
rides along the same way. A module's `origin` in a game-data document is the package's
record of the manifest it was packed from, and is left out of the manifest shown.

When the document doesn't read, the package lists as it is, `_meta_/info.msgpack`
included.

The stored path keeps the author's casing; the WAD name shows as the table holds it.
Characters Windows can't store, empty components, a file whose path is also a folder's,
and a last component over 255 UTF-16 units are shown exactly as the WAD plugin shows
them: see [names.md](names.md) for the U+F000 mapping, the fold notation and the hashed
form. The record's path hash is what a hashed form carries.

## Reading a listing back

Applied to every name 7-Zip gives when adding, renaming or re-packing:

1. `\` is `/`; a unit in U+F000..U+F0FF is its low byte; a component that is only
   U+F02F is empty; one trailing U+F02F is dropped. In the last component, a `«…»`
   block is read back: a fold is expanded, `«H»` or `«FULL-OUTER»` is the hash `H` or
   `FULL`, and the chunk becomes hex-named (`<hash>.ext`).
2. `mod.config.json` is the config. `README.md`, `LICENSE` (also `.md`, `.txt`),
   `thumbnail.webp` and `hashes/<file>` are their meta chunks; `_meta_/...` is a meta
   chunk as written. `.modignore` is dropped. File names compare without case.
3. `content/<slug>` is a layer; a folder item there declares it, chunks under it belong
   to it. `content/<slug>/game_data.json` is the layer's game-data manifest; each module
   without an `origin` gets one naming that file.
4. Under a layer, a top-level folder named `*.wad.client` or `*.wad` (any case) is the
   WAD target, stored lowercased. Anything else is a chunk with no WAD.
5. The chunk path is the rest. A path with no `/` whose stem is 16 hex digits is a
   hex-named chunk: the hash is the digits, the name is stored lowercased. Any other path
   hashes as the game hashes it (xxh64 of the ASCII-lowercased path).

A name outside the layout, a `..` component, a `:`, or a content chunk under `_meta_/`
is refused with `E_INVALIDARG`.

## Writing

A pack needs `mod.config.json`, from the host or kept from the package being updated,
and it is checked either way:

- `name`, `display_name` and `version` are non-empty; every layer name is a slug;
  `base`, if listed, has priority 0.
- Without a `layers` list the only layer is `base`. With one, every layer folder and
  every chunk's layer must be listed; a listed layer with nothing in it is still a
  layer. The table goes `base` first, then by priority and name.
- Every `hashtables` entry is `hashes/<file>`, its file is among the items, and every
  `hashes/<file>` item is declared. The reference packer refuses the same.

The document is written from the config: schema 5, this plugin as `generator`, the
config's unknown keys and `distributor` back where they were, each layer's game-data
document. A kept package's document is therefore equivalent to what it had, not
byte-identical, which is also what the reference packer does.

- WADs and paths go in the tables in the order first seen, each once by canonical
  (ASCII-lowercased) name.
- Records are written meta chunks first, then by WAD, layer and path hash. The data runs
  follow the order the items were given.
- Identical content (same XXH3 and size) is stored once; later records point at the
  first run. New data for a chunk replaces every copy of that chunk, under every WAD.
- Zstd at the `-mx` level (0 stores; absent is 3). `.bnk` and `.wpk`, and anything
  that starts like a Wwise bank, stay raw.
- The signature bytes of an existing package are carried through.

## Opening

Refused outright, nothing listed: wrong magic, a format version other than 1, a table
cut short, a name that isn't UTF-8, a record whose compression isn't 0 or 1 or whose
path, layer or WAD index is past its table.

Listed with a headers warning: no `base` layer, `base` not at priority 0, a layer name
that isn't a slug, a name that would escape the output directory (`..`, a leading
separator, `:`, `\`), a chunk under `_meta_/` with a layer or WAD, a chunk with no
layer outside `_meta_/`, two records of one path and layer with different content, or
two records of one path, layer and WAD.

Listed with an unexpected-end error: more records than the file holds (those that fit
are listed), or a chunk whose data runs past the end.

Extraction checks the decoded length against `uncompressed_size` and the XXH3 of the
decoded bytes against `uncompressed_checksum`.

The archive comment is `<display name> <version> by <authors>; layers: base (0), ...`,
from the document when it reads.
