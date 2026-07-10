use crate::split_engine::SplitNodeData;
use std::path::{Path, PathBuf};

/// Serializable snapshot of a single workspace for session persistence.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct WorkspaceSession {
    pub uuid: String,
    pub name: String,
    /// UUID of the active pane in this workspace, if any.
    pub active_pane_uuid: Option<String>,
    /// The full pane layout tree for this workspace.
    pub layout: SplitNodeData,
}

/// Root session data written to session.json.
/// `version: 1` allows forward-compatible schema evolution.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct SessionData {
    pub version: u32,
    /// Index of the active workspace in the workspaces array.
    pub active_index: usize,
    pub workspaces: Vec<WorkspaceSession>,
}

/// Returns the session file path.
/// Respects $XDG_DATA_HOME/cmux/session.json; falls back to ~/.local/share/cmux/session.json.
pub fn session_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{home}/.local/share")
    });
    PathBuf::from(base).join("cmux").join("session.json")
}

/// Save session data atomically.
/// Writes to session.json.tmp first, then rename()s to session.json.
/// rename() is atomic on Linux (same filesystem). kill -9 mid-write leaves .tmp only.
pub fn save_session_atomic(data: &SessionData) -> std::io::Result<()> {
    save_session_to(data, &session_path())
}

/// Most recent snapshot produced by AppState::trigger_session_save, kept for
/// the panic hook: the debounce task may not have flushed it to disk yet
/// when the process dies.
static LAST_SNAPSHOT: std::sync::Mutex<Option<SessionData>> =
    std::sync::Mutex::new(None);

/// Record the latest snapshot (called on the GTK main thread on every
/// session mutation, before the debounced disk write).
pub fn remember_snapshot(data: SessionData) {
    *LAST_SNAPSHOT.lock().unwrap_or_else(|p| p.into_inner()) = Some(data);
}

/// Panic-hook path: write the last remembered snapshot to disk.
/// Returns Ok(false) when there is nothing to save.
pub fn save_last_snapshot() -> std::io::Result<bool> {
    let snapshot = LAST_SNAPSHOT
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    match snapshot {
        Some(data) => save_session_atomic(&data).map(|()| true),
        None => Ok(false),
    }
}

/// Delete the saved session so the next launch starts clean (the `--fresh` flag).
/// Removes both session.json and any leftover session.json.tmp. Missing files are
/// not an error -- a wipe of nothing is still a successful wipe.
pub fn wipe_session() -> std::io::Result<()> {
    wipe_session_at(&session_path())
}

