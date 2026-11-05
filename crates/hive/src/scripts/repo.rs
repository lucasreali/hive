//! The repository's scripts (23.4): `package.json`'s `scripts`, the tasks of `Taskfile.yml` and
//! the Makefile's targets, at the root of a worktree, listed after the project's run scripts
//! from the settings. Each becomes a command Hive builds from fixed words and the script's name
//! (`pnpm run dev`, `task dev`, `make dev`), typed into a new terminal only when the user picks
//! it; nothing in the file is ever run as it is written.
//!
//! The files are untrusted: each is read only when it is a regular file at the worktree's root
//! (a link is not followed out), at most [`FILE_LIMIT`] bytes, at most [`PER_FILE`] scripts of
//! it are kept, and only names Hive can type as one shell word.

use std::fmt;
use std::fs::{Metadata, OpenOptions};
use std::io;
use std::path::Path;

use hive_protocol::{RunScript, Script, ScriptSource};
use serde::Deserialize;
use serde::de::{MapAccess, Visitor};

use crate::git::read_limited;

/// Largest script file read.
pub const FILE_LIMIT: u64 = 512 * 1024;
/// Most scripts kept from one file.
pub const PER_FILE: usize = 100;
/// Longest script name kept.
pub const NAME_LIMIT: usize = 100;

/// The package managers, by the lockfile that names each, in the order they are looked for;
/// npm without any.
const LOCKFILES: [(&str, &str); 5] = [
    ("bun.lock", "bun"),
    ("bun.lockb", "bun"),
    ("pnpm-lock.yaml", "pnpm"),
    ("yarn.lock", "yarn"),
    ("package-lock.json", "npm"),
];
/// Task's file names, in the order Task looks for them.
const TASKFILES: [&str; 2] = ["Taskfile.yml", "Taskfile.yaml"];
/// The Makefile's names, in the order GNU make looks for them.
const MAKEFILES: [&str; 3] = ["GNUmakefile", "makefile", "Makefile"];
/// Makefile lines that are directives, not rules.
const DIRECTIVES: [&str; 17] = [
    "define", "endef", "else", "endif", "ifeq", "ifneq", "ifdef", "ifndef", "include", "-include",
    "sinclude", "override", "export", "unexport", "private", "vpath", "undefine",
];

/// The scripts of the worktree `dir`: the project's run scripts from the settings (`settings`)
/// first, then the repository's that none of them replaces by name; and why a file was not
/// read, or what was left out of it.
pub fn list(dir: &Path, settings: &[RunScript]) -> (Vec<Script>, Vec<String>) {
    let mut scripts: Vec<Script> = settings
        .iter()
        .map(|run| Script {
            name: run.name.clone(),
            command: run.command.clone(),
            source: ScriptSource::Settings,
        })
        .collect();
    let mut problems = Vec::new();
    let wanted: Vec<&str> = (LOCKFILES.iter().map(|(name, _)| *name))
        .chain(["package.json"])
        .chain(TASKFILES)
        .chain(MAKEFILES)
        .collect();
    let found = present(dir, &wanted);
    let manager = package_manager(&found);
    let sources: [(ScriptSource, &[&str], Parser, &[&str]); 3] = [
        (
            ScriptSource::PackageJson,
            &["package.json"],
            package,
            &[manager, "run"],
        ),
        (ScriptSource::Taskfile, &TASKFILES, taskfile, &["task"]),
        (ScriptSource::Makefile, &MAKEFILES, makefile, &["make"]),
    ];
    for (source, names, parse, words) in sources {
        let Some(file) = names.iter().find(|name| found.contains(name)) else {
            continue;
        };
        let found = read(dir, file).and_then(|bytes| parse(&bytes));
        let found = match found {
            Ok(found) => found,
            Err(why) => {
                problems.push(format!("{file}: {why}"));
                continue;
            }
        };
        let (kept, problem) = checked(found);
        problems.extend(problem.map(|problem| format!("{file}: {problem}")));
        let taken = |name: &String| {
            scripts
                .iter()
                .any(|s| s.source == ScriptSource::Settings && &s.name == name)
        };
        let kept: Vec<String> = kept.into_iter().filter(|name| !taken(name)).collect();
        scripts.extend(kept.into_iter().map(|name| Script {
            command: [words, &[name.as_str()]].concat().join(" "),
            name,
            source,
        }));
    }
    (scripts, problems)
}

