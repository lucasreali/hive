//! Searching the text of a worktree's files (the sidebar's Files, "Contents"): `git grep` over
//! tracked files and untracked ones that are not ignored, as a fixed string, ignoring case.
//! Binary files are skipped.

use std::ffi::OsStr;
use std::io;
use std::path::Path;

use hive_protocol::SearchMatch;

use crate::git;

/// Longest query accepted, in bytes.
pub const QUERY_LIMIT: usize = 256;
/// Most matches sent; more make the answer `truncated`.
pub const MATCH_LIMIT: usize = 1000;
/// Most matching lines taken from one file.
const PER_FILE: &str = "--max-count=50";
/// Most characters of a matching line sent.
const TEXT_LIMIT: usize = 300;
/// Most bytes read from `git grep`.
const OUTPUT_LIMIT: u64 = 16_777_216; // 16 MiB

/// The lines of the worktree at `dir` holding `query`, in path order, and whether there were
/// more than [`MATCH_LIMIT`].
pub fn search(dir: &Path, query: &str) -> io::Result<(Vec<SearchMatch>, bool)> {
    if query.trim().is_empty() || query.len() > QUERY_LIMIT || query.contains(['\n', '\r']) {
        return Err(io::Error::other(format!(
            "search for 1 to {QUERY_LIMIT} bytes on one line"
        )));
    }
    let args = [
        "grep",
        "-n",
        "-I",
        "-i",
        "-F",
        "-z",
        "--untracked",
        "--no-color",
        "--full-name",
        PER_FILE,
        "-e",
        query,
    ]
    .map(OsStr::new);
    // Exit code 1: nothing matched.
    let out = git::run(dir, &args, &[], &[0, 1], OUTPUT_LIMIT)?;
    Ok(parse(&out))
}

/// Parses `git grep -n -z`: `path\0line\0text\n` per match (a path may hold any byte but NUL).
pub fn parse(out: &[u8]) -> (Vec<SearchMatch>, bool) {
    let mut matches = Vec::new();
    let mut rest = out;
    while !rest.is_empty() {
        let Some((path, after)) = split_once(rest, 0) else {
            break;
        };
        let Some((line, after)) = split_once(after, 0) else {
            break;
        };
        let (text, after) = split_once(after, b'\n').unwrap_or((after, &[]));
        rest = after;
        let Some(line) = std::str::from_utf8(line).ok().and_then(|l| l.parse().ok()) else {
            continue;
        };
        if matches.len() == MATCH_LIMIT {
            return (matches, true);
        }
        matches.push(SearchMatch {
            path: String::from_utf8_lossy(path).into_owned(),
            line,
            text: String::from_utf8_lossy(text)
                .chars()
                .take(TEXT_LIMIT)
                .collect(),
        });
    }
    (matches, false)
}

fn split_once(bytes: &[u8], at: u8) -> Option<(&[u8], &[u8])> {
    let i = bytes.iter().position(|&b| b == at)?;
    Some((&bytes[..i], &bytes[i + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(path: &str, line: u64, text: &str) -> SearchMatch {
        SearchMatch {
            path: path.into(),
            line,
            text: text.into(),
        }
    }

    #[test]
    fn matches_are_parsed() {
        let out = b"a.txt\x001\x00Hello world\nd/b c.txt\x0012\x00x hello\nodd\npath\x003\x00last";
        assert_eq!(
            parse(out),
            (
                vec![
                    found("a.txt", 1, "Hello world"),
                    found("d/b c.txt", 12, "x hello"),
                    found("odd\npath", 3, "last"),
                ],
                false
            )
        );
        // A bad line number is skipped; a cut record ends the list.
        assert_eq!(
            parse(b"a\x00x\x00t\nb\x002\x00u\n"),
            (vec![found("b", 2, "u")], false)
        );
        assert_eq!(parse(b"a\x001"), (vec![], false));
        assert_eq!(parse(b"a"), (vec![], false));
        assert_eq!(parse(b""), (vec![], false));
    }

    #[test]
    fn long_lines_and_many_matches_are_cut() {
        let long = format!("a\x001\x00{}\n", "é".repeat(TEXT_LIMIT + 5));
        let (matches, _) = parse(long.as_bytes());
        assert_eq!(matches[0].text.chars().count(), TEXT_LIMIT);
        let many = "a\x001\x00t\n".repeat(MATCH_LIMIT);
        assert_eq!(
            parse(many.as_bytes()),
            (vec![found("a", 1, "t"); MATCH_LIMIT], false)
        );
        let more = "a\x001\x00t\n".repeat(MATCH_LIMIT + 1);
        let (matches, truncated) = parse(more.as_bytes());
        assert_eq!((matches.len(), truncated), (MATCH_LIMIT, true));
    }

    #[test]
    fn bad_queries_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        for query in ["", "  ", "a\nb", "a\rb", &"x".repeat(QUERY_LIMIT + 1)] {
            let err = search(dir.path(), query).unwrap_err().to_string();
            assert_eq!(err, "search for 1 to 256 bytes on one line", "{query:?}");
        }
    }
}
