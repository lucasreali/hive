//! A stand-in for Claude Code in the Windows integration test (12.5.5), which copies it to a
//! `claude.exe` on the terminals' `PATH` behind Hive's wrapper: never the real `claude`.
//! Given `--settings <file>` first, it prints `settings`, then runs that file's hook for each
//! `<event> <value>` pair after it, as Claude Code runs a hook in exec form: the hook's input
//! names `value` as its session, worktree name and worktree path, with this folder as `cwd`.
//! For each one it prints `<event>=<exit code>:<last name of what the hook printed>`.

use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [flag, file, pairs @ ..] = &args[..] else {
        println!("no settings");
        return Ok(());
    };
    if flag != "--settings" {
        println!("no settings");
        return Ok(());
    }
    let settings: Value = serde_json::from_slice(&std::fs::read(file)?)?;
    println!("settings");
    let cwd = std::env::current_dir()?;
    for pair in pairs.chunks(2) {
        let [event, value] = pair else {
            return Err("an event without its value".into());
        };
        let hook = &settings["hooks"][event.as_str()][0]["hooks"][0];
        let command = hook["command"].as_str().ok_or("no hook command")?;
        let hook_args = hook["args"].as_array().ok_or("no hook arguments")?;
        let hook_args = hook_args.iter().filter_map(Value::as_str);
        let input = json!({
            "session_id": value,
            "name": value,
            "worktree_path": value,
            "cwd": cwd,
        });
        let mut child = Command::new(command)
            .args(hook_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        stdin.write_all(input.to_string().as_bytes())?;
        drop(stdin);
        let out = child.wait_with_output()?;
        let printed = String::from_utf8_lossy(&out.stdout);
        let name = Path::new(printed.trim()).file_name().unwrap_or_default();
        let code = out.status.code().unwrap_or(-1);
        println!("{event}={code}:{}", name.to_string_lossy());
    }
    Ok(())
}
