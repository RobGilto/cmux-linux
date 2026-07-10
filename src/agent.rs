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
///
/// Capture strategies (roadmap 3.5):
/// - Claude: SessionStart hook → `cmux agent report-session` (exact resume).
/// - pi: no id capture; resume falls back to `pi -c` (continue most recent
///   session in the launch cwd) — best effort.
/// - Codex: session ids are printed but not hook-capturable yet; resume uses
///   `codex resume <id>` when an id was reported, else fresh.
/// - Gemini: no session persistence CLI surface today; always fresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
    Gemini,
    Pi,
}

impl Provider {
    pub fn from_str(s: &str) -> Option<Provider> {
        match s.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Provider::Claude),
            "codex" => Some(Provider::Codex),
            "gemini" => Some(Provider::Gemini),
            "pi" => Some(Provider::Pi),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Gemini => "gemini",
            Provider::Pi => "pi",
        }
    }

    /// All known providers, for agent-sessions listings and doctor checks.
    #[allow(dead_code)] // consumed by cmux doctor (Phase 5)
    pub const ALL: [Provider; 4] = [
        Provider::Claude,
        Provider::Codex,
        Provider::Gemini,
        Provider::Pi,
    ];

    /// Whether a captured/implicit session can be resumed after restart.
    pub fn resumable(self) -> bool {
        !matches!(self, Provider::Gemini)
    }

    /// The command that boots this agent. When `resume` is set, boot straight
    /// into that session; otherwise start fresh.
    pub fn launch_command(self, resume: Option<&str>) -> String {
        match (self, resume) {
            (Provider::Claude, Some(id)) => format!("claude --resume {}", id),
            (Provider::Claude, None) => "claude".to_string(),
            (Provider::Codex, Some(id)) => format!("codex resume {}", id),
            (Provider::Codex, None) => "codex".to_string(),
            // Gemini has no resume flag — always fresh.
            (Provider::Gemini, _) => "gemini".to_string(),
            // pi continues the most recent session in this cwd (no id arg).
            (Provider::Pi, Some(_)) => "pi -c".to_string(),
            (Provider::Pi, None) => "pi".to_string(),
        }
    }
}

/// Runtime + persisted state for one agent surface.
#[derive(Debug, Clone)]
pub struct AgentSession {
    pub provider: Provider,
    pub session_id: Option<String>,
    /// Working directory the agent was launched in. Required for resume:
    /// agents like Claude Code key their session store by project directory,
    /// so `claude --resume <id>` only finds the session from that same cwd.
    pub cwd: Option<String>,
}

/// surface UUID (string) -> agent session. Populated when an agent surface is
/// created and updated when its session-id hook reports in.
pub static AGENT_SESSIONS: LazyLock<Mutex<HashMap<String, AgentSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Register (or re-register, on restore) an agent surface.
pub fn register(
    surface_uuid: &str,
    provider: Provider,
    session_id: Option<String>,
    cwd: Option<String>,
) {
    if let Ok(mut m) = AGENT_SESSIONS.lock() {
        m.insert(
            surface_uuid.to_string(),
            AgentSession {
                provider,
                session_id,
                cwd,
            },
        );
    }
}

/// Outcome of a session-id report.
pub enum CaptureResult {
    /// First id captured for this surface — persist it.
    Captured,
    /// Surface already has a captured id; kept it (first-wins).
    AlreadyCaptured,
    /// Surface is not a known agent surface.
    NotAgent,
}

/// Record the native session id captured by the provider's hook.
///
/// First-capture-wins: agents fire SessionStart repeatedly (startup, then
/// again on resume / clear / compact), and only the FIRST id has the
/// conversation history we want to resume — later events mint fresh, empty
/// ids. Once a surface has an id we keep it, so a resumed pane stays pinned
/// to the session that actually holds the conversation.
pub fn set_session_id(surface_uuid: &str, session_id: &str) -> CaptureResult {
    if let Ok(mut m) = AGENT_SESSIONS.lock() {
        if let Some(a) = m.get_mut(surface_uuid) {
            if a.session_id.is_some() {
                return CaptureResult::AlreadyCaptured;
            }
            a.session_id = Some(session_id.to_string());
            return CaptureResult::Captured;
        }
    }
    CaptureResult::NotAgent
}

