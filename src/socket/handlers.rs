// src/socket/handlers.rs — GTK main thread command dispatch

use crate::socket::commands::SocketCommand;
use gtk4::prelude::*;
use serde_json::{json, Value};

/// Build a success response with the given result payload.
fn ok(req_id: Value, result: Value) -> Value {
    json!({"id": req_id, "ok": true, "result": result})
}

/// Build an error response.
fn err(req_id: Value, code: &str, message: &str) -> Value {
    json!({"id": req_id, "ok": false, "error": {"code": code, "message": message}})
}

/// Resolve a surface_ref string ("surface:N" or UUID) to a UUID string.
/// Returns Ok(uuid_string) or Err((error_message, available_refs)).
fn resolve_surface_ref(
    surface_ref: &str,
    refs: &std::collections::HashMap<u32, String>,
) -> Result<String, (String, Vec<String>)> {
    if let Some(n_str) = surface_ref.strip_prefix("surface:") {
        if let Ok(n) = n_str.parse::<u32>() {
            if let Some(uuid) = refs.get(&n) {
                return Ok(uuid.clone());
            }
            let available: Vec<String> = refs.keys().map(|k| format!("surface:{}", k)).collect();
            return Err((format!("surface:{} not found", n), available));
        }
    }
    // Treat as UUID directly
    Ok(surface_ref.to_string())
}

/// Resolve the live ghostty surface pointer for a socket command target.
///
/// The split tree's `Leaf.surface` is a null placeholder that is never
/// backfilled after GLArea realize (`set_initial_surface`/`update_surface`
/// have no callers), so looking there always yields null and input commands
/// silently no-op. The authoritative pointer lives in SURFACE_REGISTRY
/// (ptr → pane_id, inserted at realize, removed at unrealize/free); reverse
/// it via the target's pane_id. UUID targets are searched across every
/// workspace engine so agents can drive non-active workspaces; the no-id
/// default stays the active pane of the active workspace.
fn resolve_surface_ptr(
    s: &crate::app_state::AppState,
    id: Option<&String>,
) -> Option<crate::ghostty::ffi::ghostty_surface_t> {
    let pane_id = match id {
        Some(uuid) => s
            .split_engines
            .iter()
            .find_map(|e| e.find_pane_id_by_uuid(uuid)),
        None => s.split_engines.get(s.active_index).and_then(|e| {
            // "active-pane" is a GUI-focus CSS class; a fresh workspace has
            // no focused pane until clicked, so fall back to the first leaf.
            e.root.find_active_pane_id().or_else(|| {
                let mut ids = Vec::new();
                e.root.collect_pane_ids(&mut ids);
                ids.first().copied()
            })
        }),
    }?;
    let reg = crate::ghostty::callbacks::SURFACE_REGISTRY.lock().ok()?;
    reg.iter()
        .find(|(_, &pid)| pid == pane_id)
        .map(|(&ptr, _)| ptr as crate::ghostty::ffi::ghostty_surface_t)
}

/// Translate a named key (macOS cmux `send-key` vocabulary) into the byte
/// sequence to feed the terminal. Single printable characters pass through.
/// `ctrl+<letter>` maps to the control byte (ctrl+c → 0x03).
fn key_to_bytes(key: &str) -> Option<Vec<u8>> {
    let bytes: &[u8] = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => b"\r",
        "tab" => b"\t",
        "escape" | "esc" => b"\x1b",
        "backspace" => b"\x7f",
        "space" => b" ",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        "home" => b"\x1b[H",
        "end" => b"\x1b[F",
        "pageup" => b"\x1b[5~",
        "pagedown" => b"\x1b[6~",
        "delete" => b"\x1b[3~",
        k if k.starts_with("ctrl+") && k.len() == 6 => {
            let c = k.as_bytes()[5];
            if c.is_ascii_lowercase() {
                return Some(vec![c & 0x1f]);
            }
            return None;
        }
        _ => {
            // Single printable char (any case) passes through as-is.
            if key.chars().count() == 1 {
                return Some(key.as_bytes().to_vec());
            }
            return None;
        }
    };
    Some(bytes.to_vec())
}

/// Feed bytes to a surface as committed typed text (not a paste): newlines
/// are normalized to carriage returns and bracketed paste is not used, so
/// shells execute them like real keystrokes.
unsafe fn surface_type_bytes(surf: crate::ghostty::ffi::ghostty_surface_t, bytes: &[u8]) {
    crate::ghostty::ffi::ghostty_surface_text_input(
        surf,
        bytes.as_ptr() as *const std::os::raw::c_char,
        bytes.len(),
    );
}

/// Translate a declarative layout spec into the session tree format plus the
/// startup command (if any) for each terminal, keyed by its generated UUID.
///
/// User schema:
///   {"type": "terminal", "cwd": "/path", "command": "htop"}      — all optional
///   {"type": "split", "direction": "horizontal"|"vertical",
///    "ratio": 0.5, "start": {...}, "end": {...}}
///
/// `default_cwd` applies to terminals that don't set their own. cwd is
/// delivered as a leading `cd` in the startup command (the surface config
/// path for working directories isn't plumbed on Linux yet).
fn layout_to_data(
    spec: &Value,
    default_cwd: Option<&str>,
    depth: u32,
    startups: &mut Vec<(String, String)>,
) -> Result<crate::split_engine::SplitNodeData, String> {
    use crate::split_engine::SplitNodeData;
    if depth > 16 {
        return Err("layout tree deeper than 16 levels".into());
    }
    let ty = spec
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "layout node missing \"type\"".to_string())?;
    match ty {
        "terminal" => {
            let uuid = uuid::Uuid::new_v4();
            let cwd = spec
                .get("cwd")
                .and_then(|v| v.as_str())
                .or(default_cwd)
                .unwrap_or("");
            // An "agent" terminal boots a provider-aware session: register it
            // and let the agent's launch command stand in for `command`.
            let agent_provider = spec
                .get("agent")
                .and_then(|v| v.as_str())
                .and_then(crate::agent::Provider::from_str);
            let command = if let Some(p) = agent_provider {
                let resume_cwd = if cwd.is_empty() {
                    None
                } else {
                    Some(cwd.to_string())
                };
                crate::agent::register(&uuid.to_string(), p, None, resume_cwd.clone());
                // startup_command handles cd (into cwd) + CMUX_PANE + launch.
                let session = crate::agent::AgentSession {
                    provider: p,
                    session_id: None,
                    cwd: resume_cwd,
                };
                crate::agent::startup_command(&uuid.to_string(), &session)
            } else {
                let cmd = spec.get("command").and_then(|v| v.as_str()).unwrap_or("");
                match (cwd.is_empty(), cmd.is_empty()) {
                    (false, false) => format!("cd '{}' && {}", cwd, cmd),
                    (false, true) => format!("cd '{}'", cwd),
                    (true, false) => cmd.to_string(),
                    (true, true) => String::new(),
                }
            };
            if !command.is_empty() {
                startups.push((uuid.to_string(), command));
            }
            Ok(SplitNodeData::Leaf {
                pane_id: 0, // regenerated by from_data
                surface_uuid: uuid,
                shell: String::new(),
                cwd: cwd.to_string(),
                agent_provider: agent_provider.map(|p| p.as_str().to_string()),
                agent_session_id: None,
            })
        }
        "split" => {
            let direction = spec
                .get("direction")
                .and_then(|v| v.as_str())
                .unwrap_or("horizontal");
            let orientation = if direction == "vertical" {
                "vertical"
            } else {
                "horizontal"
            };
            let ratio = spec.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.5);
            if !(0.05..=0.95).contains(&ratio) {
                return Err(format!("split ratio {} out of range 0.05-0.95", ratio));
            }
            let start = spec
                .get("start")
                .ok_or_else(|| "split node missing \"start\"".to_string())?;
            let end = spec
                .get("end")
                .ok_or_else(|| "split node missing \"end\"".to_string())?;
            Ok(SplitNodeData::Split {
                orientation: orientation.to_string(),
                ratio,
                start: Box::new(layout_to_data(start, default_cwd, depth + 1, startups)?),
                end: Box::new(layout_to_data(end, default_cwd, depth + 1, startups)?),
            })
        }
        other => Err(format!("unknown layout node type {:?}", other)),
    }
}

