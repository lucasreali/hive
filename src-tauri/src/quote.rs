//! Paths typed into a terminal as words its shell reads back unchanged: the files dropped from
//! Finder on a terminal pane (18.1) and, later, the files pasted from Finder (18.3).
//!
//! The same text must work in bash, zsh and fish, and in Claude Code's prompt, which turns a
//! pasted image path into an attachment. As Terminal.app and iTerm2 do, a path is written with
//! a backslash before each character a shell treats specially (`/My\ Shots/a\ \(1\).png`);
//! all three shells read `\` + any ASCII character as that character. A backslash before a
//! line break would join the lines instead, so a path with a control character (a newline, a
//! tab, an escape) is written between single quotes, which keep them as they are in all three;
//! inside them `'` is written `'\''` and `\` as `'\\'` (fish reads `\\` and `\'` inside single
//! quotes as escapes, bash and zsh do not, so both are written outside them). Non-ASCII
//! characters (accents, CJK, the narrow no-break space in macOS screenshot names) are no
//! shell's syntax and stay as they are.

/// Whether `c` reads as itself, unquoted, anywhere in a word of bash, zsh and fish.
fn plain(c: char) -> bool {
    !c.is_ascii() || c.is_ascii_alphanumeric() || "/._-+,:@%".contains(c)
}

/// `path` as one shell word.
pub fn word(path: &str) -> String {
    if path.is_empty() {
        return "''".to_owned();
    }
    let mut out = String::with_capacity(path.len() + 2);
    if path.chars().any(char::is_control) {
        out.push('\'');
        for c in path.chars() {
            match c {
                '\'' => out.push_str(r"'\''"),
                '\\' => out.push_str(r"'\\'"),
                c => out.push(c),
            }
        }
        out.push('\'');
    } else {
        for c in path.chars() {
            if !plain(c) {
                out.push('\\');
            }
            out.push(c);
        }
    }
    out
}

/// What dropping (or pasting) the files at `paths` types: each one as a word followed by a
/// space, as Terminal.app does, so the next word (or the next drop) stays apart. Never a line
/// break outside quotes: it types the paths and never presses Enter.
pub fn typed(paths: &[impl AsRef<str>]) -> String {
    paths.iter().map(|path| word(path.as_ref()) + " ").collect()
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

    #[test]
    fn control_characters_go_between_single_quotes() {
        assert_eq!(word("/tmp/a\nb"), "'/tmp/a\nb'");
        assert_eq!(word("/tmp/tab\there"), "'/tmp/tab\there'");
        assert_eq!(word("/tmp/it's\n"), "'/tmp/it'\\''s\n'");
        assert_eq!(word("/tmp/back\\slash\r"), "'/tmp/back'\\\\'slash\r'");
        assert_eq!(word("/tmp/$x \u{1b}"), "'/tmp/$x \u{1b}'");
        assert_eq!(word("/tmp/\u{85}"), "'/tmp/\u{85}'", "a C1 control too");
    }

    #[test]
    fn an_empty_path_is_an_empty_word() {
        assert_eq!(word(""), "''");
    }

    #[test]
    fn several_paths_are_words_each_followed_by_a_space() {
        assert_eq!(typed(&["/a b", "/c"]), r"/a\ b /c ");
        assert_eq!(typed(&["/a".to_owned()]), "/a ");
        assert_eq!(typed(&[] as &[&str]), "");
    }

    /// Every case above, read back by a POSIX shell (`/bin/sh`), comes out as the path.
    #[cfg(unix)]
    #[test]
    fn a_shell_reads_each_word_back_as_its_path() {
        let paths = [
            "/Users/me/My Shots/a (1).png",
            "/a'b\"c",
            "/$HOME/`x`",
            "!#&*;<>?[]^{}|~=\\",
            "/tmp/日本語/café\u{202f}x",
            "/tmp/a\nb",
            "/tmp/tab\there",
            "/tmp/it's\n",
            "/tmp/back\\slash\r",
            "/tmp/$x \u{1b}",
            "",
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