/// Reads one file's script names, in the file's order, or says why it cannot.
type Parser = fn(&[u8]) -> Result<Vec<String>, String>;

/// The names of `wanted` that `dir` has an entry of (a link counts: it is refused when read),
/// matched as the folder writes them, so a case-insensitive file system does not make a
/// `makefile` of a `Makefile`.
fn present<'a>(dir: &Path, wanted: &[&'a str]) -> Vec<&'a str> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let names: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.file_name())
        .collect();
    wanted
        .iter()
        .filter(|name| names.iter().any(|n| n == **name))
        .copied()
        .collect()
}

/// The package manager named by the first lockfile `found`, else npm.
fn package_manager(found: &[&str]) -> &'static str {
    LOCKFILES
        .iter()
        .find(|(lockfile, _)| found.contains(lockfile))
        .map_or("npm", |(_, manager)| manager)
}

/// The file `name` at the root of `dir`, when it is a regular file of at most [`FILE_LIMIT`]
/// bytes. A link is refused, also one swapped in after the check (opened without following
/// it); a FIFO does not block the read.
fn read(dir: &Path, name: &str) -> Result<Vec<u8>, String> {
    let path = dir.join(name);
    let plain = |meta: io::Result<Metadata>| match meta {
        Ok(meta) if meta.is_file() => Ok(()),
        _ => Err(io::Error::other("a link or not a regular file, not read")),
    };
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(
        &mut options,
        nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK,
    );
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::custom_flags(
        &mut options,
        windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
    );
    let read = plain(path.symlink_metadata())
        .and_then(|()| options.open(&path))
        .and_then(|mut file| {
            plain(file.metadata())?;
            read_limited(&mut file, FILE_LIMIT)
        });
    read.map_err(|err| err.to_string())
}

/// The names that can be typed, each once, at most [`PER_FILE`] of them; and what was left out.
fn checked(found: Vec<String>) -> (Vec<String>, Option<String>) {
    let mut kept: Vec<String> = Vec::new();
    let mut untypable = 0;
    let mut over = false;
    for name in found {
        if !typable(&name) {
            untypable += 1;
        } else if kept.contains(&name) {
            continue;
        } else if kept.len() == PER_FILE {
            over = true;
        } else {
            kept.push(name);
        }
    }
    let mut problems = Vec::new();
    if untypable > 0 {
        let s = if untypable == 1 { "" } else { "s" };
        problems.push(format!(
            "{untypable} script{s} left out (a name Hive cannot type as one word)"
        ));
    }
    if over {
        problems.push(format!("only the first {PER_FILE} scripts are listed"));
    }
    let problem = (!problems.is_empty()).then(|| problems.join("; "));
    (kept, problem)
}

/// Whether `name` is one word in every shell Hive's terminals run (no space, quote, control
/// character, `$`, glob or redirection): letters, digits and `_ - . : + @ /`, starting with a
/// letter, a digit or `_`, at most [`NAME_LIMIT`] bytes.
fn typable(name: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || "_-.:+@/".contains(c);
    let first = name.chars().next();
    name.len() <= NAME_LIMIT
        && first.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.chars().all(word)
}

/// `package.json`'s `scripts` with a string value, in the file's order.
fn package(bytes: &[u8]) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Package {
        #[serde(default)]
        scripts: Option<Ordered>,
    }
    let package: Package =
        serde_json::from_slice(bytes).map_err(|err| format!("not valid JSON ({err})"))?;
    let scripts = package.scripts.map(|o| o.0).unwrap_or_default();
    Ok(scripts
        .into_iter()
        .filter(|(_, command)| command.is_string())
        .map(|(name, _)| name)
        .collect())
}

/// A JSON object's entries in the file's order (`serde_json`'s map sorts them).
struct Ordered(Vec<(String, serde_json::Value)>);

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value = Ordered;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Ordered(entries))
            }
        }
        deserializer.deserialize_map(Entries)
    }
}

