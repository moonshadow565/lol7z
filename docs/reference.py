"""Reference for WAD item names: fold decoding, the character mapping, and reading a
name back to its hash. See names.md for the rules. Clarity over speed; no dependencies.

    >>> decode("sightward_multi«_skins_skin{0…2,10}».bin")
    'sightward_multi_skins_skin0_skins_skin1_skins_skin10_skins_skin2.bin'
    >>> "%016x" % read_hash("data/final/x.bin")
    '905e9e32472ea331'
    >>> "%016x" % read_hash("any/folder/«0123456789abcdef»")
    '0123456789abcdef'
"""

import re

# ---------------------------------------------------------------- fold notation

OPEN, CLOSE, RANGE = "«", "»", "…"   # « » …
RESERVED = set("{}," + OPEN + CLOSE + RANGE)
MAX_DEPTH = 8          # nesting of groups
MAX_DIGITS = 9         # per number
MAX_DECODED = 1 << 16  # bytes of output; a guard against hostile names


class FoldError(ValueError):
    pass


def decode(name: str) -> str:
    """The name `name` folds, or `name` itself when it has no block."""
    a = name.find(OPEN)
    if a == -1:
        if CLOSE in name:
            raise FoldError("close without open")
        return name
    p = Parser(name, a + 1)
    elements = p.seq(depth=0)
    if p.peek() != CLOSE:
        raise FoldError("expected " + CLOSE)
    post = name[p.i + 1 :]
    if OPEN in post or CLOSE in post or CLOSE in name[:a]:
        raise FoldError("more than one block")
    out = name[:a] + "".join(elements) + post
    if len(out.encode()) > MAX_DECODED:
        raise FoldError("too large")
    return out


class Parser:
    """Recursive descent over the block. Every method returns the list of elements its
    construct expands to, in order."""

    def __init__(self, s: str, i: int):
        self.s, self.i = s, i

    def peek(self):
        return self.s[self.i] if self.i < len(self.s) else None

    # seq = { term }        elements of the terms, concatenated into one list
    def seq(self, depth):
        elements = []
        while self.peek() not in (None, CLOSE, "}", ","):
            elements += self.term(depth)
        return elements or [""]

    # term = lit | [lit] group { group }      groups multiply: L{a,b}{c,d} -> Lac Lad Lbc Lbd
    def term(self, depth):
        elements = [self.literal()] if self.peek() != "{" else [""]
        while self.peek() == "{":
            group = self.group(depth)
            elements = [e + g for e in elements for g in group]
            self.check(len(elements))
        return elements

    def literal(self):
        j = self.i
        while self.peek() is not None and self.peek() not in RESERVED:
            self.i += 1
        if j == self.i:
            raise FoldError("empty literal at %d" % j)
        return self.s[j : self.i]

    # group = "{" numset "}" | "{" seq { "," seq } "}"
    def group(self, depth):
        if depth >= MAX_DEPTH:
            raise FoldError("nested too deep")
        self.i += 1  # {
        j = self.i
        k = self.s.find("}", j)
        if k == -1:
            raise FoldError("unclosed group")
        inner = self.s[j:k]
        if re.fullmatch(r"\d+(%s\d+)?(,\d+(%s\d+)?)*" % (RANGE, RANGE), inner):
            self.i = k + 1
            return numset(inner)
        alternatives = []
        while True:
            alternatives += self.seq(depth + 1)
            self.check(len(alternatives))
            if self.peek() == ",":
                self.i += 1
                continue
            if self.peek() == "}":
                self.i += 1
                return alternatives
            raise FoldError("expected , or } at %d" % self.i)

    def check(self, count):
        if count > MAX_DECODED:
            raise FoldError("too large")


