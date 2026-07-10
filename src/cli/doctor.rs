//! `cmux doctor` — self-service diagnostics (roadmap Phase 5.1).
//!
//! Every check reports pass/warn/fail plus a one-line remedy. Exit code is
//! non-zero when any check fails, so scripts can gate on it. `--json` emits
//! machine-readable results for agents.

use serde_json::json;

pub struct Check {
    pub name: &'static str,
    pub status: &'static str, // "pass" | "warn" | "fail"
    pub detail: String,
    pub remedy: &'static str,
}

fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|d| d.join(bin))
        .find(|c| c.is_file())
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

pub fn run_doctor(cli_socket: &Option<String>, json_mode: bool) -> Result<(), super::CliError> {
    let mut checks: Vec<Check> = Vec::new();

    // 1+2+3. Socket → ping → identify (version match, GL, launch env).
    let socket = cli_socket
        .clone()
        .or_else(super::discovery::discover_socket);
    let mut identify: Option<serde_json::Value> = None;
    match socket {
        Some(ref path) => {
            match super::socket_client::SocketClient::connect(
                path,
                std::time::Duration::from_secs(3),
            ) {
                Ok(mut c) => match c.call("system.identify", json!({})) {
                    Ok(v) => {
                        checks.push(Check {
                            name: "socket",
                            status: "pass",
                            detail: format!("answering at {path}"),
                            remedy: "",
                        });
                        identify = Some(v);
                    }
                    Err(e) => checks.push(Check {
                        name: "socket",
                        status: "fail",
                        detail: format!("{path} connected but identify failed: {e}"),
                        remedy: "restart the app: pkill cmux-app && cmux launch",
                    }),
                },
                Err(e) => checks.push(Check {
                    name: "socket",
                    status: "fail",
                    detail: format!("cannot connect to {path}: {e}"),
                    remedy: "start the app: cmux launch",
                }),
            }
        }
        None => checks.push(Check {
            name: "socket",
            status: "fail",
            detail: "no socket found".into(),
            remedy: "start the app: cmux launch",
        }),
    }

    if let Some(ref id) = identify {
        let app_ver = id.get("version").and_then(|v| v.as_str()).unwrap_or("?");
        let cli_ver = env!("CARGO_PKG_VERSION");
        checks.push(Check {
            name: "version-match",
            status: if app_ver == cli_ver { "pass" } else { "warn" },
            detail: format!("app {app_ver}, cli {cli_ver}"),
            remedy: "rebuild both binaries from the same checkout",
        });

        let gl = id.get("gl").and_then(|v| v.as_str());
        let launch_env = id
            .get("launch_env")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        match gl {
            Some(g) if g.starts_with("ERROR") => checks.push(Check {
                name: "gl-context",
                status: "fail",
                detail: g.to_string(),
                remedy: "check CMUX_LOG=debug logs; verify [launch] gl_workaround config",
            }),
            Some(g) => checks.push(Check {
                name: "gl-context",
                status: "pass",
                detail: format!("{g} ({launch_env} launch-env auto-fix(es) applied)"),
                remedy: "",
            }),
            None => checks.push(Check {
                name: "gl-context",
                status: "warn",
                detail: "no GLArea realized yet".into(),
                remedy: "open a workspace, then re-run doctor",
            }),
        }
    }

    // 4. config.toml parses.
    let config_path = format!("{}/.config/cmux/config.toml", home());
    match std::fs::read_to_string(&config_path) {
        Ok(text) => match toml::from_str::<toml::Value>(&text) {
            Ok(_) => checks.push(Check {
                name: "config",
                status: "pass",
                detail: format!("{config_path} valid"),
                remedy: "",
            }),
            Err(e) => checks.push(Check {
                name: "config",
                status: "fail",
                detail: format!("{config_path}: {e}"),
                remedy: "fix the TOML syntax error above",
            }),
        },
        Err(_) => checks.push(Check {
            name: "config",
            status: "pass",
            detail: "no config.toml (defaults in effect)".into(),
            remedy: "",
        }),
    }

    // 5. session.json valid.
    let session_path = std::env::var("XDG_DATA_HOME")
        .map(|d| format!("{d}/cmux/session.json"))
        .unwrap_or_else(|_| format!("{}/.local/share/cmux/session.json", home()));
    match std::fs::read_to_string(&session_path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(_) => checks.push(Check {
                name: "session-file",
                status: "pass",
                detail: format!("{session_path} valid"),
                remedy: "",
            }),
            Err(e) => checks.push(Check {
                name: "session-file",
                status: "fail",
                detail: format!("{session_path}: {e}"),
                remedy: "wipe it with: cmux launch --fresh",
            }),
        },
        Err(_) => checks.push(Check {
            name: "session-file",
            status: "pass",
            detail: "no saved session (fresh start)".into(),
            remedy: "",
        }),
    }

    // 6. Agent provider CLIs on PATH (informational — warn, never fail).
    for provider in ["claude", "codex", "gemini", "pi"] {
        match which(provider) {
            Some(p) => checks.push(Check {
                name: match provider {
                    "claude" => "provider-claude",
                    "codex" => "provider-codex",
                    "gemini" => "provider-gemini",
                    _ => "provider-pi",
                },
                status: "pass",
                detail: p.display().to_string(),
                remedy: "",
            }),
            None => checks.push(Check {
                name: match provider {
                    "claude" => "provider-claude",
                    "codex" => "provider-codex",
                    "gemini" => "provider-gemini",
                    _ => "provider-pi",
                },
                status: "warn",
                detail: "not on PATH".into(),
                remedy: "install it if you plan to orchestrate this agent",
            }),
        }
    }

    // 7. Claude session-capture hook installed.
    let settings = format!("{}/.claude/settings.json", home());
    let hook_installed = std::fs::read_to_string(&settings)
        .map(|t| t.contains("cmux agent report-session"))
        .unwrap_or(false);
    checks.push(Check {
        name: "resume-hook",
        status: if hook_installed { "pass" } else { "warn" },
        detail: if hook_installed {
            "SessionStart hook present".into()
        } else {
            "no cmux hook in ~/.claude/settings.json".into()
        },
        remedy: "run once: cmux hooks setup (enables agent resume-on-restart)",
    });

    // 8. Log dir writable.
    let log_dir = std::env::var("XDG_STATE_HOME")
        .map(|d| format!("{d}/cmux/logs"))
        .unwrap_or_else(|_| format!("{}/.local/state/cmux/logs", home()));
    let writable = std::fs::create_dir_all(&log_dir)
        .and_then(|()| std::fs::write(format!("{log_dir}/.doctor-probe"), b"ok"))
        .and_then(|()| std::fs::remove_file(format!("{log_dir}/.doctor-probe")));
    checks.push(Check {
        name: "log-dir",
        status: if writable.is_ok() { "pass" } else { "fail" },
        detail: log_dir.clone(),
        remedy: "fix permissions on the log directory",
    });

    // Report.
    let failed = checks.iter().filter(|c| c.status == "fail").count();
    if json_mode {
        let rows: Vec<serde_json::Value> = checks
            .iter()
            .map(|c| {
                json!({
                    "name": c.name, "status": c.status,
                    "detail": c.detail, "remedy": c.remedy,
                })
            })
            .collect();
        println!(
            "{}",
            json!({"checks": rows, "failed": failed, "ok": failed == 0})
        );
    } else {
        for c in &checks {
            let icon = match c.status {
                "pass" => "✓",
                "warn" => "!",
                _ => "✗",
            };
            let remedy = if c.remedy.is_empty() || c.status == "pass" {
                String::new()
            } else {
                format!("  → {}", c.remedy)
            };
            println!("{icon} {:<16} {}{remedy}", c.name, c.detail);
        }
        println!(
            "\n{} checks, {} failed{}",
            checks.len(),
            failed,
            if failed == 0 { " — all good" } else { "" }
        );
    }

    if failed > 0 {
        Err(super::CliError::CommandError(format!(
            "{failed} doctor check(s) failed"
        )))
    } else {
        Ok(())
    }
}
