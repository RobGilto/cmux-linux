//! cmux CLI binary entry point.
//!
//! This binary is independent of GTK4 — it communicates with the running
//! cmux-app instance via Unix socket JSON-RPC.

// The test target compiles this bin without invoking main(), which would
// flag every CLI function as dead code under -D warnings.
#![cfg_attr(test, allow(dead_code, clippy::unwrap_used, clippy::expect_used))]

#[path = "../cli/mod.rs"]
mod cli;

// This binary has no lib.rs to share modules with cmux-app, so it re-includes
// the leaf platform modules the CLI needs by path. `platform::dirs` is a pure
// std+libc leaf (no GTK/config deps), so including it here is cheap and keeps
// the CLI's socket discovery resolving the exact same runtime dir the server
// (cmux-app) writes to. The leaf is declared at top level (whose path base,
// src/bin/, is a real dir) then re-exported under `platform` so call sites can
// say `crate::platform::dirs`, matching cmux-app's module layout.
#[path = "../platform/dirs.rs"]
pub mod platform_dirs;
#[path = "../platform/procinfo.rs"]
pub mod platform_procinfo;
mod platform {
    pub(crate) use super::platform_dirs as dirs;
    pub(crate) use super::platform_procinfo as procinfo;
}

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
