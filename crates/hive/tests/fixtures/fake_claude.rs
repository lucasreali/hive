//! A stand-in for Claude Code in the Windows integration test (12.5.5), which builds it with
//! `rustc` (std only) into a `claude.exe` behind Hive's wrapper: never the real `claude`.
//! Given `--settings <file>` first, it prints `settings`, then runs that file's hook for each
//! `<event> <value>` pair after it, as Claude Code runs a hook in exec form: the hook's input
//! names `value` as its session, worktree name and worktree path, with this folder as `cwd`.
//! For each one it prints `<event>=<exit code>:<last name of what the hook printed>`.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The JSON string whose opening quote is at `text[at]`, unescaped, and the index after it.
fn string(text: &str, at: usize) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut chars = text[at..].char_indices().skip(1);
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, at + i + 1)),
            '\\' => out.push(chars.next()?.1),
            c => out.push(c),
        }
    }
    None
}

/// The command and arguments of `event`'s hook in Hive's settings `text` (pretty JSON, keys
/// sorted: `args` before `command`).
fn hook(text: &str, event: &str) -> Option<(String, Vec<String>)> {
    let rest = &text[text.find(&format!("\"{event}\":"))?..];
    let mut at = rest.find("\"args\"")? + "\"args\"".len();
    let end = at + rest[at..].find(']')?;
    let mut args = Vec::new();
    while let Some(quote) = rest[at..end].find('"') {
        let (arg, next) = string(rest, at + quote)?;
        args.push(arg);
        at = next;
    }
    let command = rest.find("\"command\"")? + "\"command\"".len();
    let (command, _) = string(rest, command + rest[command..].find('"')?)?;
    Some((command, args))
}

/// `text` as a JSON string's contents.
fn escaped(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (file, pairs) = match &args[..] {
        [flag, file, pairs @ ..] if flag == "--settings" => (file, pairs),
        _ => {
            println!("no settings");
            return Ok(());
        }
    };
    let settings = std::fs::read_to_string(file)?;
    println!("settings");
    let cwd = std::env::current_dir()?;
    let cwd = escaped(&cwd.to_string_lossy());
    for pair in pairs.chunks(2) {
        let [event, value] = pair else {
            return Err("an event without its value".into());
        };
        let (command, hook_args) = hook(&settings, event).ok_or("no such hook")?;
        let value = escaped(value);
        let input = format!(
            r#"{{"session_id":"{value}","name":"{value}","worktree_path":"{value}","cwd":"{cwd}"}}"#
        );
        let mut child = Command::new(command)
            .args(hook_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        stdin.write_all(input.as_bytes())?;
        drop(stdin);
        let out = child.wait_with_output()?;
        let printed = String::from_utf8_lossy(&out.stdout);
        let name = Path::new(printed.trim()).file_name().unwrap_or_default();
        let code = out.status.code().unwrap_or(-1);
        println!("{event}={code}:{}", name.to_string_lossy());
    }
    Ok(())
}