pub fn get(surface_uuid: &str) -> Option<AgentSession> {
    AGENT_SESSIONS.lock().ok()?.get(surface_uuid).cloned()
}

#[allow(dead_code)] // close-pane integration pending
pub fn remove(surface_uuid: &str) {
    if let Ok(mut m) = AGENT_SESSIONS.lock() {
        m.remove(surface_uuid);
    }
}

/// The startup command for an agent surface: cd into its project directory
/// (so the agent finds its per-project session store on resume), export
/// CMUX_PANE (so the hook can report against this surface), then boot the
/// agent — resuming if we have a captured session id.
pub fn startup_command(surface_uuid: &str, session: &AgentSession) -> String {
    let launch = format!(
        "export CMUX_PANE={}; {}",
        surface_uuid,
        session
            .provider
            .launch_command(session.session_id.as_deref())
    );
    match session.cwd.as_deref().filter(|c| !c.is_empty()) {
        Some(cwd) => format!("cd '{}'; {}", cwd, launch),
        None => launch,
    }
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
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if text.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?
        }
    } else {
        serde_json::json!({})
    };

    if !root.is_object() {
        return Err("~/.claude/settings.json is not a JSON object".into());
    }
    let Some(obj) = root.as_object_mut() else {
        return Err("~/.claude/settings.json is not a JSON object".into());
    };
    let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks.as_object_mut().ok_or("hooks is not an object")?;
    let starts = hooks_obj
        .entry("SessionStart")
        .or_insert_with(|| serde_json::json!([]));
    let arr = starts
        .as_array_mut()
        .ok_or("SessionStart is not an array")?;

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
        let pretty =
            serde_json::to_string_pretty(&root).map_err(|e| format!("serialize settings: {e}"))?;
        std::fs::write(&path, pretty).map_err(|e| format!("write {}: {e}", path.display()))?;
    }

    Ok(vec!["claude".to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_from_str_all_and_aliases() {
        assert_eq!(Provider::from_str("claude"), Some(Provider::Claude));
        assert_eq!(Provider::from_str("claude-code"), Some(Provider::Claude));
        assert_eq!(Provider::from_str("CODEX"), Some(Provider::Codex));
        assert_eq!(Provider::from_str("gemini"), Some(Provider::Gemini));
        assert_eq!(Provider::from_str("pi"), Some(Provider::Pi));
        assert_eq!(Provider::from_str("gpt"), None);
    }

    #[test]
    fn provider_launch_commands() {
        assert_eq!(Provider::Claude.launch_command(None), "claude");
        assert_eq!(
            Provider::Claude.launch_command(Some("abc")),
            "claude --resume abc"
        );
        assert_eq!(
            Provider::Codex.launch_command(Some("s1")),
            "codex resume s1"
        );
        assert_eq!(Provider::Gemini.launch_command(Some("x")), "gemini"); // no resume
        assert_eq!(Provider::Pi.launch_command(Some("x")), "pi -c");
    }

    #[test]
    fn provider_resumability() {
        assert!(Provider::Claude.resumable());
        assert!(Provider::Codex.resumable());
        assert!(Provider::Pi.resumable());
        assert!(!Provider::Gemini.resumable());
    }

    #[test]
    fn startup_command_shape() {
        let s = AgentSession {
            provider: Provider::Claude,
            session_id: Some("sid".into()),
            cwd: Some("/proj".into()),
        };
        let cmd = startup_command("uuid-1", &s);
        assert!(cmd.starts_with("cd '/proj'; "));
        assert!(cmd.contains("export CMUX_PANE=uuid-1;"));
        assert!(cmd.ends_with("claude --resume sid"));
    }

    #[test]
    fn first_capture_wins() {
        register("t-cap", Provider::Claude, None, None);
        assert!(matches!(
            set_session_id("t-cap", "first"),
            CaptureResult::Captured
        ));
        assert!(matches!(
            set_session_id("t-cap", "second"),
            CaptureResult::AlreadyCaptured
        ));
        assert_eq!(
            get("t-cap").and_then(|a| a.session_id).as_deref(),
            Some("first")
        );
        assert!(matches!(
            set_session_id("nope", "x"),
            CaptureResult::NotAgent
        ));
    }
}
