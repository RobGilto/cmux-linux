//! Per-surface process stats from /proc, backing `cmux top`
//! (roadmap Phase 3.3).
//!
//! ghostty spawns each pane's shell itself and exposes no PID API, so the
//! mapping surface → process is reconstructed from /proc:
//! - every pane shell is a direct child of cmux-app on its own PTY;
//! - agent panes export CMUX_PANE=<surface-uuid> (set by the agent startup
//!   command), so any descendant carrying that var is an exact match;
//! - remaining shells are matched to remaining panes in creation order
//!   (PTY numbers allocate sequentially) — a documented heuristic.
//!
//! CPU is reported as cumulative seconds (utime+stime of the shell and all
//! its descendants) — cheap (single sample), and ranking by it identifies
//! busy/stuck agents just as well as an instantaneous percentage.

/// One process's numbers, summed over its descendants.
#[derive(Debug, Clone)]
pub struct ProcEntry {
    pub pid: u32,
    /// pts number of the controlling terminal (-1 = none)
    pub pts: i32,
    /// Cumulative CPU seconds (self + descendants)
    pub cpu_secs: f64,
    /// Resident set size in bytes (self + descendants)
    pub rss_bytes: u64,
    /// Shell command line (self only)
    pub cmdline: String,
    /// Surface uuid from a descendant's CMUX_PANE env var, when present
    pub cmux_pane: Option<String>,
}

fn read_stat(pid: u32) -> Option<(u32, i32, u64)> {
    // Returns (ppid, tty_nr→pts, utime+stime jiffies). comm can contain
    // spaces/parens; split on the LAST ')'.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = stat.rsplit_once(')')?.1;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // fields[0]=state, [1]=ppid, [4]=tty_nr, [11]=utime, [12]=stime
    let ppid: u32 = fields.get(1)?.parse().ok()?;
    let tty_nr: i32 = fields.get(4)?.parse().ok()?;
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    // pts devices are major 136..143; minor = pts number.
    let major = (tty_nr >> 8) & 0xfff;
    let pts = if (136..=143).contains(&major) {
        (tty_nr & 0xff) | (((tty_nr >> 20) & 0xfff) << 8)
    } else {
        -1
    };
    Some((ppid, pts, utime + stime))
}

fn read_rss(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/statm"))
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<u64>().ok())
        })
        .map(|pages| pages * 4096)
        .unwrap_or(0)
}

fn read_cmdline(pid: u32) -> String {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|b| {
            b.split(|&c| c == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn read_cmux_pane(pid: u32) -> Option<String> {
    let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    environ
        .split(|&c| c == 0)
        .filter_map(|kv| std::str::from_utf8(kv).ok())
        .find_map(|kv| kv.strip_prefix("CMUX_PANE=").map(String::from))
}

/// Scan /proc once: pid → ppid for every process.
fn process_tree() -> Vec<(u32, u32)> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_string_lossy().parse::<u32>().ok())
        .filter_map(|pid| read_stat(pid).map(|(ppid, _, _)| (pid, ppid)))
        .collect()
}

fn descendants(root: u32, tree: &[(u32, u32)]) -> Vec<u32> {
    let mut out = vec![root];
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for &(pid, ppid) in tree {
            if ppid == parent && !out.contains(&pid) {
                out.push(pid);
                frontier.push(pid);
            }
        }
    }
    out
}

/// Stats for every direct child of this process (= every pane shell, plus
/// helpers like the agent-browser daemon), descendants aggregated.
pub fn child_process_stats() -> Vec<ProcEntry> {
    let hz = 100.0; // USER_HZ on all mainstream Linux configs
    let me = std::process::id();
    let tree = process_tree();
    let mut entries = Vec::new();
    for &(pid, ppid) in &tree {
        if ppid != me {
            continue;
        }
        let Some((_, pts, _)) = read_stat(pid) else {
            continue;
        };
        let mut cpu_jiffies = 0u64;
        let mut rss = 0u64;
        let mut cmux_pane = None;
        for desc in descendants(pid, &tree) {
            if let Some((_, _, jiffies)) = read_stat(desc) {
                cpu_jiffies += jiffies;
            }
            rss += read_rss(desc);
            if cmux_pane.is_none() {
                cmux_pane = read_cmux_pane(desc);
            }
        }
        entries.push(ProcEntry {
            pid,
            pts,
            cpu_secs: cpu_jiffies as f64 / hz,
            rss_bytes: rss,
            cmdline: read_cmdline(pid),
            cmux_pane,
        });
    }
    // Stable order: by pts (creation order), no-tty helpers last.
    entries.sort_by_key(|e| if e.pts < 0 { i32::MAX } else { e.pts });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_stat_is_readable() {
        let me = std::process::id();
        let (ppid, _pts, jiffies) = read_stat(me).expect("own stat");
        assert!(ppid > 0);
        let _ = jiffies; // may be 0 early in process life
        assert!(read_rss(me) > 0);
        assert!(!read_cmdline(me).is_empty());
    }

    #[test]
    fn child_stats_sees_spawned_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("2")
            .spawn()
            .expect("spawn sleep");
        let entries = child_process_stats();
        assert!(
            entries.iter().any(|e| e.pid == child.id()),
            "spawned child not found in {entries:?}"
        );
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn cmux_pane_env_detected() {
        let mut child = std::process::Command::new("sleep")
            .arg("2")
            .env("CMUX_PANE", "test-uuid-123")
            .spawn()
            .expect("spawn sleep");
        let entries = child_process_stats();
        let found = entries
            .iter()
            .find(|e| e.pid == child.id())
            .expect("child present");
        assert_eq!(found.cmux_pane.as_deref(), Some("test-uuid-123"));
        let _ = child.kill();
        let _ = child.wait();
    }
}
