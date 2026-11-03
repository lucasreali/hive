//! The folder every Claude account shares (21.9, D22.4): `<data>/hive/shared`, laid out as a
//! Claude Code plugin. The `claude` wrapper passes it with `--plugin-dir` to every `claude` of
//! Hive's terminals, whatever their account (`CLAUDE_CONFIG_DIR`), so its skills, agents,
//! commands and MCP servers (`.mcp.json`) are in each one. Nothing else is shared: an
//! account's login stays in its own folder, which Hive never reads.

use std::io;
use std::path::Path;

use hive_protocol::Control;
use serde_json::{Value, json};

/// The shared folder's name, in Hive's data folder.
pub const FOLDER: &str = "shared";
/// The plugin's name, which Claude Code names its skills, agents and commands under
/// (`/hive-shared:<skill>`): Hive's own, unlikely to be a plugin the user installed, which a
/// `--plugin-dir` plugin of the same name would replace.
pub const PLUGIN: &str = "hive-shared";
/// The most names listed of each kind.
pub const MOST: usize = 200;
/// The most entries read of each folder, so a huge one is listed in bounded time.
const SCAN: usize = 10_000;
/// The longest name listed, in bytes.
const NAME_BYTES: usize = 128;
/// The largest `.mcp.json` read.
const MCP_BYTES: u64 = 256 * 1024;
/// The folders made with the shared folder, where Claude Code looks for each kind.
const KINDS: [&str; 3] = ["skills", "agents", "commands"];

/// Makes the shared folder `dir` when missing: its kinds' folders and a minimal manifest
/// (`.claude-plugin/plugin.json`). What is there already, the manifest too, is left as it is.
pub fn ensure(dir: &Path) -> io::Result<()> {
    let manifest = dir.join(".claude-plugin");
    std::fs::create_dir_all(&manifest)?;
    for kind in KINDS {
        std::fs::create_dir_all(dir.join(kind))?;
    }
    let file = manifest.join("plugin.json");
    if file.symlink_metadata().is_ok() {
        return Ok(());
    }
    let text = json!({
        "name": PLUGIN,
        "description": "Skills, agents, commands and MCP servers Hive gives every Claude account",
    });
    crate::wrapper::write_atomic(&file, format!("{text:#}\n").as_bytes(), 0o644)
}

/// The `shared` answer: what the shared folder `dir` holds, made first when missing.
pub fn list(dir: &Path) -> Control {
    let made = ensure(dir).map_err(|err| format!("Cannot make {}: {err}", dir.display()));
    let (mcp_servers, mcp) = mcp_servers(&dir.join(".mcp.json"));
    Control::Shared {
        folder: dir.display().to_string(),
        plugin: PLUGIN.to_owned(),
        skills: names(&dir.join("skills"), skill),
        agents: names(&dir.join("agents"), markdown),
        commands: names(&dir.join("commands"), markdown),
        mcp_servers,
        error: made.err().or(mcp),
    }
}

/// The names `pick` finds in `folder`'s entries (a missing folder has none): fair ones only,
/// sorted, at most [`MOST`].
fn names(folder: &Path, pick: fn(&Path, &str) -> Option<String>) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let found = entries
        .take(SCAN)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            pick(&entry.path(), &name)
        });
    capped(found)
}

/// A skill: a folder holding a `SKILL.md`, named after the folder.
fn skill(path: &Path, name: &str) -> Option<String> {
    path.join("SKILL.md").is_file().then(|| name.to_owned())
}

/// An agent or a command: a `.md` file, named after it without the extension.
fn markdown(path: &Path, name: &str) -> Option<String> {
    let stem = name.strip_suffix(".md")?;
    path.is_file().then(|| stem.to_owned())
}

/// The fair names of `found`, sorted, at most [`MOST`].
fn capped(found: impl Iterator<Item = String>) -> Vec<String> {
    let mut names: Vec<String> = found.filter(|name| fair(name)).collect();
    names.sort_unstable();
    names.truncate(MOST);
    names
}

