//! The keymap (28.7, `docs/spike/keymap.md`): the settings' `keymap` section maps a command id
//! of the app's palette (6.3) to its shortcuts, holding only the user's changes; a command not
//! in it keeps #35's default, which the app knows. The service owns the grammar: it parses
//! every shortcut, saves the valid ones in one form ([`normalize`]) and lists every unknown
//! command, bad shortcut or fixed key as a problem ([`check`]). Nothing here ever fails: a bad
//! entry is ignored, and kept in the file as written.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which platform's keys a shortcut is written for: `Cmd` exists only on macOS, and the fixed
/// keys differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
}

/// The platform this build serves: macOS on macOS, Windows elsewhere (a Linux or WSL service
/// serves a Windows app; a remote's settings never reach the UI, 26.18).
pub const CURRENT: Platform = if cfg!(target_os = "macos") {
    Platform::Mac
} else {
    Platform::Windows
};

/// Every command id the keymap may name, sorted: the app's `COMMANDS` (`src/keymap.ts`), which
/// a bun test checks equal.
pub const COMMAND_IDS: &[&str] = &[
    "add-project",
    "close-tab",
    "comment",
    "find-in-terminal",
    "font-bigger",
    "font-reset",
    "font-smaller",
    "keyboard-settings",
    "new-terminal",
    "new-worktree",
    "next-pending",
    "next-tab",
    "palette",
    "previous-tab",
    "send-reference",
    "settings",
    "split-terminal",
    "tab-1",
    "tab-2",
    "tab-3",
    "tab-4",
    "tab-5",
    "tab-6",
    "tab-7",
    "tab-8",
    "tab-9",
    "toggle-markdown",
    "toggle-panel",
    "worktree-picker",
];

/// The longest shortcut, and the longest piece of user text quoted in a problem, in bytes.
pub const CHORD_LIMIT: usize = 64;
/// The most shortcuts of one command.
pub const CHORDS_LIMIT: usize = 8;
/// The most entries read.
pub const ENTRY_LIMIT: usize = 128;
/// The most problem lines; past it, one line counts the rest.
pub const PROBLEM_LIMIT: usize = 50;

/// A key the keymap never takes, with what it is (shown in the problem) and the one command
/// whose default it is, which may name it (Ctrl+Shift+V shows a Markdown file rendered, 11.2,
/// and pastes in the terminal: their contexts never meet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixed {
    pub platform: Platform,
    pub chord: &'static str,
    pub what: &'static str,
    pub except: Option<&'static str>,
}

const fn fixed(
    platform: Platform,
    chord: &'static str,
    what: &'static str,
    except: Option<&'static str>,
) -> Fixed {
    Fixed {
        platform,
        chord,
        what,
        except,
    }
}

use Platform::{Mac, Windows};

/// The fixed keys (spike section 6), in [`Chord`]'s form: the terminal's clipboard keys, the
/// macOS terminal's line keys, and the macOS menu's native items. The app's recorder
/// (`src/keymap.ts`) holds the same list, which a bun test checks against this one.
pub const FIXED: &[Fixed] = &[
    fixed(Mac, "Cmd+C", "the terminal's Copy", None),
    fixed(Mac, "Cmd+V", "the terminal's Paste", None),
    fixed(Mac, "Cmd+K", "the terminal's Clear", None),
    fixed(Mac, "Cmd+Up", "the terminal's previous prompt", None),
    fixed(Mac, "Cmd+Down", "the terminal's next prompt", None),
    fixed(Mac, "Cmd+Left", "the terminal's start of the line", None),
    fixed(Mac, "Cmd+Right", "the terminal's end of the line", None),
    fixed(Mac, "Cmd+Backspace", "the terminal's Delete line", None),
    fixed(Mac, "Cmd+Q", "the menu's Quit", None),
    fixed(Mac, "Cmd+H", "the menu's Hide", None),
    fixed(Mac, "Alt+Cmd+H", "the menu's Hide Others", None),
    fixed(Mac, "Cmd+Z", "the menu's Undo", None),
    fixed(Mac, "Shift+Cmd+Z", "the menu's Redo", None),
    fixed(Mac, "Cmd+X", "the menu's Cut", None),
    fixed(Mac, "Cmd+A", "the menu's Select All", None),
    fixed(Mac, "Ctrl+Cmd+F", "the menu's Full Screen", None),
    fixed(Mac, "Cmd+M", "the menu's Minimize", None),
    fixed(Windows, "Ctrl+Shift+C", "the terminal's Copy", None),
    fixed(
        Windows,
        "Ctrl+Shift+V",
        "the terminal's Paste",
        Some("toggle-markdown"),
    ),
];

