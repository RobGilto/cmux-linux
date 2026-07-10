//! Named rendezvous points for orchestrator↔worker synchronization
//! (roadmap Phase 3.2). macOS-cmux parity for `cmux wait-for`.
//!
//! Semantics:
//! - `wait(name, timeout)` blocks until `signal(name)` or the deadline.
//! - Waiters present at signal time ALL release.
//! - A signal with no waiters is latched: the next single `wait` consumes it
//!   and returns immediately ("signal-before-wait is remembered").
//!
//! Runs entirely on the tokio side — no GTK involvement — so a wedged main
//! thread cannot break fleet rendezvous.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

struct Entry {
    latched: bool,
    notify: Arc<tokio::sync::Notify>,
}

static REGISTRY: LazyLock<Mutex<HashMap<String, Entry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn lock() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    REGISTRY.lock().unwrap_or_else(|p| p.into_inner())
}

/// Signal a rendezvous point: release all current waiters, or latch if none.
/// (We latch AND notify unconditionally; a released waiter consumes the
/// latch, and with no waiters the latch persists for the next `wait`.)
pub fn signal(name: &str) {
    let notify = {
        let mut reg = lock();
        let entry = reg.entry(name.to_string()).or_insert_with(|| Entry {
            latched: false,
            notify: Arc::new(tokio::sync::Notify::new()),
        });
        entry.latched = true;
        entry.notify.clone()
    };
    notify.notify_waiters();
}

/// Wait for a rendezvous point. Returns Ok(()) when signalled, Err(()) on
/// timeout.
pub async fn wait(name: &str, timeout: std::time::Duration) -> Result<(), ()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        // Fast path / latch consumption.
        let notify = {
            let mut reg = lock();
            let entry = reg.entry(name.to_string()).or_insert_with(|| Entry {
                latched: false,
                notify: Arc::new(tokio::sync::Notify::new()),
            });
            if entry.latched {
                entry.latched = false;
                return Ok(());
            }
            entry.notify.clone()
        };

        // Register interest BEFORE re-checking, so a signal that fires
        // between the check above and the await below is not lost:
        // signal() latches first, so the re-check (next loop iteration
        // after notify) always observes it.
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        // Re-check the latch now that interest is registered — closes the
        // check-then-wait race.
        {
            let mut reg = lock();
            if let Some(entry) = reg.get_mut(name) {
                if entry.latched {
                    entry.latched = false;
                    return Ok(());
                }
            }
        }

        match tokio::time::timeout_at(deadline, notified).await {
            Ok(()) => {
                // Woken by signal(); consume the latch if we're first.
                let mut reg = lock();
                if let Some(entry) = reg.get_mut(name) {
                    entry.latched = false;
                }
                return Ok(());
            }
            Err(_) => return Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn signal_before_wait_is_latched() {
        signal("t-latch");
        assert!(wait("t-latch", Duration::from_millis(50)).await.is_ok());
        // Latch consumed — second wait times out.
        assert!(wait("t-latch", Duration::from_millis(50)).await.is_err());
    }

    #[tokio::test]
    async fn wait_then_signal_releases() {
        let waiter = tokio::spawn(wait("t-release", Duration::from_secs(5)));
        tokio::time::sleep(Duration::from_millis(50)).await;
        signal("t-release");
        assert!(waiter.await.expect("join").is_ok());
    }

    #[tokio::test]
    async fn multiple_waiters_all_release() {
        let w1 = tokio::spawn(wait("t-multi", Duration::from_secs(5)));
        let w2 = tokio::spawn(wait("t-multi", Duration::from_secs(5)));
        let w3 = tokio::spawn(wait("t-multi", Duration::from_secs(5)));
        tokio::time::sleep(Duration::from_millis(50)).await;
        signal("t-multi");
        assert!(w1.await.expect("join").is_ok());
        assert!(w2.await.expect("join").is_ok());
        assert!(w3.await.expect("join").is_ok());
    }

    #[tokio::test]
    async fn timeout_returns_err() {
        assert!(wait("t-timeout", Duration::from_millis(50)).await.is_err());
    }
}
