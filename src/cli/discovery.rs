//! Socket path discovery chain for the cmux CLI.
//!
//! Ports the Python `_default_socket_path()` logic from tests_v2/cmux.py to Rust.
//! Discovery order matches D-11.

use std::path::Path;

/// Discover the cmux socket path using the standard search chain.
///
/// Returns `Some(path)` if a candidate is found, `None` otherwise.
/// The caller should check whether the path actually exists if needed;
/// this function returns the best candidate even if the socket file
/// is not yet present (for env-var overrides and fallback paths).
pub fn discover_socket() -> Option<String> {
    // 1. CMUX_SOCKET_PATH — upstream `054cc9ff` standardised on this name
    //    across the socket tooling and Python client; prefer it over the
    //    fork's historical `CMUX_SOCKET` to keep wire/env compatibility.
    if let Ok(val) = std::env::var("CMUX_SOCKET_PATH") {
        if !val.is_empty() {
            return Some(val);
        }
    }

    // 2. CMUX_SOCKET — retained for back-compat with existing fork users.
    if let Ok(val) = std::env::var("CMUX_SOCKET") {
        if !val.is_empty() {
            return Some(val);
        }
    }

    // 2. <runtime_dir>/cmux/cmux.sock — must match the server's socket_path().
    // Linux: $XDG_RUNTIME_DIR (fallback /run/user/{uid}); macOS: $TMPDIR.
    // See platform::dirs::runtime_dir.
    let runtime_base = crate::platform::dirs::runtime_dir();
    let xdg_socket = runtime_base
        .join("cmux")
        .join("cmux.sock")
        .to_string_lossy()
        .to_string();
    if Path::new(&xdg_socket).exists() {
        return Some(xdg_socket);
    }

    // 3. <runtime_dir>/cmux/last-socket-path marker file
    let marker = runtime_base
        .join("cmux")
        .join("last-socket-path")
        .to_string_lossy()
        .to_string();
    if let Ok(contents) = std::fs::read_to_string(&marker) {
        let path = contents.trim().to_string();
        if !path.is_empty() && Path::new(&path).exists() {
            return Some(path);
        }
    }

    // 4. /tmp/cmux-debug.sock
    let debug_sock = "/tmp/cmux-debug.sock";
    if Path::new(debug_sock).exists() {
        return Some(debug_sock.to_string());
    }

    // 5. Glob /tmp/cmux-debug-*.sock — sort by mtime descending, return newest
    if let Ok(entries) = std::fs::read_dir("/tmp") {
        let mut candidates: Vec<(String, std::time::SystemTime)> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("cmux-debug-") && name_str.ends_with(".sock") {
                if let Ok(meta) = entry.metadata() {
                    let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    candidates.push((entry.path().to_string_lossy().to_string(), mtime));
                }
            }
        }
        if !candidates.is_empty() {
            candidates.sort_by_key(|c| std::cmp::Reverse(c.1));
            return Some(candidates[0].0.clone());
        }
    }

    None
}
