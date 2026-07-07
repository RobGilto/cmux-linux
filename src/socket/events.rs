// src/socket/events.rs — broadcast event bus for `cmux events` subscribers.
//
// Emitters (GTK main thread handlers, the bell dispatcher, etc.) call
// `emit(name, data)`; every socket connection that sent `events.subscribe`
// forwards matching events to its client as newline-delimited JSON. A
// lagging subscriber only loses events past the channel capacity — the
// stream carries monotonically increasing `seq` numbers so clients can
// detect gaps.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

/// Capacity of the broadcast ring buffer. Subscribers that fall more than
/// this many events behind will observe a gap in `seq`.
const EVENT_BUS_CAPACITY: usize = 256;

static EVENT_SEQ: AtomicU64 = AtomicU64::new(1);

pub static EVENT_BUS: LazyLock<tokio::sync::broadcast::Sender<String>> =
    LazyLock::new(|| tokio::sync::broadcast::channel(EVENT_BUS_CAPACITY).0);

/// Emit an event to all subscribers. Cheap when nobody is subscribed.
/// `name` is the dotted event name (e.g. "notification.created",
/// "surface.bell"); `data` is the event payload.
pub fn emit(name: &str, data: serde_json::Value) {
    let seq = EVENT_SEQ.fetch_add(1, Ordering::SeqCst);
    let line = serde_json::json!({
        "event": name,
        "seq": seq,
        "ts_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        "data": data,
    })
    .to_string();
    // send() only errors when there are no receivers — fine to ignore.
    let _ = EVENT_BUS.send(line);
}
