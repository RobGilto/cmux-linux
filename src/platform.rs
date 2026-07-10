//! Launch-time platform self-configuration.
//!
//! The NVIDIA GL workaround historically lived in packaging wrapper scripts
//! (packaging/scripts/cmux-app-wrapper.sh), so the raw binary needed
//! `GDK_DEBUG=gl-prefer-gl` typed by hand on NVIDIA machines. These functions
//! move that knowledge into the binary itself.
//!
//! INVARIANT: apply_launch_env() and strip_child_env() mutate the process
//! environment and MUST run as the first thing in main(), before GTK/GDK
//! initialization and before any threads spawn.

use std::sync::OnceLock;

/// What launch-time configuration was auto-applied, for `system.identify`,
/// `cmux doctor`, and the startup log banner.
static APPLIED: OnceLock<Vec<String>> = OnceLock::new();

/// The env vars stripped from the process (and therefore from every child
/// shell) so nested agents don't misfile their sessions under the parent
/// agent's identity. Overridable via `[env] strip` in config.toml; an
/// explicitly empty list disables stripping.
pub const DEFAULT_STRIP_PATTERNS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_*",
    "CLAUDE_EFFORT",
];

/// True when the NVIDIA proprietary driver is loaded. File checks instead of
/// exec'ing nvidia-smi: no PATH dependency, no subprocess before GTK init.
pub fn is_nvidia() -> bool {
    std::path::Path::new("/sys/module/nvidia").exists()
        || std::path::Path::new("/proc/driver/nvidia/version").exists()
}

fn session_type() -> String {
    std::env::var("XDG_SESSION_TYPE").unwrap_or_default()
}

/// Append `gl-prefer-gl` to an existing GDK_DEBUG value, preserving whatever
/// flags are already there. Returns None when nothing needs to change.
fn compute_gdk_debug(existing: Option<&str>) -> Option<String> {
    match existing {
        Some(v) if v.split(',').any(|f| f.trim() == "gl-prefer-gl") => None,
        Some(v) if !v.is_empty() => Some(format!("{v},gl-prefer-gl")),
        _ => Some("gl-prefer-gl".to_string()),
    }
}

/// Case-sensitive env-key match supporting a trailing `*` glob
/// (`CLAUDE_CODE_*` matches `CLAUDE_CODE_SESSION_ID`).
fn matches_pattern(key: &str, pattern: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => key.starts_with(prefix),
        None => key == pattern,
    }
}

/// Self-configure the GL environment before GTK init.
///
/// - On NVIDIA (mode "auto") or always (mode "force"): ensure GDK_DEBUG
///   contains `gl-prefer-gl` — GDK otherwise binds the GLES API at EGL init
///   and can't create the desktop-GL context libghostty's renderer needs
///   ("Unable to create a GL context", blank window).
/// - On X11 sessions with GDK_BACKEND unset: pin GDK_BACKEND=x11 so GTK4
///   doesn't pick Wayland/EGL just because the libraries are present.
/// - On Wayland+NVIDIA the native Wayland path works once gl-prefer-gl is
///   set, so the backend is left alone unless `force_x11_backend = true`
///   (the old wrapper-script behavior) is configured.
///
/// Mode "off" disables all of it. Returns the list of applied settings.
pub fn apply_launch_env(cfg: &crate::config::LaunchConfig) -> &'static Vec<String> {
    APPLIED.get_or_init(|| {
        let mut applied = Vec::new();
        let mode = cfg.gl_workaround.as_str();
        if mode == "off" {
            return applied;
        }

        let nvidia = is_nvidia();
        if mode == "force" || nvidia {
            let existing = std::env::var("GDK_DEBUG").ok();
            if let Some(v) = compute_gdk_debug(existing.as_deref()) {
                std::env::set_var("GDK_DEBUG", &v);
                applied.push(format!(
                    "GDK_DEBUG={v} ({})",
                    if mode == "force" { "forced" } else { "auto, nvidia detected" }
                ));
            }
        }

        if std::env::var("GDK_BACKEND").is_err() {
            let session = session_type();
            if session == "x11" {
                std::env::set_var("GDK_BACKEND", "x11");
                applied.push("GDK_BACKEND=x11 (auto, x11 session)".to_string());
            } else if session == "wayland" && nvidia && cfg.force_x11_backend {
                std::env::set_var("GDK_BACKEND", "x11");
                applied.push("GDK_BACKEND=x11 (config: force_x11_backend)".to_string());
            }
        }

        applied
    })
}

/// What apply_launch_env() applied, for identify/doctor/logging.
/// Empty slice if it hasn't run (or applied nothing).
pub fn applied_launch_env() -> &'static [String] {
    APPLIED.get().map(|v| v.as_slice()).unwrap_or(&[])
}

/// Strip agent-session env vars from this process so child shells (and the
/// agents launched in them) start clean. When cmux-app is itself launched
/// from inside a Claude Code session, the inherited CLAUDECODE /
/// CLAUDE_CODE_* vars make nested agents misfile their sessions — this was
/// previously operator guidance in the skill docs; now it's enforced here.
///
/// Returns the names of the vars removed.
pub fn strip_child_env(patterns: &[String]) -> Vec<String> {
    let to_remove: Vec<String> = std::env::vars()
        .map(|(k, _)| k)
        .filter(|k| patterns.iter().any(|p| matches_pattern(k, p)))
        .collect();
    for key in &to_remove {
        std::env::remove_var(key);
    }
    to_remove
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gdk_debug_set_when_unset() {
        assert_eq!(compute_gdk_debug(None), Some("gl-prefer-gl".into()));
        assert_eq!(compute_gdk_debug(Some("")), Some("gl-prefer-gl".into()));
    }

    #[test]
    fn gdk_debug_appends_preserving_existing_flags() {
        assert_eq!(
            compute_gdk_debug(Some("opengl")),
            Some("opengl,gl-prefer-gl".into())
        );
    }

    #[test]
    fn gdk_debug_idempotent_when_already_present() {
        assert_eq!(compute_gdk_debug(Some("gl-prefer-gl")), None);
        assert_eq!(compute_gdk_debug(Some("opengl,gl-prefer-gl")), None);
        assert_eq!(compute_gdk_debug(Some("opengl, gl-prefer-gl")), None);
    }

    #[test]
    fn pattern_exact_match() {
        assert!(matches_pattern("CLAUDECODE", "CLAUDECODE"));
        assert!(!matches_pattern("CLAUDECODE_X", "CLAUDECODE"));
    }

    #[test]
    fn pattern_prefix_glob() {
        assert!(matches_pattern("CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_*"));
        assert!(matches_pattern("CLAUDE_CODE_", "CLAUDE_CODE_*"));
        assert!(!matches_pattern("CLAUDE_EFFORT", "CLAUDE_CODE_*"));
    }

    #[test]
    fn default_patterns_cover_known_vars() {
        let patterns: Vec<String> =
            DEFAULT_STRIP_PATTERNS.iter().map(|s| s.to_string()).collect();
        for var in [
            "CLAUDECODE",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_CODE_CHILD_SESSION",
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_CODE_EXECPATH",
            "CLAUDE_EFFORT",
        ] {
            assert!(
                patterns.iter().any(|p| matches_pattern(var, p)),
                "{var} not covered"
            );
        }
        // Unrelated vars must survive.
        for var in ["HOME", "PATH", "ANTHROPIC_API_KEY", "CLAUDE"] {
            assert!(
                !patterns.iter().any(|p| matches_pattern(var, p)),
                "{var} wrongly stripped"
            );
        }
    }
}
