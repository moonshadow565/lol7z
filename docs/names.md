# Item names in WAD archives

How the 7-Zip plugin turns a WAD entry's path hash into a file name, and how it turns
a file name back into the hash. Every rule here is reversible without the plugin: a
name alone is enough to recover the hash, except one case marked below.

## Hash

An entry is identified by `xxh64(path, seed=0)` over the ASCII-lowercased path with
`/` separators. Paths are looked up in the Mimir `game` and `lcu` tables; a table path
is used only if it hashes back to the entry.

## Display forms

| Form | Example | When |
|---|---|---|
| plain | `assets/characters/ahri/skins/base/ahri.skn` | the path fits |
| folded | `data/characters/ahri/ahri_multi«_skins_skin{0…84}».bin` | a file name over 255 UTF-16 units that folds under 255 |
| hashed | `characters/pettftavatar_multi«76714777541aa1e8-e0227d0e2478b12c».bin` | too long even folded |
| unknown | `«003cc22b823ddd78»` | no table knows the hash |

The block is told apart by its contents, never guessed:

- 16 hex digits: unknown hash.
- 16 hex, `-`, 16 hex: hashed form, `FULL-OUTER`.
- contains `{`: a fold.
- anything else is not a block the plugin wrote; the name is hashed literally.

Bare `0123456789abcdef` and `0123456789abcdef.ext` (16-hex stem) are also read as a
hash, for files made by other tools. The plugin never writes them.

## Characters Windows can't store

Applied to plain and folded names, the same way 7-Zip maps `\`:

| In the path | Shown as |
|---|---|
| `< > : " \| ? * \` and control characters | U+F000 + the character (`:` → U+F03A) |
| an empty component (`a//b`) | U+F02F as the component (`a//b`) |
| a file whose path is also a folder's | the name plus a trailing U+F02F |

Read back: a unit in U+F000..U+F0FF is its low byte; a component that is only U+F02F is
empty; one trailing U+F02F is dropped.

## Fold notation

A block `«…»` is a template, like bash brace expansion. Each group element is
substituted once and the expansions are concatenated with nothing between them.

```
name    = outer "«" seq "»" outer          exactly one block
seq     = { term }                          an empty seq is one empty element
term    = lit | [lit] group { group }       several groups multiply: L{a,b}{c,d} → Lac Lad Lbc Lbd
group   = "{" numset "}" | "{" seq { "," seq } "}"
numset  = nitem { "," nitem }               a group that parses as a numset is one
nitem   = digits [ "…" digits ]             equal widths keep zero padding: 007…011
lit     = 1*( char - { } , « » … )
```

- A numset's elements are its numbers as strings, sorted bytewise: `{0…12}` gives
  `0 1 10 11 12 2 3 … 9`. This is the order real names use.
- Alternatives expand in written order.
- `…` is U+2026, one unit. Three ASCII dots are a literal.
- A number has at most 9 digits; groups nest at most 8 deep; a decoded name is at most
  65536 bytes. These guard against hostile names only.

`sightward_multi«_skins_skin{0…29,32…39}».bin` expands `_skins_skin{N}` per N:
`sightward_multi_skins_skin0_skins_skin1_skins_skin10_…_skins_skin39.bin`.

The encoder is deterministic. Its preamble, the text before the block, is the maximal
prefix that takes part in no repetition, and the hashed form reuses it.

## Hashed form

`dir/preamble«FULL-OUTER».ext`

- `FULL` is the entry's hash.
- `OUTER` is `xxh64` of the display path with the block removed: `dir/preamble.ext`,
  lowercased, `/` separators.
- `preamble` and `.ext` are what the fold's `split` keeps within 219 units
  (255 minus 35 for the block and 1 for a clash mark).

Read back: if `xxh64(dir/preamble.ext) == OUTER` the entry is `FULL`. Otherwise the
file was moved, renamed or given another extension. Then, and only then, the tables are
needed: look `FULL` up, split the known name the same way, and hash the user's
`dir/preamble` + hidden middle + `.ext`. Without tables the file is refused.

Moving characters between the preamble and the extension around the block is not
detected; by design.

## Reference implementation

[reference.py](reference.py) is a dependency-free Python reference: `decode` for the
fold notation, `xxh64`/`game_hash`, the character mapping, `split` for the hashed form,
and `read_hash(name, lookup)` which turns any shown name back into its hash. It is
verified against every real over-long name. The fold *encoder* is the `lol7z_fold`
crate only; the grammar above is its whole contract.

```
echo 'sightward_multi«_skins_skin{0…2}».bin' | python docs/reference.py
```