/// The keys a chord may end with, besides `A`–`Z`, `0`–`9` and `F1`–`F24`: the unshifted
/// punctuation of a US layout and the named keys, each as it is written.
const KEYS: &[&str] = &[
    "`",
    "-",
    "=",
    "[",
    "]",
    "\\",
    ";",
    "'",
    ",",
    ".",
    "/",
    "Tab",
    "Enter",
    "Esc",
    "Space",
    "Backspace",
    "Delete",
    "Insert",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Up",
    "Down",
    "Left",
    "Right",
];

/// Each character Shift types on a US layout, with the key that types it.
const SHIFTED: &[(char, char)] = &[
    ('~', '`'),
    ('!', '1'),
    ('@', '2'),
    ('#', '3'),
    ('$', '4'),
    ('%', '5'),
    ('^', '6'),
    ('&', '7'),
    ('*', '8'),
    ('(', '9'),
    (')', '0'),
    ('_', '-'),
    ('{', '['),
    ('}', ']'),
    ('|', '\\'),
    (':', ';'),
    ('"', '\''),
    ('<', ','),
    ('>', '.'),
    ('?', '/'),
];

/// One shortcut: its modifiers and one key, as it is written ([`Chord::parse`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
    pub key: String,
}

/// `text` cut to [`CHORD_LIMIT`] bytes (at a character's start) and quoted, its control
/// characters escaped.
pub fn quote(text: &str) -> String {
    let mut end = text.len().min(CHORD_LIMIT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{:?}", &text[..end])
}

/// The key `part` names, as it is written; `None` when it names none.
fn key(part: &str) -> Option<String> {
    let mut chars = part.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next())
        && c.is_ascii_alphanumeric()
    {
        return Some(c.to_ascii_uppercase().to_string());
    }
    let function = (part.strip_prefix(['F', 'f']))
        .and_then(|n| n.parse::<u8>().ok().filter(|n| part[1..] == n.to_string()))
        .filter(|n| (1..=24).contains(n));
    if let Some(n) = function {
        return Some(format!("F{n}"));
    }
    let named = KEYS.iter().find(|k| k.eq_ignore_ascii_case(part));
    named.map(|k| (*k).to_owned())
}

