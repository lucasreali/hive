//! Command-line entry point.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::paths::Paths;

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
    /// Connect stdio to the service, starting it if needed (run by the app through `wsl.exe`).
    Bridge,
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();
    let paths = Paths::from_env();
    let result = match cli.command {
        Command::Daemon => block_on(crate::daemon::run(&paths)),
        Command::Bridge => {
            std::env::current_exe().and_then(|hive| block_on(crate::bridge::run(&paths, &hive)))
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("hive: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Runs `task` to completion, then drops the runtime without waiting for blocking
/// work (a pending stdin read would otherwise keep the process alive).
fn block_on(task: impl Future<Output = std::io::Result<()>>) -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(task);
    runtime.shutdown_background();
    result
}
