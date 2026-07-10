//! Structured socket error taxonomy (roadmap Phase 2.4).
//!
//! Every error reply on the socket has the shape
//! `{"id": ..., "ok": false, "error": {"code": <code>, "message": <text>}}`.
//! The codes below are the closed vocabulary clients may branch on; the
//! message is human-readable and NOT part of the contract. Handler-specific
//! codes that predate this module (`daemon_error`, `surface_not_found`, …)
//! remain valid — they are narrower refinements of `not_found`/`internal`.

use serde_json::{json, Value};

/// The closed set of top-level error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The request line was not valid JSON.
    ParseError,
    /// The method exists but the params are missing/mistyped.
    InvalidParams,
    /// The referenced workspace/surface/pane does not exist.
    NotFound,
    /// The method is not implemented on this platform.
    NotImplemented,
    /// The handler failed internally (channel closed, handler dropped, …).
    Internal,
    /// The GTK main thread did not answer within the per-request deadline.
    Timeout,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::ParseError => "parse_error",
            ErrorCode::InvalidParams => "invalid_params",
            ErrorCode::NotFound => "not_found",
            ErrorCode::NotImplemented => "not_implemented",
            ErrorCode::Internal => "internal_error",
            ErrorCode::Timeout => "timeout",
        }
    }
}

/// Build a structured error reply.
pub fn error_reply(req_id: Value, code: ErrorCode, message: &str) -> Value {
    json!({
        "id": req_id,
        "ok": false,
        "error": {"code": code.as_str(), "message": message},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_shape_is_stable() {
        let r = error_reply(json!(7), ErrorCode::Timeout, "too slow");
        assert_eq!(r["id"], 7);
        assert_eq!(r["ok"], false);
        assert_eq!(r["error"]["code"], "timeout");
        assert_eq!(r["error"]["message"], "too slow");
    }

    #[test]
    fn codes_are_snake_case_and_unique() {
        let all = [
            ErrorCode::ParseError,
            ErrorCode::InvalidParams,
            ErrorCode::NotFound,
            ErrorCode::NotImplemented,
            ErrorCode::Internal,
            ErrorCode::Timeout,
        ];
        let strs: Vec<_> = all.iter().map(|c| c.as_str()).collect();
        let unique: std::collections::HashSet<_> = strs.iter().collect();
        assert_eq!(unique.len(), strs.len());
        for s in strs {
            assert!(s.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }
}