impl Chord {
    /// Parses `text` for `platform`, normalised; the problem with it, as a sentence, when it is
    /// not a shortcut the keymap may hold.
    pub fn parse(text: &str, platform: Platform) -> Result<Chord, String> {
        let quoted = quote(text);
        if text.len() > CHORD_LIMIT {
            return Err(format!("{quoted} is longer than {CHORD_LIMIT} bytes"));
        }
        let bad = |why: String| format!("{quoted} is not a shortcut: {why}");
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        if parts.iter().any(|p| p.is_empty()) {
            return Err(bad("a key is missing after +".to_owned()));
        }
        let mut chord = Chord {
            ctrl: false,
            alt: false,
            shift: false,
            cmd: false,
            key: String::new(),
        };
        let count = parts.len();
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == count;
            let (flag, name) = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => (&mut chord.ctrl, "Ctrl"),
                "alt" | "option" | "opt" => (&mut chord.alt, "Alt"),
                "shift" => (&mut chord.shift, "Shift"),
                "cmd" | "command" => (&mut chord.cmd, "Cmd"),
                _ if last => break,
                _ => return Err(bad(format!("{} is not a modifier", quote(part)))),
            };
            if last {
                return Err(bad(format!("a key is missing after {name}")));
            }
            if *flag {
                return Err(bad(format!("{name} twice")));
            }
            *flag = true;
        }
        let last = parts.last().copied().unwrap_or_default();
        let shifted = SHIFTED.iter().find(|(c, _)| last.starts_with(*c));
        chord.key = match (key(last), shifted) {
            (Some(key), _) => key,
            (None, Some((_, unshifted))) if last.len() == 1 => {
                return Err(bad(format!("write it as Shift+{unshifted}")));
            }
            (None, _) => return Err(bad(format!("{} is not a key", quote(last)))),
        };
        if chord.cmd && platform == Windows {
            return Err(format!("{quoted}: Cmd exists only on macOS"));
        }
        let function = chord.key.len() > 1 && chord.key.starts_with('F');
        if !(chord.ctrl || chord.alt || chord.cmd || function) {
            let add = match platform {
                Mac => "Ctrl, Alt or Cmd",
                Windows => "Ctrl or Alt",
            };
            return Err(format!(
                "{quoted} would take the key from the terminal: add {add}"
            ));
        }
        Ok(chord)
    }

    /// The key that is fixed for `id` on `platform`, if this chord is one.
    pub fn fixed(&self, platform: Platform, id: &str) -> Option<&'static Fixed> {
        let text = self.to_string();
        FIXED
            .iter()
            .find(|f| f.platform == platform && f.chord == text && f.except != Some(id))
    }

    /// Tauri's accelerator for the chord (the macOS menu, 17.13): `Shift+Cmd+B` is
    /// `Shift+Super+B`.
    pub fn accelerator(&self) -> String {
        self.text("Super+")
    }

    /// The chord written with `cmd` for the Cmd key.
    fn text(&self, cmd: &str) -> String {
        let names = [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
            (self.cmd, cmd),
        ];
        let modifiers = names.iter().filter(|(on, _)| *on).map(|(_, name)| *name);
        modifiers.chain([self.key.as_str()]).collect()
    }
}

/// Modifiers in macOS' order (⌃⌥⇧⌘), then the key: `Ctrl+Alt+Shift+Cmd+P`.
impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text("Cmd+"))
    }
}

/// The settings' `keymap`: a command id mapped to one shortcut, a list of them, or `[]`
/// (`null`) for none. Kept as any JSON value, so a wrong type in it never makes the settings
/// file invalid: [`check`] lists it, and it stays in the file as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Keymap(pub Value);

impl Default for Keymap {
    fn default() -> Self {
        Self(Value::Object(serde_json::Map::new()))
    }
}

/// An entry's shortcuts as written (at most [`CHORDS_LIMIT`]), and whether there were more;
/// `None` when the value is not a text, a list of texts or null.
fn texts(value: &Value) -> Option<(Vec<&str>, bool)> {
    let all: Vec<&str> = match value {
        Value::Null => Vec::new(),
        Value::String(text) => vec![text],
        Value::Array(items) => items.iter().map(Value::as_str).collect::<Option<_>>()?,
        _ => return None,
    };
    let more = all.len() > CHORDS_LIMIT;
    Some((all.into_iter().take(CHORDS_LIMIT).collect(), more))
}

/// Parses an entry's chord `text` for the command `id`, a fixed key refused.
fn entry_chord(text: &str, id: &str, platform: Platform) -> Result<Chord, String> {
    let chord = Chord::parse(text, platform)?;
    match chord.fixed(platform, id) {
        Some(fixed) => Err(format!("{} is {} and stays fixed", quote(text), fixed.what)),
        None => Ok(chord),
    }
}

/// What a value of the wrong type is, for its problem.
fn kind(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "true or false",
        Value::Number(_) => "a number",
        Value::Object(_) => "an object",
        Value::String(_) => "a text",
        Value::Null => "null",
        Value::Array(_) => "a list holding something other than texts",
    }
}

impl Keymap {
    /// The entries read, in the file's order, at most [`ENTRY_LIMIT`]; none when the keymap is
    /// not an object.
    fn entries(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.0.as_object().into_iter().flatten().take(ENTRY_LIMIT)
    }

