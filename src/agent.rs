// src/agent.rs — native agent sessions (provider-aware surfaces + resume).
//
// An "agent surface" is a terminal cmux knows is running a coding agent
// (Claude Code, etc.) rather than a bare shell. cmux tracks the provider and
// the agent's native session id per surface, so on restart it can relaunch
// the agent with its resume flag instead of a cold shell.
//
// Session ids are captured out-of-band: `cmux hooks setup` installs a
// SessionStart hook into the agent CLI's config that runs
// `cmux agent report-session`, which reports (surface, provider, session_id)
// back over the socket. The surface is identified by the CMUX_PANE env var
// cmux sets when it launches the agent.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// Provider-specific knowledge: how to launch fresh, how to resume, and how
/// its session-capture hook is wired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
}

impl Provider {
    pub fn from_str(s: &str) -> Option<Provider> {
        match s.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Provider::Claude),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
        }
    }

    /// The command that boots this agent. When `resume` is set, boot straight
    /// into that session; otherwise start fresh.
    pub fn launch_command(self, resume: Option<&str>) -> String {
        match self {
            Provider::Claude => match resume {
                Some(id) => format!("claude --resume {}", id),
                None => "claude".to_string(),
            },
        }
    }
}

/// Runtime + persisted state for one agent surface.
#[derive(Debug, Clone)]
pub struct AgentSession {
    pub provider: Provider,
    pub session_id: Option<String>,
}

/// surface UUID (string) -> agent session. Populated when an agent surface is
/// created and updated when its session-id hook reports in.
pub static AGENT_SESSIONS: LazyLock<Mutex<HashMap<String, AgentSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Register (or re-register, on restore) an agent surface.
pub fn register(surface_uuid: &str, provider: Provider, session_id: Option<String>) {
    if let Ok(mut m) = AGENT_SESSIONS.lock() {
        m.insert(surface_uuid.to_string(), AgentSession { provider, session_id });
    }
}

/// Record the native session id captured by the provider's hook.
/// Returns true if the surface was a known agent surface.
pub fn set_session_id(surface_uuid: &str, session_id: &str) -> bool {
    if let Ok(mut m) = AGENT_SESSIONS.lock() {
        if let Some(a) = m.get_mut(surface_uuid) {
            a.session_id = Some(session_id.to_string());
            return true;
        }
    }
    false
}

pub fn get(surface_uuid: &str) -> Option<AgentSession> {
    AGENT_SESSIONS.lock().ok()?.get(surface_uuid).cloned()
}

pub fn remove(surface_uuid: &str) {
    if let Ok(mut m) = AGENT_SESSIONS.lock() {
        m.remove(surface_uuid);
    }
}

/// The startup command for an agent surface: export CMUX_PANE (so the hook
/// can report against this surface) then boot the agent, resuming if we have
/// a captured session id.
pub fn startup_command(surface_uuid: &str, session: &AgentSession) -> String {
    format!(
        "export CMUX_PANE={}; {}",
        surface_uuid,
        session.provider.launch_command(session.session_id.as_deref())
    )
}

/// Install the session-capture hook for Claude Code into
/// ~/.claude/settings.json. Idempotent: leaves an existing cmux hook in place.
/// Returns the list of providers whose hooks are now installed.
pub fn install_hooks() -> Result<Vec<String>, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let dir = std::path::Path::new(&home).join(".claude");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir ~/.claude: {e}"))?;
    let path = dir.join("settings.json");

    let mut root: serde_json::Value = if path.exists() {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if text.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&text)
                .map_err(|e| format!("parse {}: {e}", path.display()))?
        }
    } else {
        serde_json::json!({})
    };

    if !root.is_object() {
        return Err("~/.claude/settings.json is not a JSON object".into());
    }
    let obj = root.as_object_mut().unwrap();
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or("hooks is not an object")?;
    let starts = hooks_obj
        .entry("SessionStart")
        .or_insert_with(|| serde_json::json!([]));
    let arr = starts.as_array_mut().ok_or("SessionStart is not an array")?;

    // Already installed? Look for our command anywhere in the matcher groups.
    let already = arr.iter().any(|group| {
        group
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|hs| {
                hs.iter().any(|h| {
                    h.get("command")
                        .and_then(|c| c.as_str())
                        .map(|c| c.contains("cmux agent report-session"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    });

    if !already {
        arr.push(serde_json::json!({
            "hooks": [
                {"type": "command", "command": "cmux agent report-session"}
            ]
        }));
        let pretty = serde_json::to_string_pretty(&root)
            .map_err(|e| format!("serialize settings: {e}"))?;
        std::fs::write(&path, pretty)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
    }

    Ok(vec!["claude".to_string()])
}
