//! cmux CLI — clap-based argument parser and command dispatch.
//!
//! This module is entirely independent of GTK4 and the GUI app.
//! It connects to the cmux-app via Unix socket JSON-RPC.

pub mod discovery;
pub mod doctor;
pub mod format;
pub mod quit;
pub mod socket_client;

pub use socket_client::CliError;

use clap::{Parser, Subcommand};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "cmux", about = "Control cmux terminal multiplexer")]
pub struct Cli {
    /// Path to the cmux socket (overrides discovery)
    #[arg(long, global = true, env = "CMUX_SOCKET")]
    socket: Option<String>,

    /// Output raw JSON responses
    #[arg(long, global = true)]
    json: bool,

    /// Suppress JSON output for browser commands (browser defaults to JSON)
    #[arg(long, global = true)]
    no_json: bool,

    /// Verbose output (connection info to stderr)
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Color mode: always, never, auto
    #[arg(long, global = true, default_value = "auto")]
    color: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Launch cmux-app (detached) and wait until it answers ping.
    /// Idempotent: exits 0 immediately if an instance is already running.
    Launch {
        /// Wipe the saved session before startup (clean slate)
        #[arg(long)]
        fresh: bool,
        /// Seconds to wait for the app to answer ping
        #[arg(long, default_value_t = 15)]
        wait_secs: u64,
        /// Path to the cmux-app binary (default: alongside this CLI, then $PATH)
        #[arg(long, env = "CMUX_APP")]
        app_path: Option<String>,
    },
    /// Quit the running cmux-app gracefully (counterpart of launch).
    /// Escalates: socket quit → SIGTERM → SIGKILL. Idempotent: exits 0
    /// ("not running") when no instance exists.
    Quit,
    /// Ping the running cmux instance
    Ping,
    /// Run self-service diagnostics (socket, GL, config, session, providers)
    Doctor,
    /// Block until a named rendezvous point is signalled (or signal it).
    /// Orchestrator: `cmux wait-for X --timeout 600`. Worker: `cmux wait-for -S X`.
    /// A signal with no waiters is latched for the next waiter.
    #[command(name = "wait-for")]
    WaitFor {
        /// Rendezvous point name
        name: String,
        /// Signal instead of wait
        #[arg(short = 'S', long)]
        signal: bool,
        /// Wait timeout in seconds
        #[arg(long, default_value_t = 300)]
        timeout: u64,
    },
    /// Per-surface process stats (pid, cumulative CPU, memory), busiest first
    Top {
        /// Output format: table, tsv, json
        #[arg(long, default_value = "table")]
        format: String,
    },
    /// Show cmux instance identity (version, platform, pid)
    Identify,
    /// List supported socket commands
    Capabilities,
    /// List all workspaces
    ListWorkspaces,
    /// Show the current workspace
    CurrentWorkspace,
    /// Send an arbitrary JSON-RPC method
    Raw {
        /// The method name (e.g. "workspace.list")
        method: String,
        /// JSON params string
        #[arg(long, default_value = "{}")]
        params: String,
    },

    // -- Workspace management --
    /// Create a new workspace
    NewWorkspace {
        /// Workspace name
        #[arg(long)]
        name: Option<String>,
        /// Working directory for the workspace's terminals
        #[arg(long)]
        cwd: Option<String>,
        /// Declarative layout JSON. Nodes:
        /// {"type":"terminal","cwd":"...","command":"...","agent":"claude"} or
        /// {"type":"split","direction":"horizontal|vertical","ratio":0.5,
        ///  "start":{...},"end":{...}}
        #[arg(long)]
        layout: Option<String>,
        /// Boot a native agent session in the workspace's single pane
        /// (e.g. "claude"), tracked for resume across restarts
        #[arg(long)]
        agent: Option<String>,
    },
    /// Select a workspace by ID
    SelectWorkspace {
        /// Workspace UUID
        id: String,
    },
    /// Close a workspace by ID
    CloseWorkspace {
        /// Workspace UUID
        id: String,
    },
    /// Rename a workspace
    RenameWorkspace {
        /// Workspace UUID
        id: String,
        /// New name
        name: String,
    },
    /// Switch to next workspace
    NextWorkspace,
    /// Switch to previous workspace
    PrevWorkspace,
    /// Switch to last active workspace
    LastWorkspace,
    /// Reorder a workspace
    ReorderWorkspace {
        /// Workspace UUID
        id: String,
        /// Target position (0-indexed)
        position: usize,
    },