    /// The valid shortcuts of the command `id` (the bad ones ignored), or `None` when the
    /// keymap has no entry for it, or one of the wrong type: the command keeps its default.
    pub fn chords(&self, id: &str, platform: Platform) -> Option<Vec<Chord>> {
        let (_, value) = self.entries().find(|(key, _)| *key == id)?;
        let (texts, _) = texts(value)?;
        let chords = texts.iter().map(|t| entry_chord(t, id, platform));
        Some(chords.filter_map(Result::ok).collect())
    }
}

/// Every problem of `keymap` on `platform`, one line each (at most [`PROBLEM_LIMIT`], then one
/// counting the rest); empty when it has none. Never fails.
pub fn check(keymap: &Keymap, platform: Platform) -> Vec<String> {
    let mut problems = Vec::new();
    match keymap.0.as_object() {
        None => problems.push(format!(
            "keymap: it is {} where command ids and their shortcuts were expected",
            kind(&keymap.0)
        )),
        Some(map) if map.len() > ENTRY_LIMIT => problems.push(format!(
            "keymap: more than {ENTRY_LIMIT} entries, the rest are ignored"
        )),
        Some(_) => {}
    }
    for (id, value) in keymap.entries() {
        if !COMMAND_IDS.contains(&id.as_str()) {
            problems.push(format!("keymap: Hive has no command {}", quote(id)));
            continue;
        }
        let Some((texts, more)) = texts(value) else {
            problems.push(format!(
                "keymap.{id}: a shortcut is a text like \"Ctrl+Alt+M\" or a list of them (got {})",
                kind(value)
            ));
            continue;
        };
        if more {
            problems.push(format!(
                "keymap.{id}: more than {CHORDS_LIMIT} shortcuts, the rest are ignored"
            ));
        }
        for text in texts {
            if let Err(problem) = entry_chord(text, id, platform) {
                problems.push(format!("keymap.{id}: {problem}"));
            }
        }
    }
    if problems.len() > PROBLEM_LIMIT {
        let more = problems.len() - PROBLEM_LIMIT;
        problems.truncate(PROBLEM_LIMIT);
        problems.push(format!("… and {more} more problems"));
    }
    problems
}