/// Type each pending startup command into its pane once the surface has
/// realized (surfaces are created lazily on GLArea realize, one GTK frame
/// after workspace.create returns). Retries every 200ms for up to 5s;
/// whatever hasn't resolved by then is dropped with a log line.
fn schedule_startup_commands(
    state: crate::app_state::AppStateRef,
    mut pending: Vec<(String, String)>,
) {
    if pending.is_empty() {
        return;
    }
    let mut tries = 0u32;
    glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        tries += 1;
        pending.retain(|(uuid, cmd)| {
            let surf = resolve_surface_ptr(&state.borrow(), Some(uuid));
            match surf {
                Some(s) if !s.is_null() => {
                    let mut bytes = cmd.clone().into_bytes();
                    bytes.push(b'\n'); // text_input normalizes to CR
                    unsafe { surface_type_bytes(s, &bytes) };
                    false // done — drop from pending
                }
                _ => true, // not realized yet — keep retrying
            }
        });
        if pending.is_empty() || tries >= 25 {
            for (uuid, _) in &pending {
                tracing::debug!(
                    "cmux: layout startup command dropped — surface {} never realized",
                    uuid
                );
            }
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

/// After a session restore, boot each restored agent surface — resuming its
/// captured session if one exists, otherwise starting the agent fresh. Uses
/// the same deferred-typing path as layout startup commands.
pub fn schedule_agent_resumes(state: crate::app_state::AppStateRef) {
    let pending: Vec<(String, String)> = {
        let reg = match crate::agent::AGENT_SESSIONS.lock() {
            Ok(m) => m,
            Err(_) => return,
        };
        reg.iter()
            .map(|(uuid, session)| (uuid.clone(), crate::agent::startup_command(uuid, session)))
            .collect()
    };
    schedule_startup_commands(state, pending);
}

/// Dispatch a SocketCommand on the GTK main thread.
/// SOCK-05: Only focus-intent commands (workspace.select, workspace.next/previous/last,
/// pane.focus, pane.last, surface.focus) may call grab_active_focus() or focus_active_surface().
#[allow(unused_variables)]
pub fn handle_socket_command(cmd: SocketCommand, state: &crate::app_state::AppStateRef) {
    match cmd {
        // -- system.* --
        SocketCommand::Ping { req_id, resp_tx } => {
            let _ = resp_tx.send(ok(req_id, json!({"pong": true})));
        }

        SocketCommand::Quit { req_id, resp_tx } => {
            // Ack before quitting so the CLI sees a reply instead of a
            // dropped connection. app.quit() runs the normal GTK teardown
            // (connect_shutdown → browser cleanup) and the session file
            // already reflects the latest mutation (debounced save +
            // panic-hook tee), matching a normal window close.
            let _ = resp_tx.send(ok(req_id, json!({"quitting": true})));
            let app = state.borrow().gtk_app.clone();
            gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                tracing::info!("cmux: quit requested over socket");
                app.quit();
            });
        }

        SocketCommand::Identify { req_id, resp_tx } => {
            let socket_path = crate::socket::socket_path().to_string_lossy().to_string();
            let _ = resp_tx.send(ok(
                req_id,
                json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "platform": "linux",
                    "socket_path": socket_path,
                    // What platform::apply_launch_env auto-configured at startup
                    // (e.g. GDK_DEBUG=gl-prefer-gl on NVIDIA). Empty = nothing.
                    "launch_env": crate::platform::applied_launch_env(),
                    // GL context facts from the first GLArea realize (or its
                    // error) — the first thing to check on a blank window.
                    "gl": crate::ghostty::surface::gl_info(),
                }),
            ));
        }

        SocketCommand::Capabilities { req_id, resp_tx } => {
            let methods: Vec<&str> = vec![
                "system.ping",
                "system.identify",
                "system.capabilities",
                "workspace.list",
                "workspace.current",
                "workspace.create",
                "workspace.select",
                "workspace.close",
                "workspace.rename",
                "workspace.next",
                "workspace.previous",
                "workspace.last",
                "workspace.reorder",
                "surface.list",
                "surface.split",
                "surface.spawn",
                "surface.focus",
                "surface.close",
                "surface.send_text",
                "surface.send_key",
                "surface.read_text",
                "surface.health",
                "surface.refresh",
                "surface.top",
                "rendezvous.wait",
                "rendezvous.signal",
                "workspace.set_group",
                "workspace_group.list",
                "pane.list",
                "pane.focus",
                "pane.last",
                "window.list",
                "window.current",
                "notification.list",
                "notification.clear",
                "notification.create",
                "events.subscribe",
                "workspace.set_status",
                "workspace.set_progress",
                "workspace.log",
                "agent.hooks_setup",
                "agent.list",
                "agent.report_session",
                // Browser lifecycle + streaming
                "browser.open",
                "browser.close",
                "browser.list",
                "browser.stream.enable",
                "browser.stream.disable",
                "browser.snapshot",
                "browser.screenshot",
                // P0: navigation
                "browser.navigate",
                "browser.goto",
                "browser.back",
                "browser.forward",
                "browser.reload",
                // P0: interaction
                "browser.click",
                "browser.dblclick",
                "browser.type",
                "browser.fill",
                "browser.press",
                "browser.keydown",
                "browser.keyup",
                "browser.hover",
                "browser.focus",
                "browser.check",
                "browser.uncheck",
                "browser.select",
                "browser.scroll",
                "browser.scroll_into_view",
                "browser.drag",
                "browser.upload",
                "browser.download",
                "browser.pdf",
                // P0: evaluation + waiting
                "browser.eval",
                "browser.wait",
                // P0: getters
                "browser.get.url",
                "browser.get.title",
                "browser.get.text",
                "browser.get.html",
                "browser.get.value",
                "browser.get.attr",
                "browser.get.count",
                "browser.get.box",
                "browser.get.styles",
                // P0: state checks
                "browser.is.visible",
                "browser.is.enabled",
                "browser.is.checked",
                // P1: locators
                "browser.find.role",
                "browser.find.text",
                "browser.find.label",
                "browser.find.placeholder",
                "browser.find.alt",
                "browser.find.title",
                "browser.find.testid",
                "browser.find.nth",
                "browser.find.first",
                "browser.find.last",
                // P1: frames, dialogs, console, errors
                "browser.frame.select",
                "browser.frame.main",
                "browser.dialog.accept",
                "browser.dialog.dismiss",
                "browser.console.list",
                "browser.errors.list",
                "browser.highlight",
                "browser.state.save",
                "browser.state.load",
                // Debug
                "debug.layout",
                "debug.type",
            ];
            let _ = resp_tx.send(ok(req_id, json!({"methods": methods})));
        }

        // -- workspace.* --
        SocketCommand::WorkspaceList { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let s = state.borrow();
            let list: Vec<Value> = s
                .workspaces
                .iter()
                .enumerate()
                .map(|(i, ws)| {
                    let pane_count = s
                        .split_engines
                        .get(i)
                        .map(|e| e.all_panes().len())
                        .unwrap_or(0);
                    json!({
                        "index": i,
                        "id": ws.uuid.to_string(),
                        "title": ws.name,
                        "selected": i == s.active_index,
                        "pane_count": pane_count,
                        "group": ws.group,
                    })
                })
                .collect();
            let _ = resp_tx.send(ok(req_id, json!({"workspaces": list})));
        }

        SocketCommand::WorkspaceCurrent { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let s = state.borrow();
            match s.active_workspace() {
                Some(ws) => {
                    let _ = resp_tx.send(ok(
                        req_id,
                        json!({
                            "uuid": ws.uuid.to_string(),
                            // Alias for macOS-harness compatibility
                            // (tests_v2/cmux.py reads workspace_id).
                            "workspace_id": ws.uuid.to_string(),
                            "name": ws.name,
                        }),
                    ));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "no_workspace", "no active workspace"));
                }
            }
        }

        SocketCommand::WorkspaceCreate {
            req_id,
            remote_target,
            name,
            cwd,
            layout,
            resp_tx,
        } => {
            if let Some(target) = remote_target {
                // SSH workspace creation per D-13, D-15
                // Create per-workspace bridge for SSH I/O routing
                let (write_tx, write_rx) = tokio::sync::mpsc::unbounded_channel();
                let (output_tx, _output_rx) = tokio::sync::mpsc::unbounded_channel();
                let bridge = std::sync::Arc::new(crate::ssh::bridge::SshBridge::new(
                    write_tx, write_rx, output_tx,
                ));
                let id = state
                    .borrow_mut()
                    .create_remote_workspace(target.clone(), &bridge);
                // Store bridge on AppState for later access
                state
                    .borrow_mut()
                    .workspace_bridges
                    .insert(id, bridge.clone());
                let uuid_str = {
                    let s = state.borrow();
                    s.workspaces
                        .iter()
                        .find(|ws| ws.id == id)
                        .map(|ws| ws.uuid.to_string())
                        .unwrap_or_default()
                };
                // Spawn SSH lifecycle task using the runtime_handle stored on AppState
                let ssh_tx = state.borrow().ssh_event_tx.clone();
                let rt_handle = state.borrow().runtime_handle.clone();
                if let (Some(tx), Some(rt)) = (ssh_tx, rt_handle) {
                    let handle = rt.spawn(crate::ssh::tunnel::run_ssh_lifecycle(
                        id, target, tx, bridge,
                    ));
                    state.borrow_mut().ssh_task_handles.insert(id, handle);
                }
                let _ = resp_tx.send(ok(req_id, json!({"uuid": uuid_str, "remote": true})));
            } else if let Some(ref layout_spec) = layout {
                // Declarative layout: translate the user spec into the session
                // tree format and build it with the restore machinery.
                let mut startups: Vec<(String, String)> = Vec::new();
                let data = match layout_to_data(layout_spec, cwd.as_deref(), 0, &mut startups) {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = resp_tx.send(err(req_id, "invalid_params", &e));
                        return;
                    }
                };
                let surfaces: Vec<Value> = startups
                    .iter()
                    .map(|(uuid, cmd)| json!({"uuid": uuid, "command": cmd}))
                    .collect();
                let ws_session = crate::session::WorkspaceSession {
                    uuid: String::new(),
                    name: name.clone().unwrap_or_else(|| "Workspace".to_string()),
                    active_pane_uuid: None,
                    layout: data,
                    group: None,
                };
                let created = state.borrow_mut().restore_workspace(&ws_session);
                match created {
                    Some(id) => {
                        let (uuid_str, idx) = {
                            let s = state.borrow();
                            let idx = s.workspaces.iter().position(|ws| ws.id == id).unwrap_or(0);
                            (s.workspaces[idx].uuid.to_string(), idx)
                        };
                        // restore_workspace doesn't select or save; creating does both.
                        state.borrow_mut().switch_to_index(idx);
                        state.borrow().trigger_session_save();
                        schedule_startup_commands(state.clone(), startups);
                        crate::socket::events::emit(
                            "workspace.created",
                            json!({"uuid": uuid_str, "name": ws_session.name, "layout": true}),
                        );
                        let _ = resp_tx
                            .send(ok(req_id, json!({"uuid": uuid_str, "surfaces": surfaces})));
                    }
                    None => {
                        let _ = resp_tx.send(err(
                            req_id,
                            "invalid_params",
                            "layout rejected (tree too deep)",
                        ));
                    }
                }
            } else {
                // Local workspace, plus optional --name and --cwd. The shell
                // now starts in --cwd (working_directory on the surface), so
                // no deferred `cd` and no split-timing race.
                let id = state.borrow_mut().create_workspace_in(cwd.clone());
                let uuid_str = {
                    let mut s = state.borrow_mut();
                    // create_workspace leaves the new workspace active, so
                    // rename_active targets it (same pattern as WorkspaceRename).
                    if let Some(ref n) = name {
                        s.rename_active(n.clone());
                    }
                    let idx = s.workspaces.iter().position(|ws| ws.id == id).unwrap_or(0);
                    s.workspaces[idx].uuid.to_string()
                };
                crate::socket::events::emit(
                    "workspace.created",
                    json!({"uuid": uuid_str, "name": name, "layout": false}),
                );
                let _ = resp_tx.send(ok(req_id, json!({"uuid": uuid_str})));
            }
        }

        SocketCommand::WorkspaceSelect {
            req_id,
            id,
            resp_tx,
        } => {
            // SOCK-05: workspace.select IS a focus-intent command.
            let idx = {
                let s = state.borrow();
                s.workspaces.iter().position(|ws| ws.uuid.to_string() == id)
            };
            match idx {
                Some(i) => {
                    state.borrow_mut().switch_to_index(i);
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        SocketCommand::WorkspaceClose {
            req_id,
            id,
            resp_tx,
        } => {
            // SOCK-05: No focus side effects (close_workspace adjusts index internally).
            let idx = {
                let s = state.borrow();
                s.workspaces.iter().position(|ws| ws.uuid.to_string() == id)
            };
            match idx {
                Some(i) => {
                    let closed = state.borrow_mut().close_workspace(i);
                    if closed {
                        let _ = resp_tx.send(ok(req_id, json!({})));
                    } else {
                        let _ = resp_tx.send(err(
                            req_id,
                            "last_workspace",
                            "cannot close the last workspace",
                        ));
                    }
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        SocketCommand::WorkspaceRename {
            req_id,
            id,
            name,
            resp_tx,
        } => {
            // SOCK-05: No focus side effects. Find workspace by uuid, switch to it
            // (rename_active requires the target to be active), then rename.
            let idx = {
                let s = state.borrow();
                s.workspaces.iter().position(|ws| ws.uuid.to_string() == id)
            };
            match idx {
                Some(i) => {
                    let mut s = state.borrow_mut();
                    let prev_active = s.active_index;
                    s.switch_to_index(i);
                    s.rename_active(name);
                    // Restore previous active index to avoid focus side effect.
                    s.switch_to_index(prev_active);
                    drop(s);
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        SocketCommand::WorkspaceNext { req_id, resp_tx } => {
            // SOCK-05: focus-intent command.
            state.borrow_mut().switch_next();
            let _ = resp_tx.send(ok(req_id, json!({})));
        }

        SocketCommand::WorkspacePrev { req_id, resp_tx } => {
            // SOCK-05: focus-intent command.
            state.borrow_mut().switch_prev();
            let _ = resp_tx.send(ok(req_id, json!({})));
        }

        SocketCommand::WorkspaceLast { req_id, resp_tx } => {
            // SOCK-05: focus-intent command.
            // "Last" = most recently visited; for now same as prev (Phase 4 can track history).
            state.borrow_mut().switch_prev();
            let _ = resp_tx.send(ok(req_id, json!({})));
        }

        SocketCommand::WorkspaceReorder {
            req_id,
            id,
            position,
            resp_tx,
        } => {
            // SOCK-05: No focus side effects.
            let mut s = state.borrow_mut();
            let idx = s.workspaces.iter().position(|ws| ws.uuid.to_string() == id);
            match idx {
                Some(from) => {
                    let to = position.min(s.workspaces.len().saturating_sub(1));
                    let ws = s.workspaces.remove(from);
                    let engine = s.split_engines.remove(from);
                    s.workspaces.insert(to, ws);
                    s.split_engines.insert(to, engine);
                    // Adjust active_index after reorder.
                    if from == s.active_index {
                        s.active_index = to;
                    } else if from < s.active_index && to >= s.active_index {
                        s.active_index -= 1;
                    } else if from > s.active_index && to <= s.active_index {
                        s.active_index += 1;
                    }
                    drop(s);
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => {
                    drop(s);
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        // -- window.* --
        SocketCommand::WindowList { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let workspace_count = state.borrow().workspaces.len();
            let _ = resp_tx.send(ok(
                req_id,
                json!({
                    "windows": [{"id": "main", "workspaces": workspace_count}]
                }),
            ));
        }

        SocketCommand::WindowCurrent { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let _ = resp_tx.send(ok(req_id, json!({"id": "main"})));
        }

        // -- debug.* --
        SocketCommand::DebugLayout { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let s = state.borrow();
            match s.split_engines.get(s.active_index) {
                Some(engine) => {
                    let data = engine.root.to_data();
                    let json_tree = serde_json::to_value(&data).unwrap_or(Value::Null);
                    let _ = resp_tx.send(ok(req_id, json!({"layout": json_tree})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "no_workspace", "no active workspace"));
                }
            }
        }

        SocketCommand::DebugType {
            req_id,
            text,
            resp_tx,
        } => {
            // SOCK-05: No focus side effects (sends text to active surface without changing focus).
            let s = state.borrow();
            if let Some(engine) = s.split_engines.get(s.active_index) {
                if let Some(pane_id) = engine.root.find_active_pane_id() {
                    if let Some(surface) = engine.root.find_surface_for_pane(pane_id) {
                        if !surface.is_null() {
                            let c_text = std::ffi::CString::new(text.clone()).unwrap_or_default();
                            unsafe {
                                crate::ghostty::ffi::ghostty_surface_text(
                                    surface,
                                    c_text.as_ptr(),
                                    c_text.to_bytes().len(),
                                );
                            }
                        }
                    }
                }
            }
            let _ = resp_tx.send(ok(req_id, json!({})));
        }

        // ── surface.* ────────────────────────────────────────────────────
        SocketCommand::SurfaceList { req_id, resp_tx } => {
            // SOCK-05: No focus side effects.
            let s = state.borrow();
            let mut panes: Vec<Value> = Vec::new();
            for (ws_idx, (ws, engine)) in
                s.workspaces.iter().zip(s.split_engines.iter()).enumerate()
            {
                for (pane_uuid, pane_id, active) in engine.all_panes() {
                    panes.push(json!({
                        "uuid": pane_uuid.to_string(),
                        "workspace_uuid": ws.uuid.to_string(),
                        "active": active && ws_idx == s.active_index,
                        // Last SET_TITLE the surface reported (null = none yet)
                        "title": s.surface_titles.get(&pane_id),
                    }));
                }
            }
            let _ = resp_tx.send(ok(req_id, json!({"surfaces": panes})));
        }

        SocketCommand::SurfaceTop { req_id, resp_tx } => {
            // Per-surface process stats. Shell PIDs aren't exposed by
            // ghostty, so surfaces map to cmux-app's child processes via
            // CMUX_PANE env (exact, agent panes) or creation order (pts
            // numbers allocate sequentially — heuristic, flagged as such).
            let s = state.borrow();
            let mut panes: Vec<(String, String)> = Vec::new();
            for (ws, engine) in s.workspaces.iter().zip(s.split_engines.iter()) {
                for (uuid, _pane_id, _active) in engine.all_panes() {
                    panes.push((uuid.to_string(), ws.name.clone()));
                }
            }
            drop(s);
            let procs = crate::procstat::child_process_stats();
            let row = |uuid: &str, ws: &str, p: &crate::procstat::ProcEntry, how: &str| {
                json!({
                    "surface": uuid,
                    "workspace": ws,
                    "pid": p.pid,
                    "cpu_secs": (p.cpu_secs * 100.0).round() / 100.0,
                    "rss_bytes": p.rss_bytes,
                    "cmdline": p.cmdline,
                    "matched_by": how,
                })
            };
            let mut used = vec![false; procs.len()];
            let mut rows: Vec<Value> = Vec::new();
            let mut unmatched: Vec<(String, String)> = Vec::new();
            for (uuid, ws_name) in &panes {
                match procs
                    .iter()
                    .position(|p| p.cmux_pane.as_deref() == Some(uuid.as_str()))
                {
                    Some(i) => {
                        used[i] = true;
                        rows.push(row(uuid, ws_name, &procs[i], "env"));
                    }
                    None => unmatched.push((uuid.clone(), ws_name.clone())),
                }
            }
            let free: Vec<usize> = (0..procs.len())
                .filter(|&i| !used[i] && procs[i].pts >= 0)
                .collect();
            for (n, (uuid, ws_name)) in unmatched.iter().enumerate() {
                match free.get(n) {
                    Some(&i) => rows.push(row(uuid, ws_name, &procs[i], "order")),
                    None => rows.push(json!({
                        "surface": uuid, "workspace": ws_name, "pid": null,
                    })),
                }
            }
            let _ = resp_tx.send(ok(req_id, json!({"top": rows})));
        }

        SocketCommand::SurfaceSplit {
            req_id,
            id,
            direction,
            agent,
            resp_tx,
        } => {
            // Split a specific surface (by UUID) or the active pane in the
            // active workspace. SplitEngine only knows how to split its
            // active pane, so an explicit target becomes the active pane
            // first — which matches GUI semantics (splitting focuses the
            // split location anyway).
            let orientation = if direction == "vertical" {
                gtk4::Orientation::Vertical
            } else {
                gtk4::Orientation::Horizontal
            };
            // Minimum on-screen size (logical px) a pane may have AFTER a
            // split. A horizontal split halves width, a vertical split halves
            // height; going below this yields a pane too narrow/short for
            // shells and TUI agents to render (some crash outright). ~200px is
            // roughly 24 columns at a default font — comfortably above the
            // widths that break real agent TUIs.
            const MIN_PANE_PX: i32 = 200;

            let result = {
                let mut s = state.borrow_mut();
                let idx = s.active_index;
                if let Some(engine) = s.split_engines.get_mut(idx) {
                    if let Some(ref uuid_str) = id {
                        match engine.find_pane_id_by_uuid(uuid_str) {
                            Some(pid) => engine.active_pane_id = pid,
                            None => {
                                let _ = resp_tx.send(err(req_id, "not_found", "surface not found"));
                                return;
                            }
                        }
                    }
                    // Reject splits that would produce an unusably small pane.
                    if let Some((w, h)) = engine.pane_size(engine.active_pane_id) {
                        let resulting = if orientation == gtk4::Orientation::Horizontal {
                            w / 2
                        } else {
                            h / 2
                        };
                        if resulting < MIN_PANE_PX {
                            let axis = if orientation == gtk4::Orientation::Horizontal {
                                "wide"
                            } else {
                                "tall"
                            };
                            drop(s);
                            let _ = resp_tx.send(err(
                                req_id,
                                "pane_too_small",
                                &format!(
                                    "pane not {} enough to split (would leave {}px, need {}px); resize the window or close other panes",
                                    axis, resulting, MIN_PANE_PX
                                ),
                            ));
                            return;
                        }
                    }
                    engine.split_active(orientation).and_then(|new_pane_id| {
                        // Find the uuid of the newly created pane.
                        engine
                            .all_panes()
                            .into_iter()
                            .find(|(_, pid, _)| *pid == new_pane_id)
                            .map(|(uuid, _, _)| uuid.to_string())
                    })
                } else {
                    None
                }
            };
            match result {
                Some(uuid_str) => {
                    // `split --agent <provider>`: the new pane boots a
                    // provider-aware agent session (roadmap 3.5). The
                    // startup command is typed in after the GLArea has had
                    // time to realize and spawn its shell.
                    if let Some(provider) =
                        agent.as_deref().and_then(crate::agent::Provider::from_str)
                    {
                        crate::agent::register(&uuid_str, provider, None, None);
                        let session = crate::agent::AgentSession {
                            provider,
                            session_id: None,
                            cwd: None,
                        };
                        let boot = crate::agent::startup_command(&uuid_str, &session);
                        let state2 = state.clone();
                        let uuid2 = uuid_str.clone();
                        gtk4::glib::timeout_add_local_once(
                            std::time::Duration::from_millis(700),
                            move || {
                                let s = state2.borrow();
                                if let Some(surf) = resolve_surface_ptr(&s, Some(&uuid2)) {
                                    unsafe {
                                        surface_type_bytes(surf, format!("{boot}\n").as_bytes());
                                    }
                                } else {
                                    tracing::warn!(
                                        "split --agent: surface {uuid2} not realized; agent not booted"
                                    );
                                }
                            },
                        );
                    }
                    let _ = resp_tx.send(ok(req_id, json!({"uuid": uuid_str, "agent": agent})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "split_failed", "could not split pane"));
                }
            }
        }

        SocketCommand::SurfaceSpawn {
            req_id,
            id,
            agent,
            resp_tx,
        } => {
            // Fibonacci/spiral auto-split: same min-size guard as
            // surface.split, but the target pane and orientation are decided
            // by SplitEngine::spiral_split — an explicit `id` splits that
            // pane directly; otherwise it continues from the spiral tail
            // (the pane most recently created by a spawn), NOT whatever pane
            // currently has keyboard focus. Manually navigating to inspect
            // an older pane must not redirect where the next spawn lands.
            const MIN_PANE_PX: i32 = 200;

            let result = {
                let mut s = state.borrow_mut();
                let idx = s.active_index;
                if let Some(engine) = s.split_engines.get_mut(idx) {
                    let explicit_target = match id {
                        Some(ref uuid_str) => match engine.find_pane_id_by_uuid(uuid_str) {
                            Some(pid) => Some(pid),
                            None => {
                                let _ = resp_tx.send(err(req_id, "not_found", "surface not found"));
                                return;
                            }
                        },
                        None => None,
                    };
                    let target =
                        explicit_target.unwrap_or_else(|| engine.spiral_target_pane_id());
                    // Peek the orientation spiral_split will use (decided from
                    // the target pane's own aspect ratio), to apply the same
                    // too-small guard as surface.split.
                    let next_orientation = engine.spiral_orientation_for(target);
                    if let Some((w, h)) = engine.pane_size(target) {
                        let resulting = if next_orientation == gtk4::Orientation::Horizontal {
                            w / 2
                        } else {
                            h / 2
                        };
                        if resulting < MIN_PANE_PX {
                            let axis = if next_orientation == gtk4::Orientation::Horizontal {
                                "wide"
                            } else {
                                "tall"
                            };
                            drop(s);
                            let _ = resp_tx.send(err(
                                req_id,
                                "pane_too_small",
                                &format!(
                                    "pane not {} enough to spawn a spiral split (would leave {}px, need {}px); resize the window or close other panes",
                                    axis, resulting, MIN_PANE_PX
                                ),
                            ));
                            return;
                        }
                    }
                    engine.spiral_split(explicit_target).and_then(|new_pane_id| {
                        engine
                            .all_panes()
                            .into_iter()
                            .find(|(_, pid, _)| *pid == new_pane_id)
                            .map(|(uuid, _, _)| uuid.to_string())
                    })
                } else {
                    None
                }
            };
            match result {
                Some(uuid_str) => {
                    if let Some(provider) =
                        agent.as_deref().and_then(crate::agent::Provider::from_str)
                    {
                        crate::agent::register(&uuid_str, provider, None, None);
                        let session = crate::agent::AgentSession {
                            provider,
                            session_id: None,
                            cwd: None,
                        };
                        let boot = crate::agent::startup_command(&uuid_str, &session);
                        let state2 = state.clone();
                        let uuid2 = uuid_str.clone();
                        gtk4::glib::timeout_add_local_once(
                            std::time::Duration::from_millis(700),
                            move || {
                                let s = state2.borrow();
                                if let Some(surf) = resolve_surface_ptr(&s, Some(&uuid2)) {
                                    unsafe {
                                        surface_type_bytes(surf, format!("{boot}\n").as_bytes());
                                    }
                                } else {
                                    tracing::warn!(
                                        "surface.spawn --agent: surface {uuid2} not realized; agent not booted"
                                    );
                                }
                            },
                        );
                    }
                    let _ = resp_tx.send(ok(req_id, json!({"uuid": uuid_str, "agent": agent})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "split_failed", "could not spawn spiral pane"));
                }
            }
        }

        SocketCommand::SurfaceFocus {
            req_id,
            id,
            resp_tx,
        } => {
            // SOCK-05: surface.focus IS a focus-intent command — allowed to change focus.
            let pane_id = {
                let s = state.borrow();
                s.split_engines
                    .get(s.active_index)
                    .and_then(|engine| engine.find_pane_id_by_uuid(&id))
            };
            match pane_id {
                Some(pid) => {
                    let mut s = state.borrow_mut();
                    let idx = s.active_index;
                    if let Some(engine) = s.split_engines.get_mut(idx) {
                        engine.active_pane_id = pid;
                        engine.root.update_focus_css(pid);
                        engine.grab_active_focus();
                    }
                    drop(s);
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "surface not found"));
                }
            }
        }

        SocketCommand::SurfaceClose {
            req_id,
            id,
            resp_tx,
        } => {
            // Close pane by uuid, or the active pane in the active workspace
            // when no id is given. Set the target as active, then close_active().
            let pane_id = {
                let s = state.borrow();
                s.split_engines.get(s.active_index).and_then(|engine| {
                    match id {
                        Some(ref uuid_str) => engine.find_pane_id_by_uuid(uuid_str),
                        None => Some(engine.active_pane_id),
                    }
                })
            };
            match pane_id {
                Some(pid) => {
                    let result = {
                        let mut s = state.borrow_mut();
                        let idx = s.active_index;
                        if let Some(engine) = s.split_engines.get_mut(idx) {
                            engine.active_pane_id = pid;
                            engine.root.update_focus_css(pid);
                            engine.close_active()
                        } else {
                            None
                        }
                    };
                    match result {
                        Some(_) => {
                            let _ = resp_tx.send(ok(req_id, json!({})));
                        }
                        None => {
                            let _ =
                                resp_tx.send(err(req_id, "close_failed", "cannot close last pane"));
                        }
                    }
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "surface not found"));
                }
            }
        }

        SocketCommand::SurfaceSendText {
            req_id,
            id,
            text,
            resp_tx,
        } => {
            // SOCK-05: send_text is NOT a focus-intent command — NO focus change.
            let surface = resolve_surface_ptr(&state.borrow(), id.as_ref());
            match surface {
                Some(surf) if !surf.is_null() => {
                    unsafe { surface_type_bytes(surf, text.as_bytes()) };
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                _ => {
                    let _ = resp_tx.send(err(req_id, "not_found", "surface not found"));
                }
            }
        }

        SocketCommand::SurfaceSendKey {
            req_id,
            id,
            key,
            resp_tx,
        } => {
            // SOCK-05: send_key is NOT a focus-intent command — NO focus change.
            // Named keys (enter, tab, escape, arrows…) and ctrl+<letter> map to
            // their terminal byte sequences; single printable chars pass through.
            let surface = resolve_surface_ptr(&state.borrow(), id.as_ref());
            match (surface, key_to_bytes(&key)) {
                (Some(surf), Some(bytes)) if !surf.is_null() => {
                    unsafe { surface_type_bytes(surf, &bytes) };
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                (_, None) => {
                    let _ = resp_tx.send(err(
                        req_id,
                        "invalid_params",
                        &format!("unknown key: {key:?}"),
                    ));
                }
                _ => {
                    let _ = resp_tx.send(err(req_id, "not_found", "surface not found"));
                }
            }
        }

        SocketCommand::SurfaceReadText {
            req_id,
            id,
            scrollback,
            resp_tx,
        } => {
            // SOCK-05: No focus side effects.
            // Reads via ghostty_surface_read_text (exported by the cmux ghostty
            // fork; locks renderer state internally). VIEWPORT = visible page;
            // SCREEN = full buffer including scrollback history.
            let surface = resolve_surface_ptr(&state.borrow(), id.as_ref());
            match surface {
                Some(surf) if !surf.is_null() => {
                    use crate::ghostty::ffi as g;
                    let tag = if scrollback {
                        g::ghostty_point_tag_e_GHOSTTY_POINT_SCREEN
                    } else {
                        g::ghostty_point_tag_e_GHOSTTY_POINT_VIEWPORT
                    };
                    let text = unsafe {
                        let sel = g::ghostty_selection_s {
                            top_left: g::ghostty_point_s {
                                tag,
                                coord: g::ghostty_point_coord_e_GHOSTTY_POINT_COORD_TOP_LEFT,
                                x: 0,
                                y: 0,
                            },
                            bottom_right: g::ghostty_point_s {
                                tag,
                                coord: g::ghostty_point_coord_e_GHOSTTY_POINT_COORD_BOTTOM_RIGHT,
                                x: 0,
                                y: 0,
                            },
                            rectangle: false,
                        };
                        let mut out: g::ghostty_text_s = std::mem::zeroed();
                        if g::ghostty_surface_read_text(surf, sel, &mut out) && !out.text.is_null()
                        {
                            let bytes =
                                std::slice::from_raw_parts(out.text as *const u8, out.text_len);
                            let s = String::from_utf8_lossy(bytes).into_owned();
                            g::ghostty_surface_free_text(surf, &mut out);
                            Some(s)
                        } else {
                            None
                        }
                    };
                    match text {
                        Some(t) => {
                            let _ = resp_tx.send(ok(req_id, json!({"text": t})));
                        }
                        None => {
                            let _ = resp_tx.send(err(
                                req_id,
                                "internal_error",
                                "failed to read surface text",
                            ));
                        }
                    }
                }
                _ => {
                    let _ = resp_tx.send(err(req_id, "not_found", "surface not found"));
                }
            }
        }

        SocketCommand::SurfaceHealth {
            req_id,
            id,
            resp_tx,
        } => {
            // SOCK-05: health is NOT focus-intent — NO focus change.
            let (found, has_attention) = {
                let s = state.borrow();
                if let Some(ref uuid_str) = id {
                    // Search every workspace engine, not just the active one —
                    // fleet orchestrators health-check background workspaces.
                    s.split_engines
                        .iter()
                        .find_map(|engine| {
                            engine
                                .find_pane_id_by_uuid(uuid_str)
                                .map(|pid| (true, engine.root.pane_has_attention(pid)))
                        })
                        .unwrap_or((false, false))
                } else if let Some(engine) = s.split_engines.get(s.active_index) {
                    let attn = engine
                        .root
                        .find_active_pane_id()
                        .map(|pid| engine.root.pane_has_attention(pid))
                        .unwrap_or(false);
                    (true, attn)
                } else {
                    (false, false)
                }
            };
            let _ = resp_tx.send(ok(
                req_id,
                json!({"alive": found, "has_attention": has_attention}),
            ));
        }

        SocketCommand::SurfaceRefresh {
            req_id,
            id,
            resp_tx,
        } => {
            // SOCK-05: refresh is NOT focus-intent — NO focus change.
            // Queue a render on the target surface's GLArea.
            let gl_area = {
                let s = state.borrow();
                if let Some(engine) = s.split_engines.get(s.active_index) {
                    let target_pane_id = if let Some(ref uuid_str) = id {
                        engine.find_pane_id_by_uuid(uuid_str)
                    } else {
                        engine.root.find_active_pane_id()
                    };
                    target_pane_id.and_then(|pid| engine.gl_area_for_pane(pid))
                } else {
                    None
                }
            };
            if let Some(area) = gl_area {
                area.queue_render();
            }
            let _ = resp_tx.send(ok(req_id, json!({})));
        }

        // ── pane.* ───────────────────────────────────────────────────────────
        SocketCommand::PaneList { req_id, resp_tx } => {
            // SOCK-05: No focus side effects. Alias for surface.list.
            let s = state.borrow();
            let mut panes: Vec<Value> = Vec::new();
            for (ws_idx, (ws, engine)) in
                s.workspaces.iter().zip(s.split_engines.iter()).enumerate()
            {
                for (pane_uuid, pane_id, active) in engine.all_panes() {
                    panes.push(json!({
                        "uuid": pane_uuid.to_string(),
                        "workspace_uuid": ws.uuid.to_string(),
                        "active": active && ws_idx == s.active_index,
                        // Last SET_TITLE the surface reported (null = none yet)
                        "title": s.surface_titles.get(&pane_id),
                    }));
                }
            }
            let _ = resp_tx.send(ok(req_id, json!({"panes": panes})));
        }

        SocketCommand::PaneFocus {
            req_id,
            id,
            resp_tx,
        } => {
            // SOCK-05: pane.focus IS focus-intent — allowed to change focus.
            let pane_id = {
                let s = state.borrow();
                if let Some(engine) = s.split_engines.get(s.active_index) {
                    id.as_ref()
                        .and_then(|uuid_str| engine.find_pane_id_by_uuid(uuid_str))
                } else {
                    None
                }
            };
            match pane_id {
                Some(pid) => {
                    let mut s = state.borrow_mut();
                    let idx = s.active_index;
                    if let Some(engine) = s.split_engines.get_mut(idx) {
                        engine.active_pane_id = pid;
                        engine.root.update_focus_css(pid);
                        engine.grab_active_focus();
                    }
                    drop(s);
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "pane not found"));
                }
            }
        }

        SocketCommand::PaneLast { req_id, resp_tx } => {
            // SOCK-05: pane.last IS focus-intent — allowed to change focus.
            // Phase 3 stub: re-grab focus on current active pane. Phase 4 tracks focus history.
            {
                let s = state.borrow();
                if let Some(engine) = s.split_engines.get(s.active_index) {
                    engine.grab_active_focus();
                }
            }
            let _ = resp_tx.send(ok(req_id, json!({})));
        }

        // -- notification.* (Phase 4) --
        SocketCommand::NotificationList { req_id, resp_tx } => {
            // SOCK-05: No focus side effects. Read-only attention state query.
            let s = state.borrow();
            let notifications: Vec<Value> = s
                .workspaces
                .iter()
                .map(|ws| {
                    json!({
                        "workspace_uuid": ws.uuid.to_string(),
                        "workspace_name": ws.name,
                        "has_attention": ws.has_attention,
                    })
                })
                .collect();
            let _ = resp_tx.send(ok(req_id, json!({"notifications": notifications})));
        }

        SocketCommand::NotificationClear {
            req_id,
            id,
            resp_tx,
        } => {
            // SOCK-05: No focus side effects. Clears attention without switching workspace.
            let idx = {
                let s = state.borrow();
                s.workspaces.iter().position(|ws| ws.uuid.to_string() == id)
            };
            match idx {
                Some(i) => {
                    state.borrow_mut().clear_workspace_attention(i);
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        SocketCommand::AgentHooksSetup { req_id, resp_tx } => {
            // SOCK-05: No focus side effects. Writes ~/.claude/settings.json.
            match crate::agent::install_hooks() {
                Ok(providers) => {
                    let _ = resp_tx.send(ok(req_id, json!({"installed": providers})));
                }
                Err(e) => {
                    let _ = resp_tx.send(err(req_id, "internal_error", &e));
                }
            }
        }

        SocketCommand::AgentList { req_id, resp_tx } => {
            // SOCK-05: No focus side effects. Reports captured agent sessions
            // (`cmux surface resume show` / `cmux agent-sessions`).
            let s = state.borrow();
            let sessions: Vec<Value> = {
                let reg = crate::agent::AGENT_SESSIONS.lock();
                match reg {
                    Ok(m) => m
                        .iter()
                        .map(|(uuid, a)| {
                            // Resolve which workspace the surface lives in.
                            let ws = s.split_engines.iter().enumerate().find_map(|(i, e)| {
                                e.find_pane_id_by_uuid(uuid)
                                    .map(|_| s.workspaces[i].name.clone())
                            });
                            json!({
                                "surface_uuid": uuid,
                                "provider": a.provider.as_str(),
                                "session_id": a.session_id,
                                "resumable": a.session_id.is_some() && a.provider.resumable(),
                                "workspace_name": ws,
                            })
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                }
            };
            let _ = resp_tx.send(ok(req_id, json!({"sessions": sessions})));
        }

        SocketCommand::AgentReportSession {
            req_id,
            surface,
            provider,
            session_id,
            resp_tx,
        } => {
            // SOCK-05: No focus side effects. Called by the provider's
            // SessionStart hook (via `cmux agent report-session`).
            if surface.is_empty() || session_id.is_empty() {
                let _ = resp_tx.send(err(
                    req_id,
                    "invalid_params",
                    "surface and session_id required",
                ));
            } else {
                // If the hook fired for a surface we didn't pre-register (e.g.
                // agent launched by hand), register it now from the reported
                // provider so it still becomes resumable.
                if crate::agent::get(&surface).is_none() {
                    if let Some(p) = provider
                        .as_deref()
                        .and_then(crate::agent::Provider::from_str)
                    {
                        crate::agent::register(&surface, p, None, None);
                    }
                }
                match crate::agent::set_session_id(&surface, &session_id) {
                    crate::agent::CaptureResult::Captured => {
                        state.borrow().trigger_session_save();
                        crate::socket::events::emit(
                            "agent.session",
                            json!({"surface_uuid": surface, "session_id": session_id}),
                        );
                        let _ = resp_tx.send(ok(req_id, json!({"captured": true})));
                    }
                    crate::agent::CaptureResult::AlreadyCaptured => {
                        // Keep the first id (the one with history); ack anyway.
                        let _ = resp_tx.send(ok(
                            req_id,
                            json!({"captured": false, "kept_existing": true}),
                        ));
                    }
                    crate::agent::CaptureResult::NotAgent => {
                        let _ = resp_tx.send(err(
                            req_id,
                            "not_found",
                            "surface is not an agent surface",
                        ));
                    }
                }
            }
        }

        SocketCommand::WorkspaceSetGroup {
            req_id,
            workspace,
            group,
            resp_tx,
        } => {
            // Groups are labels, not entities: assigning a new label creates
            // the group; clearing the last member removes it (roadmap 3.4).
            let mut s = state.borrow_mut();
            let idx = match workspace {
                Some(ref uuid) => s
                    .workspaces
                    .iter()
                    .position(|ws| ws.uuid.to_string() == *uuid),
                None => Some(s.active_index),
            };
            match idx {
                Some(i) => {
                    let value = if group.trim().is_empty() {
                        None
                    } else {
                        Some(group.clone())
                    };
                    s.workspaces[i].group = value.clone();
                    let ws_uuid = s.workspaces[i].uuid.to_string();
                    s.trigger_session_save();
                    drop(s);
                    crate::socket::events::emit(
                        "workspace.group",
                        json!({"workspace_uuid": ws_uuid, "group": value}),
                    );
                    let _ = resp_tx.send(ok(req_id, json!({"group": value})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        SocketCommand::WorkspaceGroupList { req_id, resp_tx } => {
            let s = state.borrow();
            let mut groups: std::collections::BTreeMap<String, Vec<Value>> =
                std::collections::BTreeMap::new();
            for ws in &s.workspaces {
                if let Some(ref g) = ws.group {
                    groups.entry(g.clone()).or_default().push(json!({
                        "uuid": ws.uuid.to_string(),
                        "title": ws.name,
                    }));
                }
            }
            let out: Vec<Value> = groups
                .into_iter()
                .map(|(name, members)| json!({"name": name, "workspaces": members}))
                .collect();
            let _ = resp_tx.send(ok(req_id, json!({"groups": out})));
        }

        SocketCommand::WorkspaceSetStatus {
            req_id,
            workspace,
            state: status,
            color,
            resp_tx,
        } => {
            // SOCK-05: No focus side effects. Status board write (prompt-13 style).
            let s = state.borrow();
            let idx = match workspace {
                Some(ref uuid) => s
                    .workspaces
                    .iter()
                    .position(|ws| ws.uuid.to_string() == *uuid),
                None => Some(s.active_index),
            };
            match idx {
                Some(i) => {
                    if let Some(row) = s.sidebar_list.row_at_index(i as i32) {
                        crate::sidebar::set_row_status(
                            &row,
                            &status,
                            color.as_deref().unwrap_or("#546E7A"),
                        );
                    }
                    let ws_uuid = s.workspaces[i].uuid.to_string();
                    crate::socket::events::emit(
                        "workspace.status",
                        json!({"workspace_uuid": ws_uuid, "state": status, "color": color}),
                    );
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        SocketCommand::WorkspaceSetProgress {
            req_id,
            workspace,
            value,
            label,
            resp_tx,
        } => {
            // SOCK-05: No focus side effects.
            let s = state.borrow();
            let idx = match workspace {
                Some(ref uuid) => s
                    .workspaces
                    .iter()
                    .position(|ws| ws.uuid.to_string() == *uuid),
                None => Some(s.active_index),
            };
            match idx {
                Some(i) => {
                    if let Some(row) = s.sidebar_list.row_at_index(i as i32) {
                        crate::sidebar::set_row_progress(&row, value, label.as_deref());
                    }
                    let ws_uuid = s.workspaces[i].uuid.to_string();
                    crate::socket::events::emit(
                        "workspace.progress",
                        json!({"workspace_uuid": ws_uuid, "value": value, "label": label}),
                    );
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        SocketCommand::WorkspaceLog {
            req_id,
            workspace,
            message,
            resp_tx,
        } => {
            // SOCK-05: No focus side effects.
            let s = state.borrow();
            let idx = match workspace {
                Some(ref uuid) => s
                    .workspaces
                    .iter()
                    .position(|ws| ws.uuid.to_string() == *uuid),
                None => Some(s.active_index),
            };
            match idx {
                Some(i) => {
                    if let Some(row) = s.sidebar_list.row_at_index(i as i32) {
                        crate::sidebar::set_row_log(&row, &message);
                    }
                    let ws_uuid = s.workspaces[i].uuid.to_string();
                    crate::socket::events::emit(
                        "workspace.log",
                        json!({"workspace_uuid": ws_uuid, "message": message}),
                    );
                    let _ = resp_tx.send(ok(req_id, json!({})));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        SocketCommand::NotificationCreate {
            req_id,
            title,
            body,
            workspace,
            desktop,
            resp_tx,
        } => {
            // SOCK-05: No focus side effects — marks attention and notifies,
            // never switches workspace. Agents raise these from lifecycle
            // hooks ("done", "needs input"); orchestrators consume them via
            // `cmux events --name notification.created`.
            let ws_info = {
                let mut s = state.borrow_mut();
                let idx = match workspace {
                    Some(ref uuid) => s
                        .workspaces
                        .iter()
                        .position(|ws| ws.uuid.to_string() == *uuid),
                    None => Some(s.active_index),
                };
                idx.map(|i| {
                    s.workspaces[i].has_attention = true;
                    s.update_sidebar_attention(i);
                    (
                        s.workspaces[i].uuid.to_string(),
                        s.workspaces[i].name.clone(),
                    )
                })
            };
            match ws_info {
                Some((ws_uuid, ws_name)) => {
                    if desktop {
                        // notify-rust is unreliable on some desktops; the app
                        // convention is shelling out to notify-send (see CLAUDE.md).
                        let _ = std::process::Command::new("notify-send")
                            .arg("--app-name=cmux")
                            .arg(&title)
                            .arg(&body)
                            .spawn();
                    }
                    crate::socket::events::emit(
                        "notification.created",
                        json!({
                            "title": title,
                            "body": body,
                            "workspace_uuid": ws_uuid,
                            "workspace_name": ws_name,
                        }),
                    );
                    let _ = resp_tx.send(ok(
                        req_id,
                        json!({"created": true, "workspace_uuid": ws_uuid}),
                    ));
                }
                None => {
                    let _ = resp_tx.send(err(req_id, "not_found", "workspace not found"));
                }
            }
        }

        // -- browser.* (Phase 8: D-04 lifecycle + streaming) --
        // SOCK-05: None of these commands steal focus.
        SocketCommand::BrowserOpen {
            req_id,
            url,
            workspace,
            resp_tx,
        } => {
            let mut s = state.borrow_mut();
            // Lazy-init BrowserManager per D-05
            let bm = s.browser_manager_mut();
            // Ensure daemon is running (auto-start per D-05)
            if let Err(e) = bm.ensure_daemon() {
                let _ = resp_tx.send(err(req_id, "daemon_error", &e));
                return;
            }
            // Build params for agent-browser, including workspace if provided
            let mut open_params = serde_json::json!({"url": url});
            if let Some(ref ws) = workspace {
                open_params["workspace"] = serde_json::json!(ws);
            }
            match bm.send_command("navigate", open_params) {
                Ok(result) => {
                    // Allocate surface ref (D-06)
                    s.browser_surface_counter += 1;
                    let ref_id = s.browser_surface_counter;
                    let uuid = result
                        .get("id")
                        .or_else(|| result.get("surface_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    s.browser_surface_refs.insert(ref_id, uuid.clone());
                    // Augment response with surface_ref
                    let mut response = result.clone();
                    if let Some(obj) = response.as_object_mut() {
                        obj.insert(
                            "surface_ref".to_string(),
                            serde_json::json!(format!("surface:{}", ref_id)),
                        );
                        obj.insert("uuid".to_string(), serde_json::json!(uuid));
                    }
                    // Create preview pane and auto-enable streaming
                    let picture = {
                        let engine = s.active_split_engine_mut();
                        if let Some(eng) = engine {
                            find_preview_picture(&eng.root)
                                .or_else(|| eng.split_active_with_preview().map(|w| w.picture))
                        } else {
                            None
                        }
                    };
                    // Enable streaming so the preview pane shows the page
                    let runtime = s.runtime_handle.clone();
                    let bm = s.browser_manager_mut();
                    let _ = bm.send_command("stream_enable", serde_json::json!({}));
                    if let Some(pic) = picture {
                        if let Some(ref rt) = runtime {
                            let _ = bm.start_stream(rt, pic);
                        }
                    }
                    let _ = resp_tx.send(ok(req_id, response));
                }
                Err(e) => {
                    let _ = resp_tx.send(err(req_id, "browser_error", &e));
                }
            }
        }

        SocketCommand::BrowserStreamEnable { req_id, resp_tx } => {
            let mut s = state.borrow_mut();
            let bm = s.browser_manager_mut();
            if let Err(e) = bm.ensure_daemon() {
                let _ = resp_tx.send(err(req_id, "daemon_error", &e));
                return;
            }
            match bm.send_command("stream_enable", serde_json::json!({})) {
                Ok(result) => {
                    // Find the Picture widget from the Preview pane in the active workspace.
                    // If no preview pane exists yet, create one first.
                    let picture = {
                        let engine = s.active_split_engine_mut();
                        if let Some(eng) = engine {
                            // Try to find existing Preview node's Picture
                            find_preview_picture(&eng.root).or_else(|| {
                                // No preview pane yet -- create one
                                eng.split_active_with_preview().map(|w| w.picture)
                            })
                        } else {
                            None
                        }
                    };

                    // Wire the WebSocket stream to the Picture widget (Gap 1 fix)
                    if let Some(pic) = picture {
                        let runtime = s.runtime_handle.clone();
                        let bm = s.browser_manager_mut();
                        if let Some(ref rt) = runtime {
                            match bm.start_stream(rt, pic) {
                                Ok(()) => {
                                    // stream wired to preview pane
                                }
                                Err(e) => {
                                    tracing::warn!("cmux: stream enable failed: {}", e);
                                }
                            }
                        } else {
                            // no runtime handle
                        }
                    } else {
                        // no preview pane available
                    }

                    let _ = resp_tx.send(ok(req_id, result));
                }
                Err(e) => {
                    let _ = resp_tx.send(err(req_id, "stream_error", &e));
                }
            }
        }

        SocketCommand::BrowserStreamDisable { req_id, resp_tx } => {
            let mut s = state.borrow_mut();
            if let Some(ref mut bm) = s.browser_manager {
                match bm.send_command("stream_disable", serde_json::json!({})) {
                    Ok(result) => {
                        let _ = resp_tx.send(ok(req_id, result));
                    }
                    Err(e) => {
                        let _ = resp_tx.send(err(req_id, "stream_error", &e));
                    }
                }
            } else {
                let _ = resp_tx.send(err(req_id, "not_running", "No browser session active"));
            }
        }

        SocketCommand::BrowserList { req_id, resp_tx } => {
            let s = state.borrow();
            let surfaces: Vec<serde_json::Value> = s
                .browser_surface_refs
                .iter()
                .map(|(ref_id, uuid)| {
                    serde_json::json!({
                        "ref": format!("surface:{}", ref_id),
                        "uuid": uuid,
                        "status": "registered",
                    })
                })
                .collect();
            let _ = resp_tx.send(ok(req_id, serde_json::json!({"surfaces": surfaces})));
        }

        // -- browser.* generic proxy (P0/P1 parity) --
        SocketCommand::BrowserAction {
            req_id,
            action,
            mut params,
            surface_ref,
            resp_tx,
        } => {
            let s = state.borrow();
            if let Some(ref bm) = s.browser_manager {
                // Resolve surface ref if provided
                if let Some(ref sref) = surface_ref {
                    match resolve_surface_ref(sref, &s.browser_surface_refs) {
                        Ok(uuid) => {
                            if let Some(obj) = params.as_object_mut() {
                                obj.remove("surface_ref");
                                obj.insert("surface_id".to_string(), serde_json::json!(uuid));
                            }
                        }
                        Err((msg, available)) => {
                            let _ = resp_tx.send(json!({
                                "id": req_id,
                                "ok": false,
                                "error": {"code": "surface_not_found", "message": msg},
                                "available": available,
                            }));
                            return;
                        }
                    }
                }
                // Translate cmux CLI action names to agent-browser action names
                let daemon_action = match action.as_str() {
                    "open" => "launch",
                    "goto" => "navigate",
                    "eval" => "evaluate",
                    "gethtml" => "innerhtml",
                    "stream.enable" => "stream_enable",
                    "stream.disable" => "stream_disable",
                    _ => &action,
                };
                match bm.send_command(daemon_action, params) {
                    Ok(result) => {
                        let _ = resp_tx.send(ok(req_id, result));
                    }
                    Err(e) => {
                        let _ = resp_tx.send(err(req_id, "browser_error", &e));
                    }
                }
            } else {
                let _ = resp_tx.send(err(req_id, "not_running", "No browser session active"));
            }
        }

        // -- Tier-2 stubs (D-10) --
        SocketCommand::NotImplemented {
            req_id,
            method,
            resp_tx,
        } => {
            let _ = resp_tx.send(err(
                req_id,
                "not_implemented",
                &format!("{method} is not implemented"),
            ));
        }
    }
}

/// Walk the split tree to find the first Preview node's Picture widget.
fn find_preview_picture(node: &crate::split_engine::SplitNode) -> Option<gtk4::Picture> {
    match node {
        crate::split_engine::SplitNode::Preview { picture, .. } => Some(picture.clone()),
        crate::split_engine::SplitNode::Split { start, end, .. } => {
            find_preview_picture(start).or_else(|| find_preview_picture(end))
        }
        crate::split_engine::SplitNode::Leaf { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- key_to_bytes: the send-key vocabulary is a wire contract --

    #[test]
    fn key_enter_tab_escape() {
        assert_eq!(key_to_bytes("enter").as_deref(), Some(b"\r".as_ref()));
        assert_eq!(key_to_bytes("return").as_deref(), Some(b"\r".as_ref()));
        assert_eq!(key_to_bytes("tab").as_deref(), Some(b"\t".as_ref()));
        assert_eq!(key_to_bytes("escape").as_deref(), Some(b"\x1b".as_ref()));
    }

    #[test]
    fn key_arrows_and_nav() {
        assert_eq!(key_to_bytes("up").as_deref(), Some(b"\x1b[A".as_ref()));
        assert_eq!(key_to_bytes("down").as_deref(), Some(b"\x1b[B".as_ref()));
        assert_eq!(key_to_bytes("pageup").as_deref(), Some(b"\x1b[5~".as_ref()));
        assert_eq!(key_to_bytes("delete").as_deref(), Some(b"\x1b[3~".as_ref()));
    }

    #[test]
    fn key_ctrl_combos() {
        assert_eq!(key_to_bytes("ctrl+c").as_deref(), Some([0x03].as_ref()));
        assert_eq!(key_to_bytes("ctrl+d").as_deref(), Some([0x04].as_ref()));
        assert_eq!(key_to_bytes("ctrl+1"), None); // only letters
    }

    #[test]
    fn key_single_char_passthrough_and_unknown() {
        assert_eq!(key_to_bytes("x").as_deref(), Some(b"x".as_ref()));
        assert_eq!(key_to_bytes("Q").as_deref(), Some(b"Q".as_ref()));
        assert_eq!(key_to_bytes("no-such-key"), None);
    }

    // -- resolve_surface_ref: "surface:N" handles --

    fn refs() -> std::collections::HashMap<u32, String> {
        [
            (1u32, "uuid-one".to_string()),
            (7u32, "uuid-seven".to_string()),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn ref_resolves_known_handle() {
        assert_eq!(
            resolve_surface_ref("surface:7", &refs()),
            Ok("uuid-seven".into())
        );
    }

    #[test]
    fn ref_unknown_handle_lists_available() {
        let err = resolve_surface_ref("surface:9", &refs()).unwrap_err();
        assert!(err.0.contains("surface:9"));
        assert_eq!(err.1.len(), 2);
    }

    #[test]
    fn ref_uuid_passes_through() {
        assert_eq!(
            resolve_surface_ref("f4349bf5-aaaa", &refs()),
            Ok("f4349bf5-aaaa".into())
        );
    }

    // -- layout_to_data: declarative layout compilation --

    #[test]
    fn layout_missing_type_rejected() {
        let mut st = Vec::new();
        assert!(layout_to_data(&json!({}), None, 0, &mut st).is_err());
    }

    #[test]
    fn layout_too_deep_rejected() {
        let mut st = Vec::new();
        assert!(layout_to_data(&json!({"type":"terminal"}), None, 17, &mut st).is_err());
    }

    #[test]
    fn layout_terminal_with_cwd_and_command() {
        let mut st = Vec::new();
        let node = layout_to_data(
            &json!({"type":"terminal","cwd":"/tmp","command":"htop"}),
            None,
            0,
            &mut st,
        )
        .expect("valid terminal");
        assert!(matches!(
            node,
            crate::split_engine::SplitNodeData::Leaf { .. }
        ));
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].1, "cd '/tmp' && htop");
    }

    #[test]
    fn layout_default_cwd_applies() {
        let mut st = Vec::new();
        layout_to_data(&json!({"type":"terminal"}), Some("/proj"), 0, &mut st).expect("valid");
        assert_eq!(st[0].1, "cd '/proj'");
    }

    #[test]
    fn layout_split_recurses() {
        let mut st = Vec::new();
        let node = layout_to_data(
            &json!({
                "type": "split", "direction": "horizontal", "ratio": 0.3,
                "start": {"type": "terminal", "command": "a"},
                "end": {"type": "terminal", "command": "b"},
            }),
            None,
            0,
            &mut st,
        )
        .expect("valid split");
        assert!(matches!(
            node,
            crate::split_engine::SplitNodeData::Split { .. }
        ));
        assert_eq!(st.len(), 2);
    }

    #[test]
    fn layout_agent_terminal_registers_and_boots() {
        let mut st = Vec::new();
        layout_to_data(
            &json!({"type":"terminal","agent":"pi","cwd":"/tmp"}),
            None,
            0,
            &mut st,
        )
        .expect("valid agent terminal");
        assert_eq!(st.len(), 1);
        let cmd = &st[0].1;
        assert!(cmd.contains("cd '/tmp'"), "missing cd: {cmd}");
        assert!(cmd.contains("CMUX_PANE="), "missing CMUX_PANE: {cmd}");
        assert!(cmd.ends_with("pi"), "should boot pi: {cmd}");
    }
}