/// Whether `name` may be shown: not empty, not hidden, at most [`NAME_BYTES`], with no control
/// or bidirectional-formatting character (which could make it read as another).
fn fair(name: &str) -> bool {
    let bidi = |c: char| matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    !name.is_empty()
        && name.len() <= NAME_BYTES
        && !name.starts_with('.')
        && !name.chars().any(|c| c.is_control() || bidi(c))
}

/// The MCP servers of the `.mcp.json` at `file` (the names under `mcpServers`, or at the top
/// when the file has no such key, as Claude Code reads it), and why it was not understood. No
/// file, no servers.
fn mcp_servers(file: &Path) -> (Vec<String>, Option<String>) {
    let read = || -> Result<Value, String> {
        if !std::fs::metadata(file)
            .map_err(|err| err.to_string())?
            .is_file()
        {
            return Err("not a file".to_owned());
        }
        let mut input = std::fs::File::open(file).map_err(|err| err.to_string())?;
        let bytes = crate::git::read_limited(&mut input, MCP_BYTES).map_err(|e| e.to_string())?;
        serde_json::from_slice(&bytes).map_err(|err| err.to_string())
    };
    if file.symlink_metadata().is_err() {
        return (Vec::new(), None);
    }
    let servers = match read() {
        Ok(Value::Object(top)) => match top.get("mcpServers") {
            Some(Value::Object(servers)) => servers.clone(),
            _ => top,
        },
        Ok(_) => return (Vec::new(), Some(".mcp.json: not a JSON object".to_owned())),
        Err(err) => return (Vec::new(), Some(format!(".mcp.json: {err}"))),
    };
    (capped(servers.into_iter().map(|(name, _)| name)), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skills, agents, commands, MCP servers and the error, as `list` answers them.
    type Listed = (
        Vec<String>,
        Vec<String>,
        Vec<String>,
        Vec<String>,
        Option<String>,
    );

    fn shared(dir: &Path) -> Listed {
        let Control::Shared {
            folder,
            plugin,
            skills,
            agents,
            commands,
            mcp_servers,
            error,
        } = list(dir)
        else {
            panic!("expected shared");
        };
        assert_eq!(folder, dir.display().to_string());
        assert_eq!(plugin, "hive-shared");
        (skills, agents, commands, mcp_servers, error)
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_missing_folder_is_made_as_a_plugin_with_a_minimal_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("data").join("shared");
        let empty = (vec![], vec![], vec![], vec![], None);
        assert_eq!(shared(&dir), empty);
        let manifest = dir.join(".claude-plugin").join("plugin.json");
        let text = std::fs::read_to_string(&manifest).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["name"], "hive-shared");
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert!(text.ends_with("}\n"), "{text}");
        for kind in KINDS {
            assert!(dir.join(kind).is_dir(), "{kind}");
        }
        // The user's own manifest stays as it is.
        std::fs::write(&manifest, "{\"name\": \"mine\"}").unwrap();
        ensure(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(&manifest).unwrap(),
            "{\"name\": \"mine\"}"
        );
        let names: Vec<_> = std::fs::read_dir(manifest.parent().unwrap())
            .unwrap()
            .collect();
        assert_eq!(names.len(), 1, "temp files left behind");
    }

    #[test]
    fn skills_agents_commands_and_mcp_servers_are_listed_by_name_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write(&dir.join("skills/review/SKILL.md"), "x");
        write(&dir.join("skills/deploy/SKILL.md"), "x");
        // A folder without a SKILL.md, and a file, are no skills.
        std::fs::create_dir_all(dir.join("skills/empty")).unwrap();
        write(&dir.join("skills/notes.md"), "x");
        write(&dir.join("agents/tester.md"), "x");
        write(&dir.join("agents/reviewer.md"), "x");
        write(&dir.join("agents/readme.txt"), "x");
        // A folder named like a Markdown file is no agent.
        std::fs::create_dir_all(dir.join("agents/folder.md")).unwrap();
        write(&dir.join("commands/about.md"), "x");
        write(
            &dir.join(".mcp.json"),
            r#"{"mcpServers": {"web": {}, "db": {}}}"#,
        );
        let (skills, agents, commands, mcp, error) = shared(dir);
        assert_eq!(skills, ["deploy", "review"]);
        assert_eq!(agents, ["reviewer", "tester"]);
        assert_eq!(commands, ["about"]);
        assert_eq!(mcp, ["db", "web"]);
        assert_eq!(error, None);
        // Without the `mcpServers` key, the servers are at the top.
        write(&dir.join(".mcp.json"), r#"{"solo": {"command": "x"}}"#);
        assert_eq!(shared(dir).3, ["solo"]);
    }

    #[test]
    fn only_fair_names_are_listed_and_at_most_200_of_a_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let long = "a".repeat(NAME_BYTES);
        for name in [
            ".hidden",
            "tab\there",
            "rtl\u{202E}gpj",
            "lrm\u{200E}",
            "iso\u{2068}",
            &format!("{long}b"),
            &long,
            "ok",
        ] {
            write(&dir.join("commands").join(format!("{name}.md")), "x");
        }
        assert_eq!(shared(dir).2, [long.clone(), "ok".to_owned()]);
        assert!(!fair("\u{2069}x") && !fair("") && fair("é"));
        for i in 0..=MOST {
            write(&dir.join("agents").join(format!("a{i:03}.md")), "x");
        }
        let agents = shared(dir).1;
        assert_eq!(agents.len(), MOST);
        assert_eq!(agents.last().unwrap(), "a199");
    }

    #[test]
    fn an_mcp_json_not_understood_is_said_and_lists_no_server() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mcp = dir.join(".mcp.json");
        write(&mcp, "[1]");
        let (.., servers, error) = shared(dir);
        assert_eq!(servers, Vec::<String>::new());
        assert_eq!(error.as_deref(), Some(".mcp.json: not a JSON object"));
        write(&mcp, "{");
        let error = shared(dir).4.unwrap();
        assert!(error.starts_with(".mcp.json: EOF"), "{error}");
        write(
            &mcp,
            &format!("{{\"a\": \"{}\"}}", "x".repeat(MCP_BYTES as usize)),
        );
        let error = shared(dir).4.unwrap();
        assert_eq!(
            error,
            format!(".mcp.json: input larger than {MCP_BYTES} bytes")
        );
        std::fs::remove_file(&mcp).unwrap();
        std::fs::create_dir(&mcp).unwrap();
        assert_eq!(shared(dir).4.as_deref(), Some(".mcp.json: not a file"));
        // Exactly the largest read.
        std::fs::remove_dir(&mcp).unwrap();
        let pad = MCP_BYTES as usize - r#"{"a": ""}"#.len();
        write(&mcp, &format!("{{\"a\": \"{}\"}}", "x".repeat(pad)));
        assert_eq!(shared(dir).3, ["a"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_made_is_said() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file");
        std::fs::write(&file, "").unwrap();
        let dir = file.join("shared");
        let error = shared(&dir).4.unwrap();
        assert!(
            error.starts_with(&format!("Cannot make {}: ", dir.display())),
            "{error}"
        );
        // A part that cannot be made after the manifest's folder.
        let dir = tmp.path().join("shared");
        std::fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        std::fs::write(dir.join("agents"), "").unwrap();
        assert!(ensure(&dir).is_err());
        assert!(!dir.join(".claude-plugin/plugin.json").exists());
        // An `.mcp.json` that cannot be read.
        let tmp = tempfile::tempdir().unwrap();
        let mcp = tmp.path().join(".mcp.json");
        std::os::unix::fs::symlink(tmp.path().join("missing"), &mcp).unwrap();
        let error = shared(tmp.path()).4.unwrap();
        assert!(error.starts_with(".mcp.json: No such file"), "{error}");
    }
}