    // -- Surface commands --
    /// Assign a workspace to a sidebar group (empty string clears)
    SetGroup {
        /// Group name ("" to remove the workspace from its group)
        group: String,
        /// Workspace UUID (default: active workspace)
        #[arg(long)]
        workspace: Option<String>,
    },
    /// List workspace groups and their members
    ListGroups,

    /// List all surfaces
    ListSurfaces,
    /// Split a surface
    Split {
        /// Split direction: horizontal or vertical
        #[arg(long, default_value = "horizontal")]
        direction: String,
        /// Target surface ID (default: focused)
        #[arg(long)]
        id: Option<String>,
        /// Boot a provider-aware agent session in the new pane
        /// (claude | codex | gemini | pi)
        #[arg(long)]
        agent: Option<String>,
    },
    /// Spawn a new pane with fibonacci/spiral auto-layout — no orientation
    /// argument. The workspace alternates orientation on each call (vertical
    /// divider, then horizontal on what's left, then vertical again, ...),
    /// so an orchestrator/lead/worker fan-out can call this repeatedly
    /// without tracking layout state itself.
    Spawn {
        /// Target surface ID to split (default: focused/active pane)
        #[arg(long)]
        id: Option<String>,
        /// Boot a provider-aware agent session in the new pane
        /// (claude | codex | gemini | pi)
        #[arg(long)]
        agent: Option<String>,
    },
    /// Focus a surface by ID
    FocusSurface {
        /// Surface UUID
        id: String,
    },
    /// Close a surface by ID, or the active pane if no ID is given.
    #[command(alias = "close")]
    CloseSurface {
        /// Surface UUID (default: active pane in the active workspace)
        id: Option<String>,
    },
    /// Send text to a surface
    SendText {
        /// Text to send
        text: String,
        /// Target surface ID (default: focused)
        #[arg(long)]
        id: Option<String>,
    },
    /// Send a key event to a surface
    SendKey {
        /// Key descriptor
        key: String,
        /// Target surface ID (default: focused)
        #[arg(long)]
        id: Option<String>,
    },
    /// Read text from a surface
    ReadText {
        /// Target surface ID (default: focused)
        #[arg(long)]
        id: Option<String>,
        /// Read the full screen buffer including scrollback history,
        /// not just the visible viewport
        #[arg(long)]
        scrollback: bool,
    },
    /// Check surface health
    Health {
        /// Target surface ID (default: focused)
        #[arg(long)]
        id: Option<String>,
    },
    /// Refresh a surface
    Refresh {
        /// Target surface ID (default: focused)
        #[arg(long)]
        id: Option<String>,
    },

    // -- Pane commands --
    /// List all panes
    ListPanes,
    /// Focus a pane
    FocusPane {
        /// Pane ID (default: next)
        id: Option<String>,
    },
    /// Switch to last focused pane
    LastPane,

    // -- Window commands --
    /// List all windows
    ListWindows,
    /// Show current window info
    CurrentWindow,

    // -- Debug commands --
    /// Show layout tree
    Layout,
    /// Type text into the focused terminal
    Type {
        /// Text to type
        text: String,
    },

    // -- Notification commands --
    /// List notifications
    ListNotifications,
    /// Clear a notification
    ClearNotification {
        /// Notification ID
        id: String,
    },

