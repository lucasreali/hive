//! Paths typed into a terminal as words its shell reads back unchanged: the files dropped from
//! Finder on a terminal pane (18.1) and, later, the files pasted from Finder (18.3).
//!
//! The same text must work in bash, zsh and fish, and in Claude Code's prompt, which turns a
//! pasted image path into an attachment. As Terminal.app and iTerm2 do, a path is written with
//! a backslash before each character a shell treats specially (`/My\ Shots/a\ \(1\).png`);
//! all three shells read `\` + any ASCII character as that character. Non-ASCII characters
//! (accents, CJK, the narrow no-break space in macOS screenshot names) are no shell's syntax
//! and stay as they are.
//!
//! A control character (C0, DEL, C1: `char::is_control`) is never typed, quoted or not: the
//! text reaches the program's line editor as keystrokes before any shell parses it, so an
//! escape could end a bracketed paste and a carriage return or a newline press Enter (a file
//! named `a\x1b[201~\x15touch x\n` would run `touch x`). A path holding one is left out, as is
//! a path that is not absolute, and a text longer than [`TYPED_LIMIT`] is not typed at all.

/// The longest text a drop or a paste types; a longer one types nothing.
pub const TYPED_LIMIT: usize = 64 * 1024;

/// Whether `c` reads as itself, unquoted, anywhere in a word of bash, zsh and fish.
fn plain(c: char) -> bool {
    !c.is_ascii() || c.is_ascii_alphanumeric() || "/._-+,:@%".contains(c)
}

/// Whether `path` may be typed: absolute, without any control character.
pub fn usable(path: &str) -> bool {
    path.starts_with('/') && !path.chars().any(char::is_control)
}

/// `path` as one shell word; empty (no word) when it holds a control character, which is
/// never typed.
pub fn word(path: &str) -> String {
    if path.chars().any(char::is_control) {
        return String::new();
    }
    if path.is_empty() {
        return "''".to_owned();
    }
    let mut out = String::with_capacity(path.len() + 2);
    for c in path.chars() {
        if !plain(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// What dropping (or pasting) the files at `paths` types: each [`usable`] one as a word
/// followed by a space, as Terminal.app does, so the next word (or the next drop) stays apart.
/// The others are left out; nothing when none is left or the text would pass [`TYPED_LIMIT`].
/// Never a control character: it types the paths and never presses Enter.
pub fn typed(paths: &[impl AsRef<str>]) -> String {
    let text: String = paths
        .iter()
        .map(AsRef::as_ref)
        .filter(|path| usable(path))
        .map(|path| word(path) + " ")
        .collect();
    if text.len() > TYPED_LIMIT {
        return String::new();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_stay_as_they_are() {
        assert_eq!(word("/Users/me/a.png"), "/Users/me/a.png");
        assert_eq!(
            word("/tmp/v1.2_final-copy+x,y:z@host%20"),
            "/tmp/v1.2_final-copy+x,y:z@host%20"
        );
    }

    #[test]
    fn spaces_and_shell_characters_get_a_backslash() {
        assert_eq!(
            word("/Users/me/My Shots/a (1).png"),
            r"/Users/me/My\ Shots/a\ \(1\).png"
        );
        assert_eq!(word("/a'b\"c"), r#"/a\'b\"c"#);
        assert_eq!(word("/$HOME/`x`"), r"/\$HOME/\`x\`");
        assert_eq!(
            word("!#&*;<>?[]^{}|~=\\"),
            r"\!\#\&\*\;\<\>\?\[\]\^\{\}\|\~\=\\"
        );
    }

    #[test]
    fn non_ascii_names_stay_as_they_are() {
        // A macOS screenshot's name has a narrow no-break space before "PM".
        let shot = "/Users/me/Desktop/Captura de Tela 2026-10-20 às 10.00.00\u{202f}PM.png";
        assert_eq!(
            word(shot),
            "/Users/me/Desktop/Captura\\ de\\ Tela\\ 2026-10-20\\ às\\ 10.00.00\u{202f}PM.png"
        );
        assert_eq!(word("/tmp/日本語/café"), "/tmp/日本語/café");
    }

    /// Names that would act as keys in a line editor: end a bracketed paste, erase the line,
    /// press Enter, start a CSI (C1 `\x9b`), delete.
    const HOSTILE: [&str; 9] = [
        "/tmp/a\x1b[201~\x15touch ~/PWNED\n",
        "/tmp/a\nb",
        "/tmp/a\rb",
        "/tmp/a\x15b",
        "/tmp/tab\there",
        "/tmp/\u{9b}201~",
        "/tmp/\u{85}",
        "/tmp/del\x7f",
        "/tmp/nul\0",
    ];

    #[test]
    fn a_path_with_a_control_character_has_no_word_and_is_left_out() {
        for path in HOSTILE {
            assert_eq!(word(path), "", "{path:?}");
            assert!(!usable(path), "{path:?}");
            assert_eq!(typed(&[path]), "", "{path:?}");
        }
        let mut paths = vec!["/a b"];
        paths.extend(HOSTILE);
        paths.push("/c");
        let text = typed(&paths);
        assert_eq!(text, r"/a\ b /c ");
        assert!(!text.chars().any(char::is_control));
    }

    #[test]
    fn only_absolute_paths_are_typed() {
        assert_eq!(word(""), "''");
        assert!(usable("/"));
        for path in ["", "a.png", "~/a.png", "./a", "-rf", " /a"] {
            assert!(!usable(path), "{path:?}");
        }
        assert_eq!(typed(&["", "a.png", "/b", "-rf"]), "/b ");
    }

    #[test]
    fn several_paths_are_words_each_followed_by_a_space() {
        assert_eq!(typed(&["/a b", "/c"]), r"/a\ b /c ");
        assert_eq!(typed(&["/a".to_owned()]), "/a ");
        assert_eq!(typed(&[] as &[&str]), "");
    }

    #[test]
    fn a_text_past_the_limit_types_nothing() {
        // Each word is "/" + 1022 × "x" plus its space: 1024 bytes.
        let path = format!("/{}", "x".repeat(1022));
        let fits = vec![path.as_str(); TYPED_LIMIT / 1024];
        assert_eq!(typed(&fits).len(), TYPED_LIMIT);
        let mut over = fits.clone();
        over.push("/");
        assert_eq!(typed(&over), "");
    }

    /// Every typed word, read back by a POSIX shell (`/bin/sh`), comes out as its path.
    #[cfg(unix)]
    #[test]
    fn a_shell_reads_each_word_back_as_its_path() {
        let paths = [
            "/Users/me/My Shots/a (1).png",
            "/a'b\"c",
            "/$HOME/`x`",
            "/!#&*;<>?[]^{}|~=\\",
            "/tmp/日本語/café\u{202f}x",
        ];
        let script = format!("printf '%s\\0' {}", typed(&paths));
        let home = std::env::temp_dir();
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", &script])
            .env_clear()
            .env("HOME", &home)
            .env("LC_ALL", "C.UTF-8")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let words: Vec<&[u8]> = out.stdout.split(|b| *b == 0).collect();
        let expected: Vec<&[u8]> = paths.iter().map(|path| path.as_bytes()).collect();
        assert_eq!(words[..words.len() - 1], expected[..]);
    }
}
