//! Command-line entry point.

use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::paths::Paths;
use crate::worktree;

#[derive(Debug, Parser)]
#[command(
    name = "hive",
    version,
    about = "Companion service and CLI for the Hive desktop app"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the service (started by `hive bridge`; lives as long as the app connection).
    Daemon,
    /// Manage git worktrees in `.claude/worktrees/` (Claude Code's convention).
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommand,
    },
}

#[derive(Debug, Subcommand)]
enum WorktreeCommand {
    /// Create `.claude/worktrees/<name>` on a new branch `worktree-<name>` and print its path.
    Create {
        name: String,
        /// Local or remote branch to start from (default: HEAD).
        #[arg(long)]
        base: Option<String>,
    },
    /// List the repository's worktrees: path and branch.
    List,
    /// Remove a worktree without changes; its branch is kept.
    Remove { name: String },
    /// `WorktreeCreate` hook: reads Claude Code's JSON on stdin, prints only the new path.
    HookCreate,
    /// `WorktreeRemove` hook: reads Claude Code's JSON on stdin.
    HookRemove,
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Daemon => {
            runtime().and_then(|rt| rt.block_on(crate::daemon::run(&Paths::from_env())))
        }
        Command::Worktree { command } => run_worktree(command),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("hive: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_worktree(command: WorktreeCommand) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    match command {
        WorktreeCommand::Create { name, base } => {
            print_path(&worktree::create(&cwd, &name, base.as_deref())?)
        }
        WorktreeCommand::List => worktree::list(&cwd)?
            .iter()
            .try_for_each(|wt| writeln!(io::stdout(), "{wt}")),
        WorktreeCommand::Remove { name } => worktree::remove(&cwd, &name),
        WorktreeCommand::HookCreate => print_path(&worktree::hook_create(io::stdin())?),
        WorktreeCommand::HookRemove => worktree::hook_remove(io::stdin()),
    }
}

/// Prints a path byte for byte (a hook's stdout must be exactly the path).
fn print_path(path: &Path) -> io::Result<()> {
    let mut line = path.as_os_str().as_bytes().to_vec();
    line.push(b'\n');
    io::stdout().write_all(&line)
}

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
}
