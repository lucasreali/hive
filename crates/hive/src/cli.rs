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
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Daemon => {
            runtime().and_then(|rt| rt.block_on(crate::daemon::run(&Paths::from_env())))
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

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
}