def numset(inner: str):
    """The numbers of `{a…b,c}` as strings, sorted bytewise. Equal-width ends keep zero
    padding (007…011); otherwise natural numbers without leading zeros."""
    numbers = set()
    for item in inner.split(","):
        lo, _, hi = item.partition(RANGE)
        hi = hi or lo
        if len(lo) > MAX_DIGITS or len(hi) > MAX_DIGITS:
            raise FoldError("number too long")
        natural = lambda n: n == "0" or not n.startswith("0")  # no leading zero
        if len(lo) == len(hi):
            width = len(lo)
        elif len(lo) < len(hi) and natural(lo) and natural(hi):
            width = 0
        else:
            raise FoldError("bad range " + item)
        a, b = int(lo), int(hi)
        if a > b:
            raise FoldError("bad range " + item)
        if b - a + 1 > MAX_DECODED:
            raise FoldError("too large")
        numbers.update(str(n).zfill(width) for n in range(a, b + 1))
    return sorted(numbers, key=lambda n: n.encode())


# ---------------------------------------------------------------- the game's hash

M = (1 << 64) - 1
P1, P2, P3, P4, P5 = (0x9E3779B185EBCA87, 0xC2B2AE3D27D4EB4F, 0x165667B19E3779F9,
                      0x85EBCA77C2B2AE63, 0x27D4EB2F165667C5)


def _rotl(x, r):
    return ((x << r) | (x >> (64 - r))) & M


def _round(acc, lane):
    return (_rotl((acc + lane * P2) & M, 31) * P1) & M


def xxh64(data: bytes, seed: int = 0) -> int:
    n, i = len(data), 0
    if n >= 32:
        v = [(seed + P1 + P2) & M, (seed + P2) & M, seed, (seed - P1) & M]
        while i + 32 <= n:
            for k in range(4):
                v[k] = _round(v[k], int.from_bytes(data[i + 8 * k : i + 8 * k + 8], "little"))
            i += 32
        h = (_rotl(v[0], 1) + _rotl(v[1], 7) + _rotl(v[2], 12) + _rotl(v[3], 18)) & M
        for x in v:
            h = ((h ^ _round(0, x)) * P1 + P4) & M
    else:
        h = (seed + P5) & M
    h = (h + n) & M
    while i + 8 <= n:
        h ^= _round(0, int.from_bytes(data[i : i + 8], "little"))
        h = (_rotl(h, 27) * P1 + P4) & M
        i += 8
    if i + 4 <= n:
        h ^= (int.from_bytes(data[i : i + 4], "little") * P1) & M
        h = (_rotl(h, 23) * P2 + P3) & M
        i += 4
    while i < n:
        h ^= (data[i] * P5) & M
        h = (_rotl(h, 11) * P1) & M
        i += 1
    h ^= h >> 33
    h = (h * P2) & M
    h ^= h >> 29
    h = (h * P3) & M
    h ^= h >> 32
    return h


def game_hash(path: str) -> int:
    """XXH64 of the ASCII-lowercased path with `/` separators."""
    return xxh64(ascii_lower(path.replace("\\", "/")).encode())


def ascii_lower(s: str) -> str:
    return "".join(c.lower() if "A" <= c <= "Z" else c for c in s)


# ---------------------------------------------------------------- characters Windows can't store

FORBIDDEN = set('<>:"|?*\\') | {chr(c) for c in range(0x20)}
EMPTY = ""   # an empty component; also the trailing clash mark


def show_component(part: str) -> str:
    if part == "":
        return EMPTY
    return "".join(chr(0xF000 + ord(c)) if c in FORBIDDEN else c for c in part)


def unshow(shown: str) -> str:
    """Inverse of show_component over a whole path."""
    parts = []
    for part in shown.split("/"):
        if part == EMPTY:
            parts.append("")
        else:
            parts.append("".join(chr(ord(c) - 0xF000) if 0xF000 <= ord(c) <= 0xF0FF else c
                                 for c in part))
    return "/".join(parts)


# ---------------------------------------------------------------- the hashed form

MAX_NAME = 255             # UTF-16 units per component on NTFS
ROOM = MAX_NAME - 35 - 1   # for preamble + ext: the block takes 35, a clash mark 1
HEX16 = r"[0-9a-fA-F]{16}"


def units(s: str) -> int:
    return len(s.encode("utf-16-le")) // 2


def split_ext(name: str):
    d = name.rfind(".")
    return (name[:d], name[d:]) if d > 0 else (name, "")


def tokens(stem: str):
    """The stem split before every `_` (the first token may lack one)."""
    return re.findall(r"^[^_]+|_[^_]*", stem) if stem else []


