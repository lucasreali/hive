//! Following lines as a file changes (27.5, shared with 27.3's anchored comments): line
//! hashes, a bounded diff of two hash vectors, mapping a line range through it, and finding a
//! run of lines again by its text and context, nearest to where it was.
//!
//! Pure functions, no I/O. Only hashes are compared, never text, so a caller keeps no file
//! text: it hashes what it read and drops it. Line numbers here are 0-based indexes into the
//! hash vectors; ranges are half-open.

use std::ops::Range;

/// Most edits (inserted plus deleted lines, after the common start and end are set aside)
/// [`diff`] looks for. Past it every line in between counts as replaced, so the cost stays
/// bounded by the input: time `O((N + M) · MAX_EDITS)`, memory `O(MAX_EDITS²)` (8 MiB).
pub const MAX_EDITS: usize = 1000;

/// Non-blank characters that make a single line worth finding again on its own.
const SIGNIFICANT: usize = 8;

/// Shortest run of lines found again on its own when none of them is significant.
const LONG_RUN: usize = 3;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The hash of one line: FNV-1a 64 over its bytes without the `\n` and without a trailing
/// `\r` (so CRLF and LF files hash alike). Its lowest bit is replaced by whether the line is
/// significant (at least 8 non-blank characters, [`significant`]): a lone `}` or a blank line
/// is never found again on its own. Stable across Rust versions and platforms, unlike std's
/// `DefaultHasher`.
pub fn line_hash(line: &[u8]) -> u64 {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let hash = fnv(FNV_OFFSET, line);
    let solid = line.iter().filter(|b| !b.is_ascii_whitespace()).count();
    (hash & !1) | u64::from(solid >= SIGNIFICANT)
}

fn fnv(seed: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(seed, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    })
}

/// Whether the line hashed to `hash` has at least 8 non-blank characters.
pub fn significant(hash: u64) -> bool {
    hash & 1 == 1
}

/// The hashes of `text`'s lines (a final `\n` ends the last line, it does not start an empty
/// one), or `None` when it has more than `most` lines.
pub fn hashes(text: &[u8], most: usize) -> Option<Vec<u64>> {
    let body = text.strip_suffix(b"\n").unwrap_or(text);
    if text.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for line in body.split(|&b| b == b'\n') {
        if out.len() == most {
            return None;
        }
        out.push(line_hash(line));
    }
    Some(out)
}

/// One hash of a whole hash vector (FNV-1a 64 over the hashes' little-endian bytes), e.g. to
/// tell whether a file is still the one a map was made from.
pub fn digest(hashes: &[u64]) -> u64 {
    hashes
        .iter()
        .fold(FNV_OFFSET, |hash, h| fnv(hash, &h.to_le_bytes()))
}

/// Where each old line went in the new file, from [`diff`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMap {
    old_to_new: Vec<Option<usize>>,
    new_len: usize,
}

/// A range of old lines in the new file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mapped {
    /// Every line is still there, together, at this range.
    Kept(Range<usize>),
    /// Some lines are gone or others came in between; the range spans what is left.
    Partly(Range<usize>),
    /// None of the lines is left.
    Gone,
}

impl LineMap {
    /// The new line of the old line `old`; `None` when it was deleted or changed.
    pub fn line(&self, old: usize) -> Option<usize> {
        self.old_to_new.get(old).copied().flatten()
    }

    /// The old lines `old` in the new file.
    pub fn range(&self, old: Range<usize>) -> Mapped {
        let mapped: Vec<usize> = old.clone().filter_map(|line| self.line(line)).collect();
        let (Some(&first), Some(&last)) = (mapped.first(), mapped.last()) else {
            return Mapped::Gone;
        };
        let span = first..last + 1;
        if mapped.len() == old.len() && span.len() == old.len() {
            Mapped::Kept(span)
        } else {
            Mapped::Partly(span)
        }
    }

    /// Lines in the old file.
    pub fn old_len(&self) -> usize {
        self.old_to_new.len()
    }

    /// Lines in the new file.
    pub fn new_len(&self) -> usize {
        self.new_len
    }
}

