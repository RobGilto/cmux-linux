//! cmux CLI binary entry point.
//!
//! This binary is independent of GTK4 — it communicates with the running
//! cmux-app instance via Unix socket JSON-RPC.

// The test target compiles this bin without invoking main(), which would
// flag every CLI function as dead code under -D warnings.
#![cfg_attr(test, allow(dead_code))]

#[path = "../cli/mod.rs"]
mod cli;

use clap::Parser;

fn main() -> std::process::ExitCode {
    let cli_args = cli::Cli::parse();
    match cli::run(cli_args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(cli::CliError::ConnectionError(msg)) => {
            eprintln!("Error: {}", msg);
            std::process::ExitCode::from(2)
        }
        Err(cli::CliError::CommandError(msg)) => {
            eprintln!("Error: {}", msg);
            std::process::ExitCode::from(1)
        }
        Err(cli::CliError::ProtocolError(msg)) => {
            eprintln!("Error: {}", msg);
            std::process::ExitCode::from(1)
        }
    }
}
