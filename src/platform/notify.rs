//! Cross-platform desktop notifications.
//!
//! Linux shells out to freedesktop's `notify-send`; macOS shells out to
//! `osascript -e 'display notification ...'`. A subprocess (rather than an
//! in-process D-Bus/UserNotifications client) is deliberate on both platforms:
//! see the note in `app_state::send_bell_notification` for the GNOME Shell
//! name-vanished race the subprocess avoids on Linux; the same
//! fire-and-forget shape keeps macOS simple.
//!
//! PORT STATUS: the macOS arm was authored on Linux and not run on a Mac.
//! The Linux arm preserves prior behavior. See
//! specs/cmux-macos-extensibility.html Phase 2.

/// Post a desktop notification titled `title` with body `body`. Fire and
/// forget — spawns a detached subprocess and never blocks the caller. Any
/// failure is logged at debug/warn, never propagated (a missing notifier must
/// not take down the app).
pub fn bell(title: &str, body: &str) {
    let title = title.to_string();
    let body = body.to_string();
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        let result = {
            // AppleScript string literals: escape backslashes then quotes so a
            // workspace name containing a quote can't break out of the literal.
            let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
            let script = format!(
                "display notification \"{}\" with title \"{}\"",
                esc(&body),
                esc(&title)
            );
            std::process::Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .status()
        };

        #[cfg(not(target_os = "macos"))]
        let result = std::process::Command::new("notify-send")
            .arg("--app-name=cmux")
            .arg("--icon=utilities-terminal")
            .arg("--expire-time=5000")
            .arg(&title)
            .arg(&body)
            .status();

        match result {
            Ok(status) if !status.success() => {
                tracing::debug!("cmux: notifier exited with {status}");
            }
            Err(e) => {
                tracing::warn!("cmux: failed to run desktop notifier: {e}");
            }
            _ => {}
        }
    });
}
