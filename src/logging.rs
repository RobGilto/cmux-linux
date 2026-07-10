//! Structured logging + crash safety (roadmap Phase 2).
//!
//! One `tracing` subscriber with two layers:
//! - console (stderr), filtered by `CMUX_LOG` (default `info`) — quiet by
//!   default; `CMUX_LOG=debug` re-enables the historical diagnostic firehose
//!   (per-frame GL render logs and friends).
//! - daily-rotated file at `$XDG_STATE_HOME/cmux/logs/cmux.log.YYYY-MM-DD`,
//!   same filter — the first place to look after a blank window or crash.
//!
//! Also installs a panic hook that logs the panic (with backtrace) and
//! best-effort saves the last session snapshot, so a crash never loses the
//! workspace topology.

use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::Layer as _;

/// Directory for log files: `$XDG_STATE_HOME/cmux/logs` (fallback
/// `~/.local/state/cmux/logs`).
pub fn log_dir() -> std::path::PathBuf {
    std::env::var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".local/state")
        })
        .join("cmux")
        .join("logs")
}

/// Initialize the global subscriber. Returns the appender guard — hold it
/// for the app's lifetime or buffered file output is lost on exit.
pub fn init() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = || {
        tracing_subscriber::EnvFilter::try_from_env("CMUX_LOG")
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };

    let console = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_filter(filter());

    let dir = log_dir();
    let guard = match std::fs::create_dir_all(&dir) {
        Ok(()) => {
            let appender = tracing_appender::rolling::daily(&dir, "cmux.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let file = tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false)
                .with_target(false)
                .with_filter(filter());
            tracing_subscriber::registry()
                .with(console)
                .with(file)
                .init();
            Some(guard)
        }
        Err(e) => {
            tracing_subscriber::registry().with(console).init();
            tracing::warn!("log dir {} unavailable: {e}; console only", dir.display());
            None
        }
    };

    install_panic_hook();
    guard
}

/// Log panics with backtrace and save the last session snapshot before the
/// process dies. Chains to the previous hook (which prints to stderr).
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("PANIC: {info}\nbacktrace:\n{backtrace}");
        match crate::session::save_last_snapshot() {
            Ok(true) => tracing::error!("panic hook: session snapshot saved"),
            Ok(false) => {}
            Err(e) => tracing::error!("panic hook: session save failed: {e}"),
        }
        previous(info);
    }));
}

/// Startup banner: the facts needed to debug a blank window, in one place.
pub fn startup_banner() {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        gdk_backend = std::env::var("GDK_BACKEND").as_deref().unwrap_or("(unset)"),
        gdk_debug = std::env::var("GDK_DEBUG").as_deref().unwrap_or("(unset)"),
        session = std::env::var("XDG_SESSION_TYPE").as_deref().unwrap_or("(unset)"),
        nvidia = crate::platform::is_nvidia(),
        launch_env = ?crate::platform::applied_launch_env(),
        "cmux-app starting"
    );
}
