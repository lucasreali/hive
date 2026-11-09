//! A kept terminal's last output (26.15): what a remote service holds of each terminal so an
//! app that reattaches gets what it missed (26.16), however long the program kept writing.

use std::collections::VecDeque;

/// How much of a terminal's output a remote service keeps (26.15, ledger 26.1 (2)).
pub const RING: usize = 1024 * 1024;

/// The last `cap` bytes of a terminal's output, and how many it ever wrote: byte offsets
/// count from the terminal's first byte, so `start()..end()` is what is still held.
pub struct Ring {
    bytes: VecDeque<u8>,
    cap: usize,
    end: u64,
}

impl Ring {
    pub fn new(cap: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            cap,
            end: 0,
        }
    }

    /// Adds `bytes`, dropping the oldest ones past the capacity.
    pub fn push(&mut self, bytes: &[u8]) {
        self.end += bytes.len() as u64;
        let kept = &bytes[bytes.len().saturating_sub(self.cap)..];
        let over = (self.bytes.len() + kept.len()).saturating_sub(self.cap);
        self.bytes.drain(..over);
        self.bytes.extend(kept);
    }

    /// The offset of the first byte still held.
    pub fn start(&self) -> u64 {
        self.end - self.bytes.len() as u64
    }

    /// The offset after the last byte written: every byte the terminal ever wrote.
    pub fn end(&self) -> u64 {
        self.end
    }

    /// The bytes from offset `from` to the end, when `from` is still held (`start()..=end()`);
    /// `None` otherwise, when the reader missed bytes that are gone.
    pub fn since(&self, from: u64) -> Option<Vec<u8>> {
        let skip = usize::try_from(from.checked_sub(self.start())?).ok()?;
        (from <= self.end).then(|| self.bytes.range(skip..).copied().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_the_last_bytes_and_counts_every_one() {
        let mut ring = Ring::new(4);
        assert_eq!((ring.start(), ring.end()), (0, 0));
        assert_eq!(ring.since(0), Some(vec![]));
        ring.push(b"ab");
        ring.push(b"cd");
        assert_eq!((ring.start(), ring.end()), (0, 4));
        assert_eq!(ring.since(0).unwrap(), b"abcd");
        // Past the capacity, the oldest bytes go.
        ring.push(b"ef");
        assert_eq!((ring.start(), ring.end()), (2, 6));
        assert_eq!(ring.since(2).unwrap(), b"cdef");
        assert_eq!(ring.since(5).unwrap(), b"f");
        // At the end: nothing missing.
        assert_eq!(ring.since(6), Some(vec![]));
        // Before the start, or past the end: not held.
        assert_eq!(ring.since(1), None);
        assert_eq!(ring.since(7), None);
        // A write bigger than the ring keeps its own last bytes.
        ring.push(b"ghijklm");
        assert_eq!((ring.start(), ring.end()), (9, 13));
        assert_eq!(ring.since(9).unwrap(), b"jklm");
    }

    #[test]
    fn the_ring_is_one_mebibyte() {
        assert_eq!(RING, 1_048_576);
    }
}
