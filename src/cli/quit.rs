//! `cmux quit` — graceful shutdown, the counterpart of `cmux launch`.
//!
//! Escalation ladder:
//! 1. Graceful: `system.quit` over the socket → normal GTK teardown, session
//!    saved exactly as on a window close.
//! 2. Fallback: socket unreachable but a cmux-app process exists → SIGTERM.
//! 3. Bounded wait (~5s) until the socket stops answering AND the process
//!    is gone; then SIGKILL any survivor and re-verify.
//!
//! Idempotent like `launch`: quitting a non-running instance prints
//! "not running" and exits 0. Narration respects CMUX_QUIET=1.

use std::time::{Duration, Instant};

fn quiet() -> bool {
    matches!(std::env::var("CMUX_QUIET").as_deref(), Ok("1") | Ok("true"))
}

fn stage(msg: &str) {
    if !quiet() {
        println!("{msg}");
    }
}

/// PIDs of this user's cmux-app processes (`cmux-app` or the packaged
/// `cmux-app.bin`), excluding ourselves. /proc comm is truncated to 15
/// chars, so prefix-match.
pub(crate) fn find_app_pids() -> Vec<i32> {
    let me = std::process::id() as i32;
    let my_uid = unsafe { libc::getuid() };
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_string_lossy().parse::<i32>().ok())
        .filter(|&pid| pid != me)
        .filter(|&pid| {
            std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .map(|c| c.trim().starts_with("cmux-app"))
                .unwrap_or(false)
        })
        .filter(|&pid| {
            // Only our own processes — never signal another user's cmux.
            std::fs::metadata(format!("/proc/{pid}"))
                .map(|m| std::os::unix::fs::MetadataExt::uid(&m) == my_uid)
                .unwrap_or(false)
        })
        .collect()
}

fn ping_ok(socket_override: &Option<String>) -> bool {
    let Some(path) = socket_override
        .clone()
        .or_else(super::discovery::discover_socket)
    else {
        return false;
    };
    super::socket_client::SocketClient::connect(&path, Duration::from_secs(2))
        .and_then(|mut c| c.call("system.ping", serde_json::json!({})))
        .is_ok()
}

pub(crate) fn signal_pids(pids: &[i32], sig: i32) {
    for &pid in pids {
        unsafe {
            libc::kill(pid, sig);
        }
    }
}

fn wait_until_down(socket_override: &Option<String>, deadline: Instant) -> bool {
    loop {
        if !ping_ok(socket_override) && find_app_pids().is_empty() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn run_quit(cli_socket: &Option<String>) -> Result<(), super::CliError> {
    let socket_up = ping_ok(cli_socket);
    let pids = find_app_pids();

    if !socket_up && pids.is_empty() {
        stage("not running");
        return Ok(());
    }

    if socket_up {
        stage("quitting gracefully…");
        // Best-effort: the app acks then quits; a dropped reply is fine.
        if let Some(path) = cli_socket
            .clone()
            .or_else(super::discovery::discover_socket)
        {
            if let Ok(mut c) =
                super::socket_client::SocketClient::connect(&path, Duration::from_secs(2))
            {
                let _ = c.call("system.quit", serde_json::json!({}));
            }
        }
    } else {
        stage("socket unreachable — sending SIGTERM");
        signal_pids(&pids, libc::SIGTERM);
    }

    if wait_until_down(cli_socket, Instant::now() + Duration::from_secs(5)) {
        stage("stopped");
        return Ok(());
    }

    stage("escalating to SIGKILL");
    signal_pids(&find_app_pids(), libc::SIGKILL);

    if wait_until_down(cli_socket, Instant::now() + Duration::from_secs(2)) {
        stage("force-killed");
        return Ok(());
    }

    Err(super::CliError::CommandError(
        "could not stop cmux-app (still alive after SIGKILL)".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawn a fake process whose comm is "cmux-app" (copied sleep binary)
    /// and check the scanner finds it and SIGTERM removes it.
    #[test]
    fn scanner_finds_and_sigterm_stops_fake_app() {
        let dir = std::env::temp_dir().join(format!("cmux-quit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let fake = dir.join("cmux-app");
        std::fs::copy("/usr/bin/sleep", &fake).expect("copy sleep");
        let mut child = std::process::Command::new(&fake)
            .arg("30")
            .spawn()
            .expect("spawn fake cmux-app");
        let pid = child.id() as i32;

        std::thread::sleep(Duration::from_millis(100));
        assert!(
            find_app_pids().contains(&pid),
            "scanner should find the fake cmux-app"
        );

        signal_pids(&[pid], libc::SIGTERM);
        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            !find_app_pids().contains(&pid),
            "fake cmux-app should be gone after SIGTERM"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scanner_ignores_unrelated_processes() {
        // Our own test process must never appear.
        assert!(!find_app_pids().contains(&(std::process::id() as i32)));
    }
}