/// Internal: wipe a specific path (used in tests with temp paths).
pub fn wipe_session_at(path: &Path) -> std::io::Result<()> {
    let tmp_path = path.with_extension("json.tmp");
    for p in [path, tmp_path.as_path()] {
        match std::fs::remove_file(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Internal: save to a specific path (used in tests with temp paths).
pub fn save_session_to(data: &SessionData, path: &Path) -> std::io::Result<()> {
    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&tmp_path, json.as_bytes())?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Load session from disk. Returns None if the file is missing, empty, or invalid JSON.
/// Never panics -- always returns a usable result for graceful fallback (SESS-04).
pub fn load_session() -> Option<SessionData> {
    load_session_from(&session_path())
}

/// Internal: load from a specific path (used in tests with temp paths).
pub fn load_session_from(path: &Path) -> Option<SessionData> {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!("cmux: no session file at {}", path.display());
            return None;
        }
        Err(e) => {
            tracing::warn!("cmux: session file read error: {e}");
            return None;
        }
    };
    match serde_json::from_str::<SessionData>(&content) {
        Ok(data) => {
            if data.version != 1 && data.version != 2 {
                tracing::debug!("cmux: session version {} not supported, ignoring", data.version);
                return None;
            }
            Some(data)
        }
        Err(e) => {
            tracing::warn!("cmux: session JSON invalid: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split_engine::SplitNodeData;

    fn dummy_session(name: &str) -> SessionData {
        SessionData {
            version: 1,
            active_index: 0,
            workspaces: vec![WorkspaceSession {
                uuid: "test-uuid-1".to_string(),
                name: name.to_string(),
                active_pane_uuid: None,
                layout: SplitNodeData::Leaf {
                    pane_id: 1000,
                    surface_uuid: uuid::Uuid::nil(),
                    shell: "/bin/sh".to_string(),
                    cwd: "/tmp".to_string(),
                    agent_provider: None,
                    agent_session_id: None,
                },
            }],
        }
    }

    /// SESS-01: save_session_to must write session.json to disk for valid data.
    /// Verifies the full trigger -> write path, not just Ok(()) return.
    #[test]
    fn test_save_triggered() {
        let dir = std::env::temp_dir().join(format!("cmux-test-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        let data = dummy_session("TestWorkspace");
        let result = save_session_to(&data, &path);
        assert!(result.is_ok(), "save_session_to failed: {:?}", result);
        // The file must exist on disk -- not just Ok(()), but actually written.
        assert!(path.exists(), "session.json not created on disk after save_session_to");
        // The content must be valid JSON with the correct workspace name.
        let content = std::fs::read_to_string(&path).expect("could not read session.json");
        let parsed: SessionData = serde_json::from_str(&content)
            .expect("session.json is not valid JSON");
        assert_eq!(parsed.workspaces[0].name, "TestWorkspace");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SESS-02: Full roundtrip -- save then load must reproduce the workspace name.
    #[test]
    fn test_restore_roundtrip() {
        let dir = std::env::temp_dir().join(format!("cmux-test-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");

        let data = dummy_session("MyWorkspace");
        save_session_to(&data, &path).expect("save failed");

        let loaded = load_session_from(&path).expect("load returned None");
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.workspaces.len(), 1);
        assert_eq!(loaded.workspaces[0].name, "MyWorkspace");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SESS-03: Atomic write -- the .tmp file is gone after a successful rename.
    #[test]
    fn test_atomic_write() {
        let dir = std::env::temp_dir().join(format!("cmux-test-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        let tmp_path = path.with_extension("json.tmp");

        let data = dummy_session("AtomicTest");
        save_session_to(&data, &path).unwrap();

        // After successful save: session.json exists, .tmp must be gone (renamed).
        assert!(path.exists(), "session.json must exist after save");
        assert!(!tmp_path.exists(), "session.json.tmp must be gone after successful rename");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SESS-04: load_session returns None for missing file without panic.
    #[test]
    fn test_graceful_fallback() {
        let path = std::path::PathBuf::from("/tmp/cmux-nonexistent-session-xyz.json");
        let result = load_session_from(&path);
        assert!(result.is_none(), "load_session_from must return None for missing file");
    }

    /// SESS-05: wipe_session_at removes session.json (+ .tmp) and load then returns None.
    #[test]
    fn test_wipe_removes_session() {
        let dir = std::env::temp_dir().join(format!("cmux-test-wipe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        let tmp_path = path.with_extension("json.tmp");
        let data = SessionData { version: 1, active_index: 0, workspaces: vec![] };
        save_session_to(&data, &path).unwrap();
        std::fs::write(&tmp_path, b"leftover").unwrap();

        wipe_session_at(&path).expect("wipe failed");

        assert!(!path.exists(), "session.json must be gone after wipe");
        assert!(!tmp_path.exists(), "session.json.tmp must be gone after wipe");
        assert!(load_session_from(&path).is_none(), "load after wipe must return None");
    }

    /// SESS-06: wiping a missing session is a no-op success (idempotent).
    #[test]
    fn test_wipe_missing_is_ok() {
        let path = std::path::PathBuf::from("/tmp/cmux-nonexistent-wipe-xyz.json");
        assert!(wipe_session_at(&path).is_ok(), "wipe of missing file must succeed");
    }
}
