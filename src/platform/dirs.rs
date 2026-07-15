//! Cross-platform base-directory resolution.
//!
//! Linux resolves the freedesktop `$XDG_*` variables (with their standard
//! fallbacks); macOS has no `$XDG_*` convention, so it maps to the
//! `~/Library/{Application Support,Caches,Logs}` hierarchy and `$TMPDIR`.
//!
//! Every function returns a **base** directory. Callers append their own
//! `cmux` (or `cmux/<file>`) subpath, exactly as the pre-port code did with
//! the raw `$XDG_*` values — so on Linux the resulting paths are byte-for-byte
//! identical to before this module existed.
//!
//! PORT STATUS: the macOS arms were authored on Linux and have not been
//! compiled or run on a Mac. The Linux arms preserve prior behavior and are
//! covered by the unit tests below. See specs/cmux-macos-extensibility.html
//! Phase 2.
//!
//! This module is the complete base-directory API surface; call sites are
//! being migrated onto it incrementally (socket path first), so some
//! resolvers are not yet wired everywhere — allow dead code until the sweep
//! of the remaining `$XDG_*` sites lands.
#![allow(dead_code)]

use std::path::PathBuf;

/// The user's home directory, falling back to `/tmp` if `HOME` is unset
/// (which should never happen for an interactive session).
#[cfg(target_os = "macos")]
fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()))
}

/// Per-user runtime directory — ephemeral sockets, pid markers, the control
/// socket. Base dir; callers join their own `cmux` subdir.
///
/// - Linux: `$XDG_RUNTIME_DIR`, fallback `/run/user/{uid}` (unchanged).
/// - macOS: `$TMPDIR` (the per-user, auto-reaped runtime dir), fallback `/tmp`.
pub fn runtime_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from(
            std::env::var("XDG_RUNTIME_DIR")
                .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() })),
        )
    }
    #[cfg(target_os = "macos")]
    {
        // $TMPDIR on macOS is a per-user dir like /var/folders/xx/.../T/; it is
        // the closest analog to XDG_RUNTIME_DIR (private, cleaned periodically).
        let raw = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(raw.trim_end_matches('/'))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PathBuf::from("/tmp")
    }
}

/// Persistent state (session snapshots, last-workspace). Base dir; join `cmux`.
///
/// - Linux: `$XDG_STATE_HOME`, fallback `~/.local/state`.
/// - macOS: `~/Library/Application Support`.
pub fn state_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        xdg_or_home("XDG_STATE_HOME", ".local/state")
    }
    #[cfg(target_os = "macos")]
    {
        home().join("Library").join("Application Support")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PathBuf::from("/tmp")
    }
}

/// User configuration (config.toml). Base dir; join `cmux`.
///
/// - Linux: `$XDG_CONFIG_HOME`, fallback `~/.config`.
/// - macOS: `~/Library/Application Support`.
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        xdg_or_home("XDG_CONFIG_HOME", ".config")
    }
    #[cfg(target_os = "macos")]
    {
        home().join("Library").join("Application Support")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PathBuf::from("/tmp")
    }
}

/// User data (persisted artifacts). Base dir; join `cmux`.
///
/// - Linux: `$XDG_DATA_HOME`, fallback `~/.local/share`.
/// - macOS: `~/Library/Application Support`.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        xdg_or_home("XDG_DATA_HOME", ".local/share")
    }
    #[cfg(target_os = "macos")]
    {
        home().join("Library").join("Application Support")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PathBuf::from("/tmp")
    }
}

/// Disposable cache. Base dir; join `cmux`.
///
/// - Linux: `$XDG_CACHE_HOME`, fallback `~/.cache`.
/// - macOS: `~/Library/Caches`.
pub fn cache_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        xdg_or_home("XDG_CACHE_HOME", ".cache")
    }
    #[cfg(target_os = "macos")]
    {
        home().join("Library").join("Caches")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PathBuf::from("/tmp")
    }
}

/// Log output. Base dir; join `cmux`.
///
/// - Linux: `$XDG_STATE_HOME`, fallback `~/.local/state` (logs live under
///   state on freedesktop systems).
/// - macOS: `~/Library/Logs`.
pub fn log_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        xdg_or_home("XDG_STATE_HOME", ".local/state")
    }
    #[cfg(target_os = "macos")]
    {
        home().join("Library").join("Logs")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PathBuf::from("/tmp")
    }
}

/// `$VAR` if set and non-empty, else `$HOME/<fallback>`. Linux-only helper.
#[cfg(target_os = "linux")]
fn xdg_or_home(var: &str, fallback: &str) -> PathBuf {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()))
            .join(fallback),
    }
}

/// Serializes tests that mutate process-global environment variables. The env
/// is shared across all test threads in a binary, so any two tests that
/// set/remove the same var race unless they hold this lock. Referenced from
/// other modules' tests too (e.g. `socket::tests::test_socket_path_creation`)
/// so the whole binary's env-mutating tests run mutually exclusive.
#[cfg(test)]
pub(crate) static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// Acquire the shared env lock, tolerating a poisoned mutex from an
    /// earlier panicking test (we only guard env access, not invariants).
    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn runtime_dir_prefers_xdg() {
        let _g = env_guard();
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", "/tmp/xdg-runtime-test") };
        assert_eq!(runtime_dir(), PathBuf::from("/tmp/xdg-runtime-test"));
    }

    #[test]
    fn runtime_dir_falls_back_to_run_user() {
        let _g = env_guard();
        unsafe { std::env::remove_var("XDG_RUNTIME_DIR") };
        let uid = unsafe { libc::getuid() };
        assert_eq!(runtime_dir(), PathBuf::from(format!("/run/user/{uid}")));
    }

    #[test]
    fn state_dir_prefers_xdg_then_home_fallback() {
        let _g = env_guard();
        unsafe { std::env::set_var("XDG_STATE_HOME", "/tmp/xdg-state-test") };
        assert_eq!(state_dir(), PathBuf::from("/tmp/xdg-state-test"));
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
        unsafe { std::env::set_var("HOME", "/home/tester") };
        assert_eq!(state_dir(), PathBuf::from("/home/tester/.local/state"));
    }

    #[test]
    fn empty_xdg_var_uses_home_fallback() {
        let _g = env_guard();
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "") };
        unsafe { std::env::set_var("HOME", "/home/tester") };
        assert_eq!(config_dir(), PathBuf::from("/home/tester/.config"));
    }
}