def preamble_tokens(toks) -> int:
    """How many leading tokens take part in no repetition: the encoder's preamble.

    Tokens are read in pairs from an even or odd start (whichever gives fewer runs of
    pairs sharing a first token); the preamble ends at the first run of 2+ pairs."""
    def runs(p):
        out = []
        for k in range(p, len(toks) - 1, 2):
            if out and toks[out[-1][0]] == toks[k]:
                out[-1][1] += 1
            else:
                out.append([k, 1])
        return out
    even, odd = runs(0), runs(1)
    chosen = odd if len(odd) < len(even) else even
    for start, count in chosen:
        if count >= 2:
            return start
    return len(toks)


def split(name: str, room: int = ROOM):
    """(preamble, ext) kept visible for a name too long even folded, within `room` units:
    the extension goes if it alone is longer, then the preamble loses tokens from its end."""
    stem, ext = split_ext(name)
    if units(ext) > room:
        ext = ""
    toks = tokens(stem)
    pre = "".join(toks[: preamble_tokens(toks)])
    while units(pre) > room - units(ext):
        pre = pre[: pre.rfind("_")] if "_" in pre else ""
    return pre, ext


def hashed_name(path: str, full: int) -> str:
    """`dir/preamble«FULL-OUTER».ext` for a name too long even folded."""
    folder, _, name = path.rpartition("/")
    folder = folder + "/" if folder else ""
    pre, ext = split(name)
    outer = game_hash(folder + pre + ext)
    return "%s%s%s%016x-%016x%s%s" % (folder, pre, OPEN, full, outer, CLOSE, ext)


# ---------------------------------------------------------------- name -> hash

def read_hash(shown: str, lookup=None) -> int:
    """The hash a shown name stands for. `lookup(full) -> table path or None` is needed
    only for a hashed form that was moved, renamed or given another extension."""
    shown = shown.replace("\\", "/")
    if shown.endswith(EMPTY):
        shown = shown[:-1]
    folder, _, name = shown.rpartition("/")
    folder = folder + "/" if folder else ""
    a, b = name.find(OPEN), name.find(CLOSE)
    if a == -1:
        stem = name.split(".", 1)[0]
        if re.fullmatch(HEX16, stem):
            return int(stem, 16)                       # bare hex, written by other tools
        return game_hash(unshow(shown))
    pre, inner, ext = name[:a], name[a + 1 : b], name[b + 1 :]
    if re.fullmatch(HEX16, inner):
        return int(inner, 16)                          # unknown hash: location independent
    if re.fullmatch(HEX16 + "-" + HEX16, inner):
        full, outer = (int(h, 16) for h in inner.split("-"))
        if game_hash(unshow(folder + pre + ext)) == outer:
            return full                                # not moved
        known = lookup(full) if lookup else None       # moved: the tables are required
        if known is None:
            raise FoldError("moved hashed name needs the tables")
        kname = known.rpartition("/")[2]
        kpre, kext = split(kname)
        middle = kname[len(kpre) : len(kname) - len(kext)]
        return game_hash(unshow(folder + pre) + middle + unshow(ext))
    if "{" in inner:
        return game_hash(unshow(folder) + decode(name))  # folded
    return game_hash(unshow(shown))                    # not a block the plugin writes


def display(path, full: int, folded=None, is_also_folder=False) -> str:
    """What the plugin shows. `folded` is the Rust encoder's output for the file name when
    it is over 255 units (the encoder is the lol7z_fold crate, not reproduced here)."""
    if path is None or game_hash(path) != full:
        return "%s%016x%s" % (OPEN, full, CLOSE)
    folder, _, name = path.rpartition("/")
    if units(name) > MAX_NAME:
        if folded is not None and units(folded) <= MAX_NAME:
            name = folded
            path = (folder + "/" if folder else "") + name
        else:
            return hashed_name(path, full)
    shown = "/".join(show_component(p) for p in path.split("/"))
    return shown + EMPTY if is_also_folder else shown


if __name__ == "__main__":
    import sys

    sys.stdin.reconfigure(encoding="utf-8")
    sys.stdout.reconfigure(encoding="utf-8")
    for line in sys.stdin:
        name = line.rstrip("\n")
        print("%016x  %s" % (read_hash(name), decode(name)))