    /// Set a workspace's colored status pill in the sidebar (empty state clears)
    SetStatus {
        /// Status text, e.g. "working", "done", "error"
        state: String,
        /// Pill background color (hex or named), e.g. "#1565C0"
        #[arg(long, default_value = "#546E7A")]
        color: String,
        /// Target workspace UUID (default: active workspace)
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Set a workspace's sidebar progress bar (negative value clears)
    SetProgress {
        /// Progress 0.0–1.0
        value: f64,
        /// Text shown on the bar
        #[arg(long)]
        label: Option<String>,
        /// Target workspace UUID (default: active workspace)
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Set a workspace's one-line sidebar log (empty message clears)
    Log {
        /// Log message
        message: String,
        /// Target workspace UUID (default: active workspace)
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Raise a cmux notification (marks workspace attention, fires
    /// notification.created for event subscribers, desktop notify)
    Notify {
        /// Notification title
        #[arg(long, default_value = "cmux")]
        title: String,
        /// Notification body
        #[arg(long, default_value = "")]
        body: String,
        /// Target workspace UUID (default: active workspace)
        #[arg(long)]
        workspace: Option<String>,
        /// Skip the desktop notification (notify-send)
        #[arg(long)]
        no_desktop: bool,
    },
    /// Install session-capture hooks for agent CLIs (Claude Code, ...)
    #[command(subcommand)]
    Hooks(HooksCommand),
    /// List captured agent sessions (provider, session id, resumable)
    AgentSessions,
    /// Agent session helpers (mostly invoked by hooks)
    #[command(subcommand)]
    Agent(AgentCommand),
    /// Subscribe to the event stream (newline-delimited JSON events)
    Events {
        /// Comma-separated event names to include (e.g.
        /// "notification.created,surface.bell"). Default: all events.
        #[arg(long)]
        name: Option<String>,
        /// Exit after this many events
        #[arg(long)]
        limit: Option<u64>,
        /// Suppress heartbeat lines
        #[arg(long)]
        no_heartbeat: bool,
    },

    // -- Browser subcommand group (agent primary interface) --
    /// Browser automation (agent primary interface)
    #[command(subcommand)]
    Browser(BrowserCommand),
}

/// `cmux hooks <action>` — manage agent session-capture hooks.
#[derive(Subcommand)]
pub enum HooksCommand {
    /// Install session-capture hooks into agent CLI configs
    Setup {
        /// Assume yes to prompts (accepted for macOS parity; always yes here)
        #[arg(long)]
        yes: bool,
    },
}

/// `cmux agent <action>` — agent session helpers.
#[derive(Subcommand)]
pub enum AgentCommand {
    /// Report a captured native session id (invoked by the SessionStart hook;
    /// reads the provider's hook JSON on stdin, uses $CMUX_PANE)
    ReportSession,
}

/// Browser subcommands for `cmux browser <action>` / `cmux browser <surface> <action>`.
#[derive(Subcommand)]
pub enum BrowserCommand {
    /// Open a URL in the browser pane
    Open {
        /// URL to open
        url: String,
        /// Target workspace ID
        #[arg(long)]
        workspace: Option<String>,
    },
    /// List browser surfaces
    List,
    /// Close browser surface(s)
    Close {
        /// Surface reference (surface:N or UUID); closes all if omitted
        #[arg(long)]
        surface: Option<String>,
    },
    /// Take a browser snapshot (accessibility tree / DOM text)
    Snapshot {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// Include interactive element annotations
        #[arg(long)]
        interactive: bool,
        /// Compact output
        #[arg(long)]
        compact: bool,
        /// Maximum depth
        #[arg(long)]
        max_depth: Option<u32>,
    },
    /// Click an element
    Click {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// Target element (e1 or CSS selector)
        target: String,
        /// Take snapshot after action
        #[arg(long)]
        snapshot_after: bool,
    },
    /// Fill an input field (clears first, then types)
    Fill {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// Target element (CSS selector)
        target: String,
        /// Value to fill
        text: String,
        /// Take snapshot after action
        #[arg(long)]
        snapshot_after: bool,
    },
    /// Type text into an element
    #[command(name = "type")]
    BrowserType {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector of the element
        selector: String,
        /// Text to type
        text: String,
    },
    /// Press a key (e.g. "Enter", "Tab", "Escape")
    Press {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// Key name
        key: String,
    },
    /// Hover over an element
    Hover {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector of the element
        selector: String,
    },
    /// Scroll the page
    Scroll {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// Direction: up, down, left, right
        direction: String,
        /// Amount in pixels
        #[arg(long, default_value = "300")]
        amount: i32,
    },
    /// Select an option from a dropdown
    #[command(name = "select")]
    Select {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector of the select element
        selector: String,
        /// Value to select
        value: String,
    },
    /// Evaluate JavaScript in the browser
    Eval {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// JavaScript expression to evaluate
        expression: String,
    },
    /// Wait for a condition
    Wait {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector to wait for
        #[arg(long)]
        selector: Option<String>,
        /// Text to wait for
        #[arg(long)]
        text: Option<String>,
        /// URL substring to wait for
        #[arg(long)]
        url_contains: Option<String>,
        /// Load state to wait for
        #[arg(long)]
        load_state: Option<String>,
        /// JavaScript function to wait for
        #[arg(long)]
        function: Option<String>,
        /// Timeout in milliseconds
        #[arg(long, default_value = "30000")]
        timeout_ms: u64,
    },
    /// Navigate to a URL
    Goto {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// URL to navigate to
        url: String,
    },
    /// Go back in browser history
    Back {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Go forward in browser history
    Forward {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Reload the current page
    Reload {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Get the current page URL
    #[command(name = "get-url")]
    GetUrl {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Get the current page title
    #[command(name = "get-title")]
    GetTitle {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Get text content of an element
    #[command(name = "get-text")]
    GetText {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector of the element
        selector: String,
    },
    /// Get HTML content of an element
    #[command(name = "get-html")]
    GetHtml {
        /// Surface reference (surface:N or UUID)
        surface: String,
        /// CSS selector of the element
        selector: String,
    },
    /// Take a browser screenshot (base64 PNG)
    Screenshot {
        /// Surface reference (surface:N or UUID)
        surface: String,
    },
    /// Enable browser streaming
    #[command(name = "stream-enable")]
    StreamEnable,
    /// Disable browser streaming
    #[command(name = "stream-disable")]
    StreamDisable,
}

/// Run the CLI with the parsed arguments.
pub fn run(cli: Cli) -> Result<(), CliError> {
    // Launch is handled before socket resolution: there is no socket yet.
    if let Commands::Launch {
        fresh,
        wait_secs,
        ref app_path,
    } = cli.command
    {
        return run_launch(&cli, fresh, wait_secs, app_path.as_deref());
    }
    // Doctor likewise must work when the app is down — that's its job.
    if let Commands::Doctor = cli.command {
        return doctor::run_doctor(&cli.socket, cli.json);
    }
    // Quit must also work when the socket is dead (SIGTERM fallback).
    if let Commands::Quit = cli.command {
        return quit::run_quit(&cli.socket);
    }

    // Resolve socket path: --socket flag > discovery > error
    let socket_path = if let Some(ref path) = cli.socket {
        path.clone()
    } else {
        discovery::discover_socket().ok_or_else(|| {
            CliError::ConnectionError("no cmux socket found (is cmux-app running?)".into())
        })?
    };

    // Use longer timeouts for commands that legitimately block server-side.
    let timeout = match &cli.command {
        Commands::Browser(BrowserCommand::Wait { timeout_ms, .. }) => {
            Duration::from_millis(timeout_ms + 5000)
        }
        Commands::WaitFor {
            signal: false,
            timeout,
            ..
        } => Duration::from_secs(timeout + 5),
        _ => Duration::from_secs(5),
    };

    let mut client = socket_client::SocketClient::connect(&socket_path, timeout)?;

    if cli.verbose {
        eprintln!("Connected to {}", socket_path);
    }

    let use_color = format::use_color(&cli.color);

    // Agent report-session: invoked by the provider's SessionStart hook.
    // Reads the hook's JSON from stdin (for session_id) and the CMUX_PANE env
    // var cmux set when it launched the agent (for the target surface).
    if let Commands::Agent(AgentCommand::ReportSession) = cli.command {
        use std::io::Read as _;
        let mut buf = String::new();
        let _ = std::io::stdin().read_to_string(&mut buf);
        let hook: serde_json::Value = serde_json::from_str(&buf).unwrap_or(serde_json::Value::Null);
        let session_id = hook
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let surface = std::env::var("CMUX_PANE").unwrap_or_default();
        if surface.is_empty() || session_id.is_empty() {
            // Not in an agent pane, or no id — succeed quietly so we never
            // break the agent's startup.
            return Ok(());
        }
        let _ = client.call(
            "agent.report_session",
            serde_json::json!({
                "surface": surface,
                "provider": "claude",
                "session_id": session_id,
            }),
        );
        return Ok(());
    }

    // Handle Events separately: it streams newline-delimited JSON until the
    // server closes the connection (--limit) or the pipe breaks.
    if let Commands::Events {
        ref name,
        limit,
        no_heartbeat,
    } = cli.command
    {
        let mut params = serde_json::Map::new();
        if let Some(ref n) = name {
            params.insert("name".into(), serde_json::json!(n));
        }
        if let Some(l) = limit {
            params.insert("limit".into(), serde_json::json!(l));
        }
        params.insert("heartbeat".into(), serde_json::json!(!no_heartbeat));

        use std::io::Write as _;
        let mut stdout = std::io::stdout();
        client.subscribe(serde_json::Value::Object(params), |line| {
            // A broken pipe (e.g. `| head -1`) ends the stream cleanly.
            writeln!(stdout, "{}", line).is_ok() && stdout.flush().is_ok()
        })?;
        return Ok(());
    }

    // Top: sorted, formatted client-side (table/tsv/json).
    if let Commands::Top { ref format } = cli.command {
        let result = client.call("surface.top", serde_json::json!({}))?;
        let mut rows: Vec<serde_json::Value> = result
            .get("top")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        rows.sort_by(|a, b| {
            let cpu =
                |r: &serde_json::Value| r.get("cpu_secs").and_then(|v| v.as_f64()).unwrap_or(-1.0);
            cpu(b)
                .partial_cmp(&cpu(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        match format.as_str() {
            "json" => println!("{}", serde_json::json!({"top": rows})),
            fmt => {
                let sep = if fmt == "tsv" { "\t" } else { "  " };
                println!("SURFACE{sep}WORKSPACE{sep}PID{sep}CPU_SECS{sep}RSS_MB{sep}CMDLINE");
                for r in &rows {
                    let g = |k: &str| {
                        r.get(k)
                            .map(|v| v.to_string().trim_matches('"').to_string())
                            .unwrap_or_default()
                    };
                    let rss_mb =
                        r.get("rss_bytes").and_then(|v| v.as_u64()).unwrap_or(0) / (1024 * 1024);
                    println!(
                        "{}{sep}{}{sep}{}{sep}{}{sep}{}{sep}{}",
                        &g("surface")[..8.min(g("surface").len())],
                        g("workspace"),
                        g("pid"),
                        g("cpu_secs"),
                        rss_mb,
                        g("cmdline"),
                    );
                }
            }
        }
        return Ok(());
    }

    // Handle Raw command separately (dynamic method name)
    let (method_name, result) = if let Commands::Raw {
        ref method,
        ref params,
    } = cli.command
    {
        let params_val: serde_json::Value = serde_json::from_str(params)
            .map_err(|e| CliError::ProtocolError(format!("invalid JSON params: {}", e)))?;
        let result = client.call(method, params_val)?;
        (method.clone(), result)
    } else {
        let (method, params) = command_to_rpc(&cli.command);
        let result = client.call(method, params)?;
        (method.to_string(), result)
    };

    // Browser commands default to JSON; everything else defaults to human-readable
    let json_mode = match &cli.command {
        Commands::Browser(_) => !cli.no_json,
        _ => cli.json,
    };

    // Output formatted result
    let output = format::format_response(&method_name, &result, json_mode, use_color);
    if !output.is_empty() {
        println!("{}", output);
    }

    Ok(())
}

/// Locate cmux-app: explicit path/$CMUX_APP > alongside this CLI > $PATH.
fn find_app_binary(explicit: Option<&str>) -> Option<std::path::PathBuf> {
    if let Some(p) = explicit {
        let path = std::path::PathBuf::from(p);
        return path.is_file().then_some(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        // resolve the symlink first so ~/.local/bin/cmux -> target/debug/cmux
        // finds the cmux-app sitting next to the real binary, not the link.
        let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("cmux-app");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|d| d.join("cmux-app"))
        .find(|c| c.is_file())
}

/// `cmux launch`: spawn cmux-app detached (own session, output to a log
/// file), then poll system.ping until it answers or the deadline passes.
fn run_launch(
    cli: &Cli,
    fresh: bool,
    wait_secs: u64,
    app_path: Option<&str>,
) -> Result<(), CliError> {
    use std::os::unix::process::CommandExt as _;

    let try_ping = |socket_override: &Option<String>| -> Option<String> {
        let path = socket_override
            .clone()
            .or_else(discovery::discover_socket)?;
        let mut c = socket_client::SocketClient::connect(&path, Duration::from_secs(2)).ok()?;
        c.call("system.ping", serde_json::json!({})).ok()?;
        Some(path)
    };

    // Idempotent: an already-answering instance means we're done.
    if let Some(path) = try_ping(&cli.socket) {
        println!("cmux-app already running (socket: {path})");
        return Ok(());
    }

    let app = find_app_binary(app_path).ok_or_else(|| {
        CliError::ConnectionError(
            "cmux-app binary not found (searched --app-path/$CMUX_APP, \
             alongside the CLI, and $PATH)"
                .into(),
        )
    })?;

    // App output goes to a log file, not the caller's terminal — the app
    // outlives this CLI invocation.
    let log_dir = std::env::var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("cmux");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("launch.log");
    let open_log = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
    };

    let mut cmd = std::process::Command::new(&app);
    if fresh {
        cmd.arg("--fresh");
    }
    cmd.stdin(std::process::Stdio::null());
    match (open_log(), open_log()) {
        (Ok(out), Ok(err)) => {
            cmd.stdout(out);
            cmd.stderr(err);
        }
        _ => {
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
        }
    }
    // Detach into its own session so the app survives this CLI (and its
    // terminal) exiting — the in-process equivalent of `setsid`.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().map_err(|e| {
        CliError::ConnectionError(format!("failed to spawn {}: {e}", app.display()))
    })?;

    if cli.verbose {
        eprintln!("Spawned {} (log: {})", app.display(), log_path.display());
    }

    let deadline = std::time::Instant::now() + Duration::from_secs(wait_secs.max(1));
    loop {
        if let Some(path) = try_ping(&cli.socket) {
            println!("cmux-app ready (socket: {path})");
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(CliError::ProtocolError(format!(
                "cmux-app did not answer ping within {wait_secs}s \
                 (see {} for startup errors)",
                log_path.display()
            )));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Map a BrowserCommand variant to its JSON-RPC method and params.
fn browser_command_to_rpc(cmd: &BrowserCommand) -> (&'static str, serde_json::Value) {
    use serde_json::json;
    match cmd {
        BrowserCommand::Open { url, workspace } => {
            let url = if !url.contains("://") {
                format!("https://{}", url)
            } else {
                url.clone()
            };
            ("browser.open", json!({"url": url, "workspace": workspace}))
        }
        BrowserCommand::List => ("browser.list", json!({})),
        BrowserCommand::Close { surface } => ("browser.close", json!({"surface_ref": surface})),
        BrowserCommand::Snapshot {
            surface,
            interactive,
            compact,
            max_depth,
        } => (
            "browser.snapshot",
            json!({
                "surface_ref": surface,
                "interactive": interactive,
                "compact": compact,
                "max_depth": max_depth
            }),
        ),
        BrowserCommand::Click {
            surface,
            target,
            snapshot_after,
        } => (
            "browser.click",
            json!({
                "surface_ref": surface,
                "target": target,
                "snapshot_after": snapshot_after
            }),
        ),
        BrowserCommand::Fill {
            surface,
            target,
            text,
            snapshot_after,
        } => (
            "browser.fill",
            json!({
                "surface_ref": surface,
                "target": target,
                "text": text,
                "snapshot_after": snapshot_after
            }),
        ),
        BrowserCommand::BrowserType {
            surface,
            selector,
            text,
        } => (
            "browser.type",
            json!({
                "surface_ref": surface,
                "selector": selector,
                "text": text
            }),
        ),
        BrowserCommand::Press { surface, key } => {
            ("browser.press", json!({"surface_ref": surface, "key": key}))
        }
        BrowserCommand::Hover { surface, selector } => (
            "browser.hover",
            json!({"surface_ref": surface, "selector": selector}),
        ),
        BrowserCommand::Scroll {
            surface,
            direction,
            amount,
        } => (
            "browser.scroll",
            json!({
                "surface_ref": surface,
                "direction": direction,
                "amount": amount
            }),
        ),
        BrowserCommand::Select {
            surface,
            selector,
            value,
        } => (
            "browser.select",
            json!({
                "surface_ref": surface,
                "selector": selector,
                "value": value
            }),
        ),
        BrowserCommand::Eval {
            surface,
            expression,
        } => (
            "browser.eval",
            json!({"surface_ref": surface, "script": expression}),
        ),
        BrowserCommand::Wait {
            surface,
            selector,
            text,
            url_contains,
            load_state,
            function,
            timeout_ms,
        } => (
            "browser.wait",
            json!({
                "surface_ref": surface,
                "selector": selector,
                "text": text,
                "url_contains": url_contains,
                "load_state": load_state,
                "function": function,
                "timeout_ms": timeout_ms
            }),
        ),
        BrowserCommand::Goto { surface, url } => {
            ("browser.goto", json!({"surface_ref": surface, "url": url}))
        }
        BrowserCommand::Back { surface } => ("browser.back", json!({"surface_ref": surface})),
        BrowserCommand::Forward { surface } => ("browser.forward", json!({"surface_ref": surface})),
        BrowserCommand::Reload { surface } => ("browser.reload", json!({"surface_ref": surface})),
        BrowserCommand::GetUrl { surface } => ("browser.url", json!({"surface_ref": surface})),
        BrowserCommand::GetTitle { surface } => ("browser.title", json!({"surface_ref": surface})),
        BrowserCommand::GetText { surface, selector } => (
            "browser.gettext",
            json!({"surface_ref": surface, "selector": selector}),
        ),
        BrowserCommand::GetHtml { surface, selector } => (
            "browser.gethtml",
            json!({"surface_ref": surface, "selector": selector}),
        ),
        BrowserCommand::Screenshot { surface } => {
            ("browser.screenshot", json!({"surface_ref": surface}))
        }
        BrowserCommand::StreamEnable => ("browser.stream.enable", json!({})),
        BrowserCommand::StreamDisable => ("browser.stream.disable", json!({})),
    }
}

/// Convert a CLI command to a JSON-RPC method and params.
/// Raw is handled separately in run() — panics if called with Raw.
fn command_to_rpc(cmd: &Commands) -> (&'static str, serde_json::Value) {
    use serde_json::{json, Value};
    match cmd {
        // Launch never reaches RPC mapping — run() intercepts it first.
        Commands::Launch { .. } => unreachable!("launch handled in run()"),
        Commands::Ping => ("system.ping", json!({})),
        Commands::Doctor => unreachable!("doctor handled in run()"),
        Commands::Quit => unreachable!("quit handled in run()"),
        Commands::WaitFor {
            name, signal: true, ..
        } => ("rendezvous.signal", json!({"name": name})),
        Commands::WaitFor {
            name,
            signal: false,
            timeout,
        } => (
            "rendezvous.wait",
            json!({"name": name, "timeout_ms": timeout * 1000}),
        ),
        Commands::Top { .. } => ("surface.top", json!({})),
        Commands::Identify => ("system.identify", json!({})),
        Commands::Capabilities => ("system.capabilities", json!({})),
        Commands::ListWorkspaces => ("workspace.list", json!({})),
        Commands::CurrentWorkspace => ("workspace.current", json!({})),

        Commands::Raw { .. } => unreachable!("Raw handled separately"),

        Commands::NewWorkspace {
            name,
            cwd,
            layout,
            agent,
        } => {
            let mut p = serde_json::Map::new();
            if let Some(ref n) = name {
                p.insert("name".into(), json!(n));
            }
            if let Some(ref d) = cwd {
                p.insert("cwd".into(), json!(d));
            }
            // --agent is sugar for a single-terminal agent layout.
            if let Some(ref a) = agent {
                let mut term = serde_json::Map::new();
                term.insert("type".into(), json!("terminal"));
                term.insert("agent".into(), json!(a));
                if let Some(ref d) = cwd {
                    term.insert("cwd".into(), json!(d));
                }
                p.insert("layout".into(), Value::Object(term));
                return ("workspace.create", Value::Object(p));
            }
            if let Some(ref l) = layout {
                match serde_json::from_str::<Value>(l) {
                    Ok(v) => {
                        p.insert("layout".into(), v);
                    }
                    Err(e) => {
                        // Surface the parse error via the server's validation
                        // path (send the raw string; server rejects objects
                        // only, so reject here instead).
                        eprintln!("error: --layout is not valid JSON: {}", e);
                        std::process::exit(2);
                    }
                }
            }
            ("workspace.create", Value::Object(p))
        }
        Commands::SelectWorkspace { id } => ("workspace.select", json!({"id": id})),
        Commands::CloseWorkspace { id } => ("workspace.close", json!({"id": id})),
        Commands::RenameWorkspace { id, name } => {
            ("workspace.rename", json!({"id": id, "name": name}))
        }
        Commands::NextWorkspace => ("workspace.next", json!({})),
        Commands::PrevWorkspace => ("workspace.previous", json!({})),
        Commands::LastWorkspace => ("workspace.last", json!({})),
        Commands::ReorderWorkspace { id, position } => {
            ("workspace.reorder", json!({"id": id, "position": position}))
        }

        Commands::SetGroup { group, workspace } => {
            let mut p = serde_json::Map::new();
            p.insert("group".into(), json!(group));
            if let Some(ref ws) = workspace {
                p.insert("workspace".into(), json!(ws));
            }
            ("workspace.set_group", Value::Object(p))
        }
        Commands::ListGroups => ("workspace_group.list", json!({})),
        Commands::ListSurfaces => ("surface.list", json!({})),
        Commands::Split {
            direction,
            id,
            agent,
        } => {
            let mut p = serde_json::Map::new();
            p.insert("direction".into(), json!(direction));
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            if let Some(ref agent) = agent {
                p.insert("agent".into(), json!(agent));
            }
            ("surface.split", Value::Object(p))
        }
        Commands::Spawn { id, agent } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            if let Some(ref agent) = agent {
                p.insert("agent".into(), json!(agent));
            }
            ("surface.spawn", Value::Object(p))
        }
        Commands::FocusSurface { id } => ("surface.focus", json!({"id": id})),
        Commands::CloseSurface { id } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("surface.close", Value::Object(p))
        }
        Commands::SendText { text, id } => {
            let mut p = serde_json::Map::new();
            p.insert("text".into(), json!(text));
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("surface.send_text", Value::Object(p))
        }
        Commands::SendKey { key, id } => {
            let mut p = serde_json::Map::new();
            p.insert("key".into(), json!(key));
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("surface.send_key", Value::Object(p))
        }
        Commands::ReadText { id, scrollback } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            if *scrollback {
                p.insert("scrollback".into(), json!(true));
            }
            ("surface.read_text", Value::Object(p))
        }
        Commands::Health { id } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("surface.health", Value::Object(p))
        }
        Commands::Refresh { id } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("surface.refresh", Value::Object(p))
        }

        Commands::ListPanes => ("pane.list", json!({})),
        Commands::FocusPane { id } => {
            let mut p = serde_json::Map::new();
            if let Some(ref id) = id {
                p.insert("id".into(), json!(id));
            }
            ("pane.focus", Value::Object(p))
        }
        Commands::LastPane => ("pane.last", json!({})),

        Commands::ListWindows => ("window.list", json!({})),
        Commands::CurrentWindow => ("window.current", json!({})),

        Commands::Layout => ("debug.layout", json!({})),
        Commands::Type { text } => ("debug.type", json!({"text": text})),

        Commands::ListNotifications => ("notification.list", json!({})),
        Commands::ClearNotification { id } => ("notification.clear", json!({"id": id})),
        Commands::Hooks(HooksCommand::Setup { .. }) => ("agent.hooks_setup", json!({})),
        Commands::AgentSessions => ("agent.list", json!({})),
        // Agent(ReportSession) is handled by a dedicated stdin path in run().
        Commands::Agent(AgentCommand::ReportSession) => ("agent.report_session", json!({})),
        Commands::SetStatus {
            state,
            color,
            workspace,
        } => {
            let mut p = serde_json::Map::new();
            p.insert("state".into(), json!(state));
            p.insert("color".into(), json!(color));
            if let Some(ref ws) = workspace {
                p.insert("workspace".into(), json!(ws));
            }
            ("workspace.set_status", Value::Object(p))
        }
        Commands::SetProgress {
            value,
            label,
            workspace,
        } => {
            let mut p = serde_json::Map::new();
            p.insert("value".into(), json!(value));
            if let Some(ref l) = label {
                p.insert("label".into(), json!(l));
            }
            if let Some(ref ws) = workspace {
                p.insert("workspace".into(), json!(ws));
            }
            ("workspace.set_progress", Value::Object(p))
        }
        Commands::Log { message, workspace } => {
            let mut p = serde_json::Map::new();
            p.insert("message".into(), json!(message));
            if let Some(ref ws) = workspace {
                p.insert("workspace".into(), json!(ws));
            }
            ("workspace.log", Value::Object(p))
        }
        Commands::Notify {
            title,
            body,
            workspace,
            no_desktop,
        } => {
            let mut p = serde_json::Map::new();
            p.insert("title".into(), json!(title));
            p.insert("body".into(), json!(body));
            if let Some(ref ws) = workspace {
                p.insert("workspace".into(), json!(ws));
            }
            p.insert("desktop".into(), json!(!no_desktop));
            ("notification.create", Value::Object(p))
        }
        // Events is handled by a dedicated streaming path in run(); this arm
        // is unreachable but keeps the match exhaustive.
        Commands::Events { .. } => ("events.subscribe", json!({})),

        Commands::Browser(cmd) => browser_command_to_rpc(cmd),
    }
}
