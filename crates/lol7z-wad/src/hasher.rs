//! [`Hasher`]: a path's hash from its pieces, normalized on the way in, without
//! allocating.

use crate::hash::PathHash;
use xxhash_rust::xxh64::Xxh64;

pub(crate) struct Hasher {
    h: Xxh64,
    buf: [u8; 64],
    n: usize,
    /// At the start of a component.
    start: bool,
    /// The component so far is one U+F02F, which is nothing if it ends there.
    slash: bool,
}

impl Hasher {
    pub fn new() -> Self {
        Self {
            h: Xxh64::new(0),
            buf: [0; 64],
            n: 0,
            start: true,
            slash: false,
        }
    }

    /// `s` as the game hashes it: ASCII lowercased.
    pub fn game(&mut self, s: &str) {
        for b in s.bytes() {
            self.byte(b.to_ascii_lowercase());
        }
    }

    /// `s` read back from how [`Names`](crate::Names) shows it: `\` is `/`, a unit in
    /// U+F000..=U+F0FF is its low byte, a component that is only U+F02F is empty, then as
    /// [`game`](Self::game).
    pub fn shown(&mut self, s: &str) {
        for c in s.chars() {
            if c == '/' || c == '\\' {
                self.slash = false;
                self.start = true;
                self.byte(b'/');
                continue;
            }
            if self.slash {
                self.slash = false;
                self.byte(b'/');
            }
            if self.start && c == '\u{F02F}' {
                self.start = false;
                self.slash = true;
                continue;
            }
            self.start = false;
            match u32::from(c) {
                u @ 0xF000..=0xF0FF => match u as u8 {
                    b'\\' => self.byte(b'/'),
                    b => self.byte(b.to_ascii_lowercase()),
                },
                _ => {
                    let mut b = [0; 4];
                    for &b in c.to_ascii_lowercase().encode_utf8(&mut b).as_bytes() {
                        self.byte(b);
                    }
                }
            }
        }
    }

    fn byte(&mut self, b: u8) {
        if self.n == self.buf.len() {
            self.h.update(&self.buf);
            self.n = 0;
        }
        self.buf[self.n] = b;
        self.n += 1;
    }

    pub fn finish(mut self) -> PathHash {
        self.h.update(&self.buf[..self.n]);
        PathHash(self.h.digest())
    }
}
