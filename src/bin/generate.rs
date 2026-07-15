//! Standalone generator for shell completions and man page.
//! Usage: cargo run --bin cmux-generate
//! Outputs to packaging/completions/ and packaging/man/
//!
//! NOTE: Uses #[path] to include the CLI module directly instead of
//! going through lib.rs. A lib.rs target breaks ghostty FFI linking
//! for cmux-app (see commit fd436c5b).

// Only the clap definitions are used here; the rest of the CLI module
// is intentionally dead code in this binary.
#![allow(dead_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

#[path = "../cli/mod.rs"]
mod cli;

// cli/discovery.rs resolves the socket via crate::platform::dirs; this binary
// includes cli by path and so needs the same leaf module available. See the
// matching shim in src/bin/cmux.rs for why the leaf is top-level + re-exported.
#[path = "../platform/dirs.rs"]
pub mod platform_dirs;
#[path = "../platform/procinfo.rs"]
pub mod platform_procinfo;
mod platform {
    pub(crate) use super::platform_dirs as dirs;
    pub(crate) use super::platform_procinfo as procinfo;
}

use clap::CommandFactory;
use clap_complete::{generate_to, Shell};
use clap_mangen::Man;
use std::fs;
use std::path::Path;

use cli::Cli;

fn main() -> std::io::Result<()> {
    let mut cmd = Cli::command();

    // Generate shell completions
    let comp_dir = Path::new("packaging/completions");
    fs::create_dir_all(comp_dir)?;

    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        let path = generate_to(shell, &mut cmd, "cmux", comp_dir)?;
        eprintln!("Generated: {}", path.display());
    }

    // Generate man page
    let man_dir = Path::new("packaging/man");
    fs::create_dir_all(man_dir)?;

    let man = Man::new(cmd);
    let mut buf = Vec::new();
    man.render(&mut buf)?;
    fs::write(man_dir.join("cmux.1"), buf)?;
    eprintln!("Generated: packaging/man/cmux.1");

    Ok(())
}