/// The tasks of a Taskfile (Task, taskfile.dev), in the file's order, but its internal ones.
fn taskfile(bytes: &[u8]) -> Result<Vec<String>, String> {
    use serde_norway::Value;
    let file: Value =
        serde_norway::from_slice(bytes).map_err(|err| format!("not valid YAML ({err})"))?;
    let tasks = match file.get("tasks") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Mapping(tasks)) => tasks,
        Some(_) => return Err("`tasks` is not a map".to_owned()),
    };
    let internal = |task: &Value| task.get("internal") == Some(&Value::Bool(true));
    Ok(tasks
        .iter()
        .filter(|(_, task)| !internal(task))
        .filter_map(|(name, _)| name.as_str().map(str::to_owned))
        .collect())
}

/// A Makefile's targets, in the file's order: those it declares `.PHONY` when it declares any
/// (the commands, not the files it builds), else every explicit one. Special targets (`.X`),
/// patterns (`%.o`), variables (`$(OUT)`), recipes and directives are not targets.
fn makefile(bytes: &[u8]) -> Result<Vec<String>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8 text".to_owned())?;
    let text = text.replace("\\\r\n", " ").replace("\\\n", " ");
    let mut targets = Vec::new();
    let mut phony = Vec::new();
    let mut defining = false;
    for line in text.lines() {
        let first = line.split_whitespace().next().unwrap_or("");
        if defining {
            defining = first != "endef";
            continue;
        }
        if line.starts_with('\t') {
            continue;
        }
        if DIRECTIVES.contains(&first) {
            defining = first == "define";
            continue;
        }
        let line = line.split('#').next().unwrap_or("");
        let Some((names, rest)) = line.split_once(':') else {
            continue;
        };
        // `X = a:b`, `X := y`, `X ::= y`, `X :::= y`: variables.
        if names.contains('=') || rest.trim_start_matches(':').starts_with('=') {
            continue;
        }
        let names = names.split_whitespace().map(str::to_owned);
        if names.clone().eq([".PHONY"]) {
            phony.extend(
                rest.trim_start_matches(':')
                    .split_whitespace()
                    .map(str::to_owned),
            );
        } else {
            let target = |name: &String| !name.starts_with('.') && !name.contains(['%', '$']);
            targets.extend(names.filter(target));
        }
    }
    if !phony.is_empty() {
        targets.retain(|target| phony.contains(target));
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(name: &str, command: &str) -> RunScript {
        RunScript {
            name: name.into(),
            command: command.into(),
        }
    }

    fn script(name: &str, command: &str, source: ScriptSource) -> Script {
        Script {
            name: name.into(),
            command: command.into(),
            source,
        }
    }

    /// `(name, command)` of each script listed in `dir` without settings, and the problems.
    fn listed(dir: &Path) -> (Vec<(String, String)>, Vec<String>) {
        let (scripts, problems) = list(dir, &[]);
        let scripts = scripts.into_iter().map(|s| (s.name, s.command)).collect();
        (scripts, problems)
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| ((*a).into(), (*b).into()))
            .collect()
    }

    #[test]
    fn nothing_without_files_but_the_settings() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(list(tmp.path(), &[]), (Vec::new(), Vec::new()));
        let settings = [run("dev", "bun dev --port $PORT")];
        let (scripts, problems) = list(tmp.path(), &settings);
        let dev = script("dev", "bun dev --port $PORT", ScriptSource::Settings);
        assert_eq!((scripts, problems), (vec![dev], Vec::new()));
    }

    #[test]
    fn package_json_scripts_run_with_the_lockfiles_manager_in_file_order() {
        let tmp = tempfile::tempdir().unwrap();
        let json =
            r#"{"name":"x","scripts":{"dev":"vite","build":"tsc","lint":7,"test:e2e":"pw"}}"#;
        std::fs::write(tmp.path().join("package.json"), json).unwrap();
        let expect = |manager: &str| {
            pairs(&[
                ("dev", &format!("{manager} run dev")),
                ("build", &format!("{manager} run build")),
                ("test:e2e", &format!("{manager} run test:e2e")),
            ])
        };
        assert_eq!(listed(tmp.path()), (expect("npm"), Vec::new()));
        // Each lockfile names its manager; a later one in the list loses to an earlier one.
        for (lockfile, manager) in LOCKFILES.iter().rev() {
            std::fs::write(tmp.path().join(lockfile), "").unwrap();
            assert_eq!(listed(tmp.path()).0, expect(manager), "{lockfile}");
        }
        std::fs::remove_file(tmp.path().join("bun.lock")).unwrap();
        assert_eq!(listed(tmp.path()).0, expect("bun"));
        let (scripts, _) = list(tmp.path(), &[]);
        assert!(
            scripts
                .iter()
                .all(|s| s.source == ScriptSource::PackageJson)
        );
    }

    #[test]
    fn package_json_without_scripts_lists_none() {
        let tmp = tempfile::tempdir().unwrap();
        for json in [r#"{}"#, r#"{"scripts":null}"#, r#"{"scripts":{}}"#] {
            std::fs::write(tmp.path().join("package.json"), json).unwrap();
            assert_eq!(listed(tmp.path()), (Vec::new(), Vec::new()), "{json}");
        }
    }

    #[test]
    fn a_malformed_file_is_a_problem_and_the_others_still_count() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Makefile"), "build:\n\tcc x.c\n").unwrap();
        for json in [r#"{"scripts":"#, r#""x""#, r#"{"scripts":[]}"#] {
            std::fs::write(tmp.path().join("package.json"), json).unwrap();
            let (scripts, problems) = listed(tmp.path());
            assert_eq!(scripts, pairs(&[("build", "make build")]), "{json}");
            assert_eq!(problems.len(), 1, "{json}");
            assert!(
                problems[0].starts_with("package.json: not valid JSON ("),
                "{problems:?}"
            );
        }
        std::fs::remove_file(tmp.path().join("package.json")).unwrap();
        for yaml in ["tasks: [", "tasks: 3"] {
            std::fs::write(tmp.path().join("Taskfile.yml"), yaml).unwrap();
            let (scripts, problems) = listed(tmp.path());
            assert_eq!(scripts, pairs(&[("build", "make build")]), "{yaml}");
            assert_eq!(problems.len(), 1, "{yaml}");
            assert!(problems[0].starts_with("Taskfile.yml: "), "{problems:?}");
        }
        let problems = |yaml: &str| {
            std::fs::write(tmp.path().join("Taskfile.yml"), yaml).unwrap();
            listed(tmp.path()).1
        };
        assert!(
            problems("tasks: [")
                .first()
                .unwrap()
                .starts_with("Taskfile.yml: not valid YAML (")
        );
        assert_eq!(problems("tasks: 3"), ["Taskfile.yml: `tasks` is not a map"]);
        std::fs::remove_file(tmp.path().join("Taskfile.yml")).unwrap();
        std::fs::write(tmp.path().join("Makefile"), b"a:\n\xff\n").unwrap();
        assert_eq!(
            listed(tmp.path()),
            (Vec::new(), vec!["Makefile: not UTF-8 text".to_owned()])
        );
    }

    #[test]
    fn taskfile_tasks_run_with_task_but_internal_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let yaml = "version: '3'\ntasks:\n  dev:\n    cmds: [go run .]\n  build: go build\n  \
                    helper:\n    internal: true\n  db:migrate:\n    internal: false\n  7: x\n";
        std::fs::write(tmp.path().join("Taskfile.yaml"), yaml).unwrap();
        let expect = pairs(&[
            ("dev", "task dev"),
            ("build", "task build"),
            ("db:migrate", "task db:migrate"),
        ]);
        assert_eq!(listed(tmp.path()), (expect, Vec::new()));
        // `Taskfile.yml` wins over `.yaml`, as in Task.
        std::fs::write(tmp.path().join("Taskfile.yml"), "tasks:\n  up: x\n").unwrap();
        assert_eq!(listed(tmp.path()).0, pairs(&[("up", "task up")]));
        for empty in ["version: '3'", "tasks:", "''"] {
            std::fs::write(tmp.path().join("Taskfile.yml"), empty).unwrap();
            assert_eq!(listed(tmp.path()), (Vec::new(), Vec::new()), "{empty}");
        }
        let (scripts, _) = list(tmp.path(), &[]);
        assert!(scripts.iter().all(|s| s.source == ScriptSource::Taskfile));
    }

    #[test]
    fn makefile_targets_run_with_make() {
        let tmp = tempfile::tempdir().unwrap();
        let make = "CC := cc\nFLAGS = -a:b\nOUT ::= x\nNEW :::= y\nSH != echo a:b\nX ?= a:b\n\
                    all: app\n\napp: main.o util.o\n\t$(CC) -o app $^ # not: a target\n\
                    %.o: %.c\n\t$(CC) -c $<\n.SUFFIXES:\nclean dist-clean:: # both\n\trm -f app\n\
                    define RULE\nfake: x\nendef\nifeq ($(A),b:c)\nendif\ninclude other.mk\n\
                    long \\\n  wrapped: x\n$(OUT): x\napp: extra\n";
        std::fs::write(tmp.path().join("Makefile"), make).unwrap();
        let expect = ["all", "app", "clean", "dist-clean", "long", "wrapped"]
            .map(|t| (t.to_owned(), format!("make {t}")))
            .to_vec();
        assert_eq!(listed(tmp.path()), (expect, Vec::new()));
        // With `.PHONY`, only its targets: the commands, not the files built.
        let phony = format!("{make}.PHONY: all clean\n.PHONY : test\ntest:\n\tgo test\n");
        std::fs::write(tmp.path().join("Makefile"), phony).unwrap();
        let expect = pairs(&[
            ("all", "make all"),
            ("clean", "make clean"),
            ("test", "make test"),
        ]);
        assert_eq!(listed(tmp.path()), (expect, Vec::new()));
        let (scripts, _) = list(tmp.path(), &[]);
        assert!(scripts.iter().all(|s| s.source == ScriptSource::Makefile));
    }

    #[test]
    fn makefile_lines_end_with_crlf_too() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Makefile"), "a \\\r\n b:\r\n\techo\r\n").unwrap();
        assert_eq!(
            listed(tmp.path()).0,
            pairs(&[("a", "make a"), ("b", "make b")])
        );
    }

    #[test]
    fn the_makefile_make_reads_first() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Makefile"), "plain:\n").unwrap();
        assert_eq!(listed(tmp.path()).0, pairs(&[("plain", "make plain")]));
        std::fs::write(tmp.path().join("GNUmakefile"), "gnu:\n").unwrap();
        assert_eq!(listed(tmp.path()).0, pairs(&[("gnu", "make gnu")]));
    }

    #[test]
    fn every_source_in_order_after_the_settings_which_replace_a_same_named_one() {
        let tmp = tempfile::tempdir().unwrap();
        let json = r#"{"scripts":{"dev":"vite","test":"vitest"}}"#;
        std::fs::write(tmp.path().join("package.json"), json).unwrap();
        std::fs::write(tmp.path().join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(
            tmp.path().join("Taskfile.yml"),
            "tasks:\n  dev: x\n  up: x\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("Makefile"), "dev:\nbuild:\n").unwrap();
        let settings = [
            run("dev", "pnpm dev --port $HIVE_PORT"),
            run("seed", "./seed.sh"),
        ];
        let (scripts, problems) = list(tmp.path(), &settings);
        use ScriptSource::*;
        let expect = vec![
            script("dev", "pnpm dev --port $HIVE_PORT", Settings),
            script("seed", "./seed.sh", Settings),
            script("test", "pnpm run test", PackageJson),
            script("up", "task up", Taskfile),
            script("build", "make build", Makefile),
        ];
        assert_eq!((scripts, problems), (expect, Vec::new()));
        // Without the settings, the repository's same-named scripts all show.
        let (scripts, _) = list(tmp.path(), &[]);
        let names: Vec<_> = scripts
            .iter()
            .map(|s| (s.name.as_str(), s.source))
            .collect();
        let all = [
            ("dev", PackageJson),
            ("test", PackageJson),
            ("dev", Taskfile),
            ("up", Taskfile),
            ("dev", Makefile),
            ("build", Makefile),
        ];
        assert_eq!(names, all);
    }

    #[test]
    fn names_that_cannot_be_typed_as_one_word_are_left_out() {
        assert!(typable("dev"));
        assert!(typable("test:e2e"));
        assert!(typable("_x-y.z+a@b/c"));
        assert!(typable("9"));
        assert!(typable(&"a".repeat(NAME_LIMIT)));
        for bad in [
            "", "-rf", ".x", ":a", "@a", "/a", "a b", "a;b", "a$(x)", "a`x`", "a'b", "a\"b",
            "a\nb", "a\u{1b}b", "a|b", "a>b", "a*", "a?", "a~", "a#", "a=b", "a%", "a\\b", "é",
            "a&b", "a!b", "a(b)", "a{b}", "a[b]", "a,b", "a^b",
        ] {
            assert!(!typable(bad), "{bad:?}");
        }
        assert!(!typable(&"a".repeat(NAME_LIMIT + 1)));
        let tmp = tempfile::tempdir().unwrap();
        let json = r#"{"scripts":{"ok":"x","rm -rf /":"x","$(boom)":"x"}}"#;
        std::fs::write(tmp.path().join("package.json"), json).unwrap();
        let problem = "package.json: 2 scripts left out (a name Hive cannot type as one word)";
        assert_eq!(
            listed(tmp.path()),
            (pairs(&[("ok", "npm run ok")]), vec![problem.to_owned()])
        );
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{"scripts":{"a\u0007":"x"}}"#,
        )
        .unwrap();
        let problem = "package.json: 1 script left out (a name Hive cannot type as one word)";
        assert_eq!(listed(tmp.path()), (Vec::new(), vec![problem.to_owned()]));
    }

    #[test]
    fn at_most_so_many_scripts_of_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let names: Vec<String> = (0..=PER_FILE).map(|i| format!("t{i}")).collect();
        // A name twice counts once, and a bad one does not take a place.
        let make = format!("t0:\nbad'q:\n{}:\n", names.join(":\n"));
        std::fs::write(tmp.path().join("Makefile"), make).unwrap();
        let (scripts, problems) = listed(tmp.path());
        let kept: Vec<String> = scripts.into_iter().map(|(name, _)| name).collect();
        assert_eq!(kept, names[..PER_FILE]);
        let problem = format!(
            "Makefile: 1 script left out (a name Hive cannot type as one word); only the first \
             {PER_FILE} scripts are listed"
        );
        assert_eq!(problems, [problem]);
        let exact = format!("{}:\n", names[..PER_FILE].join(":\n"));
        std::fs::write(tmp.path().join("Makefile"), exact).unwrap();
        assert_eq!(listed(tmp.path()).1, Vec::<String>::new());
    }

    #[test]
    fn a_file_over_the_limit_is_not_read() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("Makefile");
        let mut make = "a:\n".to_owned();
        make.push_str(&"#".repeat(FILE_LIMIT as usize - make.len()));
        std::fs::write(&file, &make).unwrap();
        assert_eq!(listed(tmp.path()).0, pairs(&[("a", "make a")]));
        make.push('#');
        std::fs::write(&file, &make).unwrap();
        let problem = format!("Makefile: input larger than {FILE_LIMIT} bytes");
        assert_eq!(listed(tmp.path()), (Vec::new(), vec![problem]));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_or_a_special_file_is_not_read() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("package.json");
        std::fs::write(&target, r#"{"scripts":{"evil":"x"}}"#).unwrap();
        std::os::unix::fs::symlink(&target, tmp.path().join("package.json")).unwrap();
        // Even a link to a file inside the worktree.
        std::fs::write(tmp.path().join("real.yml"), "tasks:\n  a: x\n").unwrap();
        std::os::unix::fs::symlink("real.yml", tmp.path().join("Taskfile.yml")).unwrap();
        // A folder, and a FIFO, which must not block.
        std::fs::create_dir(tmp.path().join("Makefile")).unwrap();
        let not_read = "a link or not a regular file, not read";
        let expect =
            ["package.json", "Taskfile.yml", "Makefile"].map(|f| format!("{f}: {not_read}"));
        assert_eq!(listed(tmp.path()), (Vec::new(), expect.to_vec()));
        std::fs::remove_dir(tmp.path().join("Makefile")).unwrap();
        nix::unistd::mkfifo(&tmp.path().join("Makefile"), nix::sys::stat::Mode::S_IRWXU).unwrap();
        assert_eq!(listed(tmp.path()).1[2], format!("Makefile: {not_read}"));
        // A dangling link names nothing to read either.
        std::fs::remove_file(&target).unwrap();
        assert_eq!(listed(tmp.path()).1[0], format!("package.json: {not_read}"));
    }
}