/// Old line → new line between two hash vectors: a Myers diff (`O(ND)`) of what lies between
/// their common start and end, giving up past [`MAX_EDITS`], where every line in between
/// counts as replaced.
pub fn diff(old: &[u64], new: &[u64]) -> LineMap {
    let start = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let end = old[start..]
        .iter()
        .rev()
        .zip(new[start..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut old_to_new = vec![None; old.len()];
    for (line, place) in old_to_new.iter_mut().enumerate().take(start) {
        *place = Some(line);
    }
    let shift = new.len() as isize - old.len() as isize;
    let tail = old.len() - end;
    for (line, place) in old_to_new.iter_mut().enumerate().skip(tail) {
        *place = Some((line as isize + shift) as usize);
    }
    let (a, b) = (&old[start..old.len() - end], &new[start..new.len() - end]);
    for (x, y) in myers(a, b).unwrap_or_default() {
        old_to_new[start + x] = Some(start + y);
    }
    LineMap {
        old_to_new,
        new_len: new.len(),
    }
}

/// The matching lines `(a index, b index)` of a shortest edit script, or `None` past
/// [`MAX_EDITS`].
fn myers(a: &[u64], b: &[u64]) -> Option<Vec<(usize, usize)>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let limit = (a.len() + b.len()).min(MAX_EDITS) as isize;
    let off = limit + 1;
    let mut v = vec![0isize; (2 * limit + 3) as usize];
    // Before each step d, the diagonals -(d+1)..=d+1 it reads.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    for d in 0..=limit {
        trace.push(v[(off - d - 1) as usize..=(off + d + 1) as usize].to_vec());
        for k in (-d..=d).step_by(2) {
            let at = (k + off) as usize;
            let mut x = if k == -d || (k != d && v[at - 1] < v[at + 1]) {
                v[at + 1]
            } else {
                v[at - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[at] = x;
            if x >= n && y >= m {
                return Some(backtrack(&trace, n, m));
            }
        }
    }
    None
}

/// The matching lines of the path [`myers`] found, read back from its trace.
fn backtrack(trace: &[Vec<isize>], n: isize, m: isize) -> Vec<(usize, usize)> {
    let (mut x, mut y) = (n, m);
    let mut matches = Vec::new();
    for (d, v) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let at = |k: isize| v[(k + d + 1) as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = at(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            matches.push((x as usize, y as usize));
        }
        x = prev_x;
        y = prev_y;
    }
    matches.reverse();
    matches
}

/// A run of lines to find again: its own line hashes and up to a few lines of context on
/// each side, and where it started (0-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub start: usize,
    pub lines: Vec<u64>,
    pub before: Vec<u64>,
    pub after: Vec<u64>,
}

impl Anchor {
    /// The lines `range` of `file` (clamped to it) with up to `context` lines on each side.
    pub fn at(file: &[u64], range: Range<usize>, context: usize) -> Anchor {
        let end = range.end.min(file.len());
        let start = range.start.min(end);
        Anchor {
            start,
            lines: file[start..end].to_vec(),
            before: file[start.saturating_sub(context)..start].to_vec(),
            after: file[end..(end + context).min(file.len())].to_vec(),
        }
    }

    /// Too little to be found on its own: fewer than 3 lines, none of them significant (a
    /// blank line, a lone `}`).
    pub fn trivial(&self) -> bool {
        self.lines.len() < LONG_RUN && !self.lines.iter().any(|&h| significant(h))
    }
}

/// Where `anchor` is in `file`: a run equal to its lines, the one with the most matching
/// context lines (counted outward from the run), ties broken by the distance from `hint`, then
/// by the earlier place. `None`: the text is gone (outdated), or the anchor is
/// [trivial](Anchor::trivial).
pub fn relocate(anchor: &Anchor, file: &[u64], hint: usize) -> Option<Range<usize>> {
    let len = anchor.lines.len();
    if len == 0 || anchor.trivial() || len > file.len() {
        return None;
    }
    let score = |at: usize| {
        let before = anchor.before.iter().rev();
        let above = file[..at].iter().rev();
        let after = anchor.after.iter().zip(&file[at + len..]);
        let matching = |(a, b): (&u64, &u64)| a == b;
        before.zip(above).take_while(|p| matching(*p)).count()
            + after.take_while(|p| matching(*p)).count()
    };
    (0..=file.len() - len)
        .filter(|&at| file[at..at + len] == anchor.lines[..])
        .map(|at| (score(at), at))
        .min_by_key(|&(score, at)| (std::cmp::Reverse(score), at.abs_diff(hint), at))
        .map(|(_, at)| at..at + len)
}

/// Packs hashes as base64 (standard alphabet, padded) of their little-endian bytes, for a
/// compact JSON store.
pub fn pack(hashes: &[u64]) -> String {
    let bytes: Vec<u8> = hashes.iter().flat_map(|h| h.to_le_bytes()).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word = chunk
            .iter()
            .enumerate()
            .fold(0u32, |w, (i, &b)| w | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(B64[(word >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The hashes [`pack`] made; `None` for text it could not have made.
pub fn unpack(text: &str) -> Option<Vec<u64>> {
    let text = text.as_bytes();
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    for chunk in text.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut word = 0u32;
        for &c in &chunk[..4 - pad] {
            word = word << 6 | u32::from(B64.iter().position(|&b| b == c)? as u8);
        }
        word <<= 6 * pad as u32;
        bytes.extend_from_slice(&word.to_be_bytes()[1..4 - pad]);
    }
    if !bytes.len().is_multiple_of(8) {
        return None;
    }
    Some(
        bytes
            .chunks(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap_or_default()))
            .collect(),
    )
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[cfg(test)]
mod tests {
    use super::*;

    fn h(lines: &[&str]) -> Vec<u64> {
        lines.iter().map(|l| line_hash(l.as_bytes())).collect()
    }

    #[test]
    fn a_line_hashes_without_its_line_ending() {
        let plain = line_hash(b"let x = compute();");
        assert_eq!(line_hash(b"let x = compute();\n"), plain);
        assert_eq!(line_hash(b"let x = compute();\r\n"), plain);
        assert_eq!(line_hash(b"let x = compute();\r"), plain);
        assert_ne!(line_hash(b"let x = compute();;"), plain);
        assert_ne!(line_hash(b"let x = compute()\r;"), plain);
    }

    #[test]
    fn the_hash_is_fnv_1a_with_the_significance_bit() {
        // FNV-1a 64 of "a" is 0xaf63dc4c8601ec8c; "a" is not significant.
        assert_eq!(line_hash(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(line_hash(b""), FNV_OFFSET & !1);
        assert_eq!(fnv(FNV_OFFSET, b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn a_line_is_significant_from_eight_non_blank_characters() {
        assert!(!significant(line_hash(b"}")));
        assert!(!significant(line_hash(b"")));
        assert!(!significant(line_hash(b"  a b c d e f g  ")));
        assert!(significant(line_hash(b"  a b c d e f g h ")));
        assert!(significant(line_hash(b"abcdefgh")));
        assert!(!significant(line_hash(b"abcdefg\r")));
        assert!(!significant(0b10));
        assert!(significant(0b11));
    }

    #[test]
    fn text_splits_into_line_hashes() {
        assert_eq!(hashes(b"", 10), Some(vec![]));
        assert_eq!(hashes(b"\n", 10), Some(h(&[""])));
        assert_eq!(hashes(b"a\nb", 10), Some(h(&["a", "b"])));
        assert_eq!(hashes(b"a\nb\n", 10), Some(h(&["a", "b"])));
        assert_eq!(hashes(b"a\r\nb\r\n", 10), Some(h(&["a", "b"])));
        assert_eq!(hashes(b"a\n\nb\n\n", 10), Some(h(&["a", "", "b", ""])));
        assert_eq!(hashes(b"a\nb\nc\n", 3), Some(h(&["a", "b", "c"])));
        assert_eq!(hashes(b"a\nb\nc\n", 2), None);
        assert_eq!(hashes(b"a", 0), None);
    }

    #[test]
    fn the_digest_depends_on_every_hash_and_their_order() {
        assert_eq!(digest(&[]), FNV_OFFSET);
        assert_eq!(digest(&[1, 2]), digest(&[1, 2]));
        assert_ne!(digest(&[1, 2]), digest(&[2, 1]));
        assert_ne!(digest(&[1, 2]), digest(&[1, 3]));
        assert_ne!(digest(&[1]), digest(&[1, 0]));
        assert_eq!(digest(&[0]), fnv(FNV_OFFSET, &[0; 8]));
    }

    fn lines(map: &LineMap) -> Vec<Option<usize>> {
        (0..map.old_len()).map(|l| map.line(l)).collect()
    }

    #[test]
    fn the_diff_follows_inserts_deletes_and_replacements() {
        let old = h(&["a", "b", "c", "d"]);
        let same = diff(&old, &old);
        assert_eq!(lines(&same), [Some(0), Some(1), Some(2), Some(3)]);
        assert_eq!(same.new_len(), 4);
        let inserted = diff(&old, &h(&["a", "x", "y", "b", "c", "d"]));
        assert_eq!(lines(&inserted), [Some(0), Some(3), Some(4), Some(5)]);
        assert_eq!(inserted.new_len(), 6);
        let deleted = diff(&old, &h(&["a", "d"]));
        assert_eq!(lines(&deleted), [Some(0), None, None, Some(1)]);
        let replaced = diff(&old, &h(&["a", "B", "c", "d"]));
        assert_eq!(lines(&replaced), [Some(0), None, Some(2), Some(3)]);
        let at_start = diff(&old, &h(&["x", "a", "b", "c", "d"]));
        assert_eq!(lines(&at_start), [Some(1), Some(2), Some(3), Some(4)]);
        let emptied = diff(&old, &[]);
        assert_eq!(lines(&emptied), [None, None, None, None]);
        assert_eq!(lines(&diff(&[], &old)), []);
        assert_eq!(diff(&[], &old).new_len(), 4);
    }

    #[test]
    fn a_moved_line_keeps_the_rest_in_place() {
        let old = h(&["a", "b", "c", "d", "e"]);
        let moved = diff(&old, &h(&["b", "c", "d", "a", "e"]));
        assert_eq!(lines(&moved), [None, Some(0), Some(1), Some(2), Some(4)]);
        let both_ends = diff(&h(&["p", "a", "b", "q"]), &h(&["p", "b", "a", "q"]));
        assert_eq!(
            lines(&both_ends).iter().filter(|l| l.is_some()).count(),
            3,
            "{both_ends:?}"
        );
    }

    /// The longest common subsequence's length, by the textbook table.
    fn lcs(a: &[u64], b: &[u64]) -> usize {
        let mut t = vec![vec![0; b.len() + 1]; a.len() + 1];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                t[i][j] = if a[i] == b[j] {
                    t[i + 1][j + 1] + 1
                } else {
                    t[i + 1][j].max(t[i][j + 1])
                };
            }
        }
        t[0][0]
    }

    /// A small deterministic generator, so the property test needs no dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, below: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % below
        }
    }

    #[test]
    fn the_diff_keeps_a_longest_common_subsequence_in_order() {
        let mut rng = Rng(7);
        for _ in 0..400 {
            let a: Vec<u64> = (0..rng.next(14)).map(|_| rng.next(5)).collect();
            let b: Vec<u64> = (0..rng.next(14)).map(|_| rng.next(5)).collect();
            let map = diff(&a, &b);
            let pairs: Vec<(usize, usize)> = (0..a.len())
                .filter_map(|x| map.line(x).map(|y| (x, y)))
                .collect();
            assert_eq!(pairs.len(), lcs(&a, &b), "{a:?} {b:?} {pairs:?}");
            for &(x, y) in &pairs {
                assert_eq!(a[x], b[y]);
            }
            assert!(pairs.windows(2).all(|w| w[0].1 < w[1].1), "{pairs:?}");
            assert_eq!(map.new_len(), b.len());
        }
    }

    #[test]
    fn past_the_edit_cap_every_line_in_between_counts_as_replaced() {
        // Interleaved: every other line differs, so the edits double the length.
        let old: Vec<u64> = (0..MAX_EDITS as u64 + 2).map(|i| i * 2).collect();
        let mut new: Vec<u64> = old.iter().map(|&x| if x % 4 == 0 { x } else { x + 1 }).collect();
        new.insert(0, 999_999);
        new.push(999_998);
        let map = diff(&old, &new);
        assert!(lines(&map).iter().all(Option::is_none), "{map:?}");
        // At the cap it still finds them: half the lines changed, MAX_EDITS edits in all.
        let old: Vec<u64> = (0..MAX_EDITS as u64).collect();
        let new: Vec<u64> = old.iter().map(|&x| if x % 2 == 0 { x } else { x + 10_000 }).collect();
        let map = diff(&old, &new);
        assert_eq!(map.line(0), Some(0));
        assert_eq!(map.line(1), None);
        assert_eq!(map.line(MAX_EDITS - 2), Some(MAX_EDITS - 2));
        // The common start and end stay mapped past the cap.
        let mut old: Vec<u64> = (0..MAX_EDITS as u64 + 1).map(|x| x + 100).collect();
        let mut new: Vec<u64> = (0..MAX_EDITS as u64 + 1).map(|x| x + 50_000).collect();
        for side in [&mut old, &mut new] {
            side.insert(0, 1);
            side.push(2);
        }
        let map = diff(&old, &new);
        assert_eq!(map.line(0), Some(0));
        assert_eq!(map.line(1), None);
        assert_eq!(map.line(old.len() - 1), Some(new.len() - 1));
    }

    #[test]
    fn a_large_file_with_many_hunks_diffs_quickly() {
        let old: Vec<u64> = (0..50_000u64).collect();
        let mut new = old.clone();
        for hunk in 0..500 {
            new[hunk * 100] += 1_000_000;
        }
        let started = std::time::Instant::now();
        let map = diff(&old, &new);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(map.line(0), None);
        assert_eq!(map.line(1), Some(1));
        assert_eq!(map.line(49_999), Some(49_999));
    }

    #[test]
    fn a_range_is_kept_partly_kept_or_gone() {
        let old = h(&["a", "b", "c", "d", "e"]);
        let map = diff(&old, &h(&["x", "a", "b", "c", "d", "e"]));
        assert_eq!(map.range(1..3), Mapped::Kept(2..4));
        let map = diff(&old, &h(&["a", "c", "d", "e"]));
        assert_eq!(map.range(1..3), Mapped::Partly(1..2));
        assert_eq!(map.range(0..3), Mapped::Partly(0..2));
        let map = diff(&old, &h(&["a", "b", "y", "c", "d", "e"]));
        assert_eq!(map.range(1..3), Mapped::Partly(1..4));
        let map = diff(&old, &h(&["a", "e"]));
        assert_eq!(map.range(1..4), Mapped::Gone);
        assert_eq!(map.range(2..2), Mapped::Gone);
        assert_eq!(map.range(9..12), Mapped::Gone);
    }

    #[test]
    fn an_anchor_takes_its_lines_and_context() {
        let file = [1, 2, 3, 4, 5, 6];
        let anchor = Anchor::at(&file, 2..4, 1);
        assert_eq!(
            anchor,
            Anchor {
                start: 2,
                lines: vec![3, 4],
                before: vec![2],
                after: vec![5]
            }
        );
        let wide = Anchor::at(&file, 1..5, 3);
        assert_eq!((wide.before, wide.after), (vec![1], vec![6]));
        let clamped = Anchor::at(&file, 5..9, 2);
        assert_eq!((clamped.start, clamped.lines), (5, vec![6]));
        assert_eq!(clamped.after, Vec::<u64>::new());
        let past = Anchor::at(&file, 8..9, 2);
        assert_eq!((past.start, past.lines.len(), past.before), (6, 0, vec![5, 6]));
    }

    const S: u64 = 1; // significant
    const T: u64 = 2; // trivial

    #[test]
    fn short_runs_without_a_significant_line_are_trivial() {
        let anchor = |lines: Vec<u64>| Anchor {
            start: 0,
            lines,
            before: vec![],
            after: vec![],
        };
        assert!(anchor(vec![T]).trivial());
        assert!(anchor(vec![T, T + 2]).trivial());
        assert!(!anchor(vec![T, T + 2, T + 4]).trivial());
        assert!(!anchor(vec![S]).trivial());
        assert!(!anchor(vec![T, S + 2]).trivial());
    }

    #[test]
    fn relocate_finds_a_moved_run_by_context_then_nearest_the_hint() {
        let s = |n: u64| n * 4 + S;
        let file = [s(1), s(2), s(3), s(9), s(1), s(2), s(4), s(9)];
        let anchor = Anchor {
            start: 0,
            lines: vec![s(2)],
            before: vec![s(1)],
            after: vec![s(4)],
        };
        assert_eq!(relocate(&anchor, &file, 0), Some(5..6));
        let no_context = Anchor {
            before: vec![],
            after: vec![],
            ..anchor.clone()
        };
        assert_eq!(relocate(&no_context, &file, 0), Some(1..2));
        assert_eq!(relocate(&no_context, &file, 4), Some(5..6));
        assert_eq!(relocate(&no_context, &file, 3), Some(1..2));
        // Context is counted outward: a mismatch next to the run stops the count.
        let far = Anchor {
            start: 0,
            lines: vec![s(3)],
            before: vec![s(1), s(7)],
            after: vec![s(7), s(1)],
        };
        assert_eq!(relocate(&far, &file, 0), Some(2..3));
        let deep = Anchor {
            start: 0,
            lines: vec![s(9)],
            before: vec![s(1), s(2), s(4)],
            after: vec![],
        };
        assert_eq!(relocate(&deep, &file, 0), Some(7..8));
        let after = Anchor {
            start: 0,
            lines: vec![s(1)],
            before: vec![],
            after: vec![s(2), s(4)],
        };
        assert_eq!(relocate(&after, &file, 0), Some(4..5));
    }

    #[test]
    fn relocate_gives_up_on_gone_empty_or_trivial_text() {
        let file = [S, T, S + 4, T + 4];
        let anchor = |lines: Vec<u64>| Anchor {
            start: 0,
            lines,
            before: vec![S],
            after: vec![S + 4],
        };
        assert_eq!(relocate(&anchor(vec![S + 8]), &file, 0), None);
        assert_eq!(relocate(&anchor(vec![]), &file, 0), None);
        assert_eq!(relocate(&anchor(vec![T]), &file, 0), None);
        assert_eq!(relocate(&anchor(vec![S, T, S + 4, T + 4, S]), &file, 0), None);
        assert_eq!(relocate(&anchor(vec![S + 4, T + 4]), &file, 0), Some(2..4));
        assert_eq!(relocate(&anchor(vec![S, T, S + 4, T + 4]), &file, 9), Some(0..4));
    }

    #[test]
    fn hashes_pack_and_unpack() {
        for hashes in [vec![], vec![0], vec![u64::MAX, 1, 0x0123_4567_89ab_cdef]] {
            let packed = pack(&hashes);
            assert_eq!(packed.len(), (hashes.len() * 8).div_ceil(3) * 4);
            assert_eq!(unpack(&packed), Some(hashes));
        }
        assert_eq!(pack(&[0]), "AAAAAAAAAAA=");
        assert_eq!(pack(&[u64::MAX]), "//////////8=");
        assert_eq!(pack(&[1, 2]), "AQAAAAAAAAACAAAAAAAAAA==");
        assert_eq!(unpack("AQAAAAAAAAACAAAAAAAAAA=="), Some(vec![1, 2]));
    }

    #[test]
    fn unpack_refuses_what_pack_cannot_make() {
        assert_eq!(unpack("AAA"), None);
        assert_eq!(unpack("AAAA"), None);
        assert_eq!(unpack("AAAAAAAAAA*="), None);
        assert_eq!(unpack("AAAAAAAA===="), None);
        assert_eq!(unpack("AAAAAAAAAA=="), None);
    }
}