/// Saves each valid entry in one form: its shortcuts normalised ([`Chord`]'s `Display`), each
/// once, one as a text, none or several as a list. An entry with any problem stays as written,
/// so a save never erases a mistake the user has not seen.
pub fn normalize(keymap: &mut Keymap, platform: Platform) {
    let Some(map) = keymap.0.as_object_mut() else {
        return;
    };
    for (id, value) in map.iter_mut().take(ENTRY_LIMIT) {
        if !COMMAND_IDS.contains(&id.as_str()) {
            continue;
        }
        let Some((texts, false)) = texts(value) else {
            continue;
        };
        let chords: Result<Vec<Chord>, String> =
            texts.iter().map(|t| entry_chord(t, id, platform)).collect();
        let Ok(chords) = chords else {
            continue;
        };
        let mut written: Vec<String> = Vec::new();
        for chord in chords.iter().map(Chord::to_string) {
            if !written.contains(&chord) {
                written.push(chord);
            }
        }
        *value = match <[String; 1]>::try_from(written) {
            Ok([one]) => Value::String(one),
            Err(several) => Value::from(several),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(text: &str, platform: Platform) -> Result<String, String> {
        Chord::parse(text, platform).map(|c| c.to_string())
    }

    /// The samples the app's `parseChord` runs too (`src/keymap.test.ts`).
    #[test]
    fn the_shared_samples_parse_alike() {
        let samples: Vec<Value> =
            serde_json::from_str(include_str!("../tests/chords.json")).unwrap();
        assert!(samples.len() > 20);
        for sample in samples {
            let text = sample["text"].as_str().unwrap();
            for (platform, name) in [(Mac, "mac"), (Windows, "windows")] {
                let want = sample[name].as_str().map(str::to_owned);
                let id = sample["id"].as_str().unwrap_or("palette");
                let got = entry_chord(text, id, platform).map(|c| c.to_string());
                assert_eq!(got.ok(), want, "{text} on {name}");
            }
        }
    }

    #[test]
    fn modifiers_take_their_aliases_any_case_and_come_out_in_macos_order() {
        let mac = |t| parse(t, Mac);
        assert_eq!(mac("cmd+shift+p").unwrap(), "Shift+Cmd+P");
        assert_eq!(
            mac("Command + Option + Control + Shift + p").unwrap(),
            "Ctrl+Alt+Shift+Cmd+P"
        );
        assert_eq!(mac("opt+x").unwrap(), "Alt+X");
        assert_eq!(mac("alt+x").unwrap(), "Alt+X");
        assert_eq!(mac("CTRL+tab").unwrap(), "Ctrl+Tab");
        assert_eq!(mac("ctrl+pageup").unwrap(), "Ctrl+PageUp");
        assert_eq!(mac("Cmd+7").unwrap(), "Cmd+7");
        assert_eq!(mac("Cmd+,").unwrap(), "Cmd+,");
        assert_eq!(mac("Cmd+\\").unwrap(), "Cmd+\\");
        assert_eq!(parse("ctrl+shift+p", Windows).unwrap(), "Ctrl+Shift+P");
    }

    #[test]
    fn every_named_key_and_function_key_is_a_key() {
        for name in KEYS {
            let text = format!("ctrl+{}", name.to_ascii_lowercase());
            assert_eq!(parse(&text, Windows).unwrap(), format!("Ctrl+{name}"));
        }
        for n in 1..=24 {
            assert_eq!(parse(&format!("f{n}"), Windows).unwrap(), format!("F{n}"));
            assert_eq!(
                parse(&format!("Shift+F{n}"), Mac).unwrap(),
                format!("Shift+F{n}")
            );
        }
        for refused in ["F0", "F25", "F01", "F", "Fx", "Ctrl+F1F"] {
            let Err(problem) = parse(refused, Windows) else {
                panic!("{refused}")
            };
            assert!(problem.contains("not a key") || problem.contains("from the terminal"));
        }
        // The letter F alone is a letter, needing a modifier.
        assert_eq!(parse("Ctrl+f", Windows).unwrap(), "Ctrl+F");
        assert_eq!(key("F"), Some("F".to_owned()));
        assert_eq!(key("F9"), Some("F9".to_owned()));
        assert_eq!(key("F10"), Some("F10".to_owned()));
        assert_eq!(key("é"), None);
        assert_eq!(key("ab"), None);
    }

    #[test]
    fn bad_chords_say_what_is_wrong() {
        let problem = |t| parse(t, Mac).unwrap_err();
        assert_eq!(
            problem("Cmd+Shift+"),
            r#""Cmd+Shift+" is not a shortcut: a key is missing after +"#
        );
        assert_eq!(
            problem(""),
            r#""" is not a shortcut: a key is missing after +"#
        );
        assert_eq!(
            problem("Cmd++P"),
            r#""Cmd++P" is not a shortcut: a key is missing after +"#
        );
        assert_eq!(
            problem("Hyper+P"),
            r#""Hyper+P" is not a shortcut: "Hyper" is not a modifier"#
        );
        assert_eq!(
            problem("P+P"),
            r#""P+P" is not a shortcut: "P" is not a modifier"#
        );
        assert_eq!(
            problem("Cmd+Cmd"),
            r#""Cmd+Cmd" is not a shortcut: a key is missing after Cmd"#
        );
        assert_eq!(
            problem("P+Cmd"),
            r#""P+Cmd" is not a shortcut: "P" is not a modifier"#
        );
        assert_eq!(
            problem("Cmd+Cmd+P"),
            r#""Cmd+Cmd+P" is not a shortcut: Cmd twice"#
        );
        assert_eq!(
            problem("Ctrl+control+P"),
            r#""Ctrl+control+P" is not a shortcut: Ctrl twice"#
        );
        assert_eq!(
            problem("Alt+opt+P"),
            r#""Alt+opt+P" is not a shortcut: Alt twice"#
        );
        assert_eq!(
            problem("Shift+shift+P"),
            r#""Shift+shift+P" is not a shortcut: Shift twice"#
        );
        assert_eq!(
            problem("Cmd+Shift"),
            r#""Cmd+Shift" is not a shortcut: a key is missing after Shift"#
        );
        assert_eq!(
            problem("Cmd"),
            r#""Cmd" is not a shortcut: a key is missing after Cmd"#
        );
        assert_eq!(
            problem("Shift+Ctrl"),
            r#""Shift+Ctrl" is not a shortcut: a key is missing after Ctrl"#
        );
        assert_eq!(
            problem("Ctrl+Alt"),
            r#""Ctrl+Alt" is not a shortcut: a key is missing after Alt"#
        );
        assert_eq!(
            problem("Cmd+{"),
            r#""Cmd+{" is not a shortcut: write it as Shift+["#
        );
        assert_eq!(
            problem("Cmd+?"),
            r#""Cmd+?" is not a shortcut: write it as Shift+/"#
        );
        assert_eq!(
            problem("Cmd+~"),
            r#""Cmd+~" is not a shortcut: write it as Shift+`"#
        );
        assert_eq!(
            problem("Cmd+{x"),
            r#""Cmd+{x" is not a shortcut: "{x" is not a key"#
        );
        assert_eq!(
            problem("Cmd+π"),
            r#""Cmd+π" is not a shortcut: "π" is not a key"#
        );
        assert_eq!(
            problem("Cmd+Escape"),
            r#""Cmd+Escape" is not a shortcut: "Escape" is not a key"#
        );
        assert_eq!(
            parse("Cmd+P", Windows).unwrap_err(),
            r#""Cmd+P": Cmd exists only on macOS"#
        );
        assert_eq!(
            problem("M"),
            r#""M" would take the key from the terminal: add Ctrl, Alt or Cmd"#
        );
        assert_eq!(
            parse("Shift+Enter", Windows).unwrap_err(),
            r#""Shift+Enter" would take the key from the terminal: add Ctrl or Alt"#
        );
        for one in ["Ctrl+M", "Alt+M", "Cmd+M", "F8", "Shift+F8"] {
            assert!(parse(one, Mac).is_ok(), "{one}");
        }
    }

    #[test]
    fn long_chords_and_control_characters_are_quoted_safely() {
        let long = format!("Cmd+{}", "x".repeat(CHORD_LIMIT));
        let problem = parse(&long, Mac).unwrap_err();
        assert!(problem.ends_with(&format!(" is longer than {CHORD_LIMIT} bytes")));
        assert!(problem.len() < CHORD_LIMIT + 40, "{problem}");
        let exactly = format!("Ctrl+{}", " ".repeat(CHORD_LIMIT - 6)) + "P";
        assert_eq!(exactly.len(), CHORD_LIMIT);
        assert_eq!(parse(&exactly, Windows).unwrap(), "Ctrl+P");
        assert_eq!(
            parse("Cmd+\u{1b}[31m", Mac).unwrap_err(),
            r#""Cmd+\u{1b}[31m" is not a shortcut: "\u{1b}[31m" is not a key"#
        );
        // Cut at a character's start.
        let wide = "é".repeat(CHORD_LIMIT);
        assert_eq!(quote(&wide), format!("{:?}", "é".repeat(CHORD_LIMIT / 2)));
        assert_eq!(
            quote(&format!("a{wide}")),
            format!("\"a{}\"", "é".repeat(31))
        );
    }

    #[test]
    fn fixed_keys_are_refused_but_for_the_command_they_are_the_default_of() {
        let fixed = |t, id, p| entry_chord(t, id, p).unwrap_err();
        assert_eq!(
            fixed("cmd+c", "palette", Mac),
            r#""cmd+c" is the terminal's Copy and stays fixed"#
        );
        assert_eq!(
            fixed("Cmd+Q", "palette", Mac),
            r#""Cmd+Q" is the menu's Quit and stays fixed"#
        );
        assert_eq!(
            fixed("Ctrl+Shift+V", "palette", Windows),
            r#""Ctrl+Shift+V" is the terminal's Paste and stays fixed"#
        );
        assert!(entry_chord("Ctrl+Shift+V", "toggle-markdown", Windows).is_ok());
        assert!(entry_chord("Ctrl+Shift+C", "toggle-markdown", Windows).is_err());
        // Each platform's own: ⌘C is no Windows key, Ctrl+Shift+C is free on macOS.
        assert!(entry_chord("Ctrl+Shift+C", "palette", Mac).is_ok());
        for f in FIXED {
            let chord = Chord::parse(f.chord, f.platform).unwrap();
            assert_eq!(chord.to_string(), f.chord, "written in Chord's form");
            assert!(chord.fixed(f.platform, "palette").is_some());
        }
    }

    #[test]
    fn accelerators_are_tauris_form() {
        let accelerator = |t| Chord::parse(t, Mac).unwrap().accelerator();
        assert_eq!(accelerator("Shift+Cmd+B"), "Shift+Super+B");
        assert_eq!(accelerator("Cmd+,"), "Super+,");
        assert_eq!(accelerator("Ctrl+Alt+F8"), "Ctrl+Alt+F8");
        assert_eq!(accelerator("F8"), "F8");
        assert_eq!(accelerator("Cmd+PageUp"), "Super+PageUp");
    }

    #[test]
    fn command_ids_are_sorted_and_unique() {
        assert!(COMMAND_IDS.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(
            CURRENT,
            if cfg!(target_os = "macos") {
                Mac
            } else {
                Windows
            }
        );
    }

    #[test]
    fn a_keymap_round_trips_any_value() {
        let keymap: Keymap =
            serde_json::from_value(json!({"palette": "Cmd+K", "comment": 3})).unwrap();
        assert_eq!(keymap.0["comment"], json!(3));
        assert_eq!(
            serde_json::to_value(&keymap).unwrap()["palette"],
            json!("Cmd+K")
        );
        let odd: Keymap = serde_json::from_value(json!([1])).unwrap();
        assert_eq!(odd.0, json!([1]));
        assert_eq!(Keymap::default().0, json!({}));
    }

    #[test]
    fn problems_name_each_bad_entry_and_never_fail() {
        let keymap = Keymap(json!({
            "comment": 3,
            "next-pending": ["F8", "Shift+Cmd+J", 1],
            "palete": "Cmd+K",
            "palette": ["Cmd+J", "Hyper+P", "Cmd+C"],
            "settings": null,
            "tab-1": [],
            "toggle-panel": {"a": 1},
            "worktree-picker": true,
        }));
        assert_eq!(
            check(&keymap, Mac),
            [
                r#"keymap.comment: a shortcut is a text like "Ctrl+Alt+M" or a list of them (got a number)"#,
                r#"keymap.next-pending: a shortcut is a text like "Ctrl+Alt+M" or a list of them (got a list holding something other than texts)"#,
                r#"keymap: Hive has no command "palete""#,
                r#"keymap.palette: "Hyper+P" is not a shortcut: "Hyper" is not a modifier"#,
                r#"keymap.palette: "Cmd+C" is the terminal's Copy and stays fixed"#,
                r#"keymap.toggle-panel: a shortcut is a text like "Ctrl+Alt+M" or a list of them (got an object)"#,
                r#"keymap.worktree-picker: a shortcut is a text like "Ctrl+Alt+M" or a list of them (got true or false)"#,
            ]
        );
        assert_eq!(check(&Keymap::default(), Windows), Vec::<String>::new());
        assert_eq!(
            check(&Keymap(json!("x")), Mac),
            ["keymap: it is a text where command ids and their shortcuts were expected"]
        );
        assert_eq!(
            check(&Keymap(json!(1)), Mac),
            ["keymap: it is a number where command ids and their shortcuts were expected"]
        );
        for (value, kind) in [
            (json!(null), "null"),
            (json!([]), "a list holding something other than texts"),
        ] {
            assert_eq!(
                check(&Keymap(value), Windows),
                [format!(
                    "keymap: it is {kind} where command ids and their shortcuts were expected"
                )]
            );
        }
    }

    #[test]
    fn limits_cut_chords_entries_and_problems() {
        let nine: Vec<String> = (1..=9).map(|n| format!("Ctrl+{n}")).collect();
        let keymap = Keymap(json!({"palette": nine}));
        assert_eq!(
            check(&keymap, Windows),
            ["keymap.palette: more than 8 shortcuts, the rest are ignored"]
        );
        let chords = keymap.chords("palette", Windows).unwrap();
        assert_eq!(chords.len(), CHORDS_LIMIT);
        let eight = Keymap(json!({"palette": nine[..8]}));
        assert_eq!(check(&eight, Windows), Vec::<String>::new());

        let many: serde_json::Map<String, Value> = (0..ENTRY_LIMIT + 1)
            .map(|n| (format!("x{n:03}"), json!("Ctrl+P")))
            .collect();
        let problems = check(&Keymap(Value::Object(many.clone())), Windows);
        assert_eq!(
            problems[0],
            "keymap: more than 128 entries, the rest are ignored"
        );
        assert_eq!(problems.len(), PROBLEM_LIMIT + 1);
        let rest = ENTRY_LIMIT + 1 - PROBLEM_LIMIT;
        assert_eq!(
            problems[PROBLEM_LIMIT],
            format!("… and {rest} more problems")
        );
        let exactly: serde_json::Map<String, Value> =
            many.into_iter().take(PROBLEM_LIMIT).collect();
        let problems = check(&Keymap(Value::Object(exactly)), Windows);
        assert_eq!(problems.len(), PROBLEM_LIMIT);
        assert!(
            problems
                .iter()
                .all(|p| p.starts_with("keymap: Hive has no"))
        );

        // Entries past the limit are not read.
        let mut past: serde_json::Map<String, Value> = (0..ENTRY_LIMIT)
            .map(|n| (format!("a{n:03}"), json!("Ctrl+P")))
            .collect();
        let full = check(&Keymap(Value::Object(past.clone())), Windows);
        assert!(!full.iter().any(|p| p.contains("entries")), "{full:?}");
        past.insert("palette".into(), json!("ctrl+k"));
        let past = Keymap(Value::Object(past));
        assert_eq!(past.chords("palette", Windows), None);
        let mut normalized = past.clone();
        normalize(&mut normalized, Windows);
        assert_eq!(normalized, past);
    }

    #[test]
    fn chords_are_the_valid_ones_of_an_entry() {
        let keymap = Keymap(json!({
            "palette": ["cmd+j", "Hyper+P", "Cmd+C"],
            "comment": [],
            "settings": null,
            "close-tab": 3,
        }));
        let palette = keymap.chords("palette", Mac).unwrap();
        assert_eq!(palette.len(), 1);
        assert_eq!(palette[0].to_string(), "Cmd+J");
        assert_eq!(keymap.chords("comment", Mac), Some(Vec::new()));
        assert_eq!(keymap.chords("settings", Mac), Some(Vec::new()));
        assert_eq!(keymap.chords("close-tab", Mac), None);
        assert_eq!(keymap.chords("tab-1", Mac), None);
        assert_eq!(Keymap(json!([])).chords("palette", Mac), None);
    }

    #[test]
    fn valid_entries_are_normalised_and_bad_ones_kept_as_written() {
        let mut keymap = Keymap(json!({
            "close-tab": 3,
            "comment": ["ctrl+m", "Ctrl+M"],
            "next-pending": ["f8", "shift+cmd+j", "Shift+Cmd+J"],
            "palette": ["cmd+k", "Hyper+P"],
            "palete": "cmd+k",
            "settings": null,
            "tab-1": [],
            "toggle-panel": "cmd+shift+b",
        }));
        normalize(&mut keymap, Mac);
        assert_eq!(
            keymap.0,
            json!({
                "close-tab": 3,
                "comment": "Ctrl+M",
                "next-pending": ["F8", "Shift+Cmd+J"],
                "palette": ["cmd+k", "Hyper+P"],
                "palete": "cmd+k",
                "settings": [],
                "tab-1": [],
                "toggle-panel": "Shift+Cmd+B",
            })
        );
        let mut nine =
            Keymap(json!({"palette": (1..=9).map(|n| format!("ctrl+{n}")).collect::<Vec<_>>()}));
        let before = nine.clone();
        normalize(&mut nine, Windows);
        assert_eq!(nine, before);
        let mut odd = Keymap(json!("x"));
        normalize(&mut odd, Mac);
        assert_eq!(odd.0, json!("x"));
    }
}
