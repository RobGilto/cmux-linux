//! Synchronous Unix socket JSON-RPC client for the cmux CLI.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Errors from CLI socket operations.
#[derive(Debug)]
pub enum CliError {
    /// Could not connect to the socket.
    ConnectionError(String),
    /// The server returned an error response.
    CommandError(String),
    /// Unexpected protocol-level error (malformed response, timeout, etc).
    ProtocolError(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::ConnectionError(msg) => write!(f, "{}", msg),
            CliError::CommandError(msg) => write!(f, "{}", msg),
            CliError::ProtocolError(msg) => write!(f, "{}", msg),
        }
    }
}

/// A synchronous Unix socket JSON-RPC client.
pub struct SocketClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl SocketClient {
    /// Connect to the cmux socket at the given path with the specified timeout.
    pub fn connect(path: &str, timeout: Duration) -> Result<Self, CliError> {
        let stream = UnixStream::connect(path)
            .map_err(|e| CliError::ConnectionError(format!("cannot connect to {}: {}", path, e)))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| CliError::ConnectionError(format!("set_read_timeout: {}", e)))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|e| CliError::ConnectionError(format!("set_write_timeout: {}", e)))?;
        let writer = stream
            .try_clone()
            .map_err(|e| CliError::ConnectionError(format!("clone stream: {}", e)))?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
            next_id: 1,
        })
    }

    /// Send a JSON-RPC call and return the result value.
    ///
    /// On success (ok: true), returns the `result` field.
    /// On error (ok: false), returns `Err(CliError::CommandError(...))`.
    pub fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, CliError> {
        let id = self.next_id;
        self.next_id += 1;

        let request = serde_json::json!({
            "id": id,
            "method": method,
            "params": params,
        });

        let mut line = request.to_string();
        line.push('\n');

        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| CliError::ProtocolError(format!("write failed: {}", e)))?;

        let mut response_line = String::new();
        self.reader
            .read_line(&mut response_line)
            .map_err(|e| CliError::ProtocolError(format!("read failed: {}", e)))?;

        if response_line.is_empty() {
            return Err(CliError::ProtocolError("empty response from server".into()));
        }

        let resp: serde_json::Value = serde_json::from_str(&response_line)
            .map_err(|e| CliError::ProtocolError(format!("invalid JSON response: {}", e)))?;

        let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if ok {
            Ok(resp
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null))
        } else {
            let msg = resp
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            Err(CliError::CommandError(msg.to_string()))
        }
    }

    /// Subscribe to the event stream and invoke `on_line` for each event
    /// line until the server closes the connection (e.g. `limit` reached)
    /// or `on_line` returns false. Clears the read timeout: an event stream
    /// legitimately idles between heartbeats.
    pub fn subscribe(
        &mut self,
        params: serde_json::Value,
        mut on_line: impl FnMut(&str) -> bool,
    ) -> Result<(), CliError> {
        self.reader
            .get_ref()
            .set_read_timeout(None)
            .map_err(|e| CliError::ProtocolError(format!("clear read timeout: {}", e)))?;

        let request = serde_json::json!({
            "id": self.next_id,
            "method": "events.subscribe",
            "params": params,
        });
        self.next_id += 1;
        let mut line = request.to_string();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| CliError::ProtocolError(format!("write failed: {}", e)))?;

        // First line is the subscription ack.
        let mut ack = String::new();
        self.reader
            .read_line(&mut ack)
            .map_err(|e| CliError::ProtocolError(format!("read failed: {}", e)))?;
        let ack_v: serde_json::Value = serde_json::from_str(&ack)
            .map_err(|e| CliError::ProtocolError(format!("invalid ack: {}", e)))?;
        if !ack_v.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Err(CliError::CommandError("subscribe rejected".into()));
        }

        loop {
            let mut event_line = String::new();
            let n = self
                .reader
                .read_line(&mut event_line)
                .map_err(|e| CliError::ProtocolError(format!("read failed: {}", e)))?;
            if n == 0 {
                return Ok(()); // server closed (limit reached or shutdown)
            }
            if !on_line(event_line.trim_end()) {
                return Ok(());
            }
        }
    }
}
