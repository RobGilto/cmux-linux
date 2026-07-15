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

// ---------------------------------------------------------------------------
// Leaf process readers. These are the only platform-specific parts of `cmux
// top`: Linux reads /proc, macOS reads the same facts through libproc. Both
// arms keep identical signatures so the aggregation below
// (descendants/child_process_stats/match_pane_pts) is platform-neutral.
//
// macOS PORT STATUS: the macOS arms were authored on Linux and not compiled on
// a Mac; libproc field names (TaskAllInfo.pbsd.pbi_ppid,
// ptinfo.pti_total_user/pti_resident_size) may need adjusting. Where a fact
// has no cheap macOS equivalent (pts number, another process's CMUX_PANE env)
// the arm safely degrades (-1 / None) — `cmux top` still lists processes and
// their CPU/RSS, only the pts-ordering and exact agent-pane match soften.
// See specs/cmux-macos-extensibility.html Phase 3.

/// Returns (ppid, pts, cpu_ticks) where cpu_ticks / 100 == cumulative CPU
/// seconds (matching Linux jiffies at USER_HZ=100, which child_process_stats
/// divides by).
#[cfg(target_os = "linux")]
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

/// macOS: ppid + cpu from libproc TaskAllInfo. pts is not cheaply available
/// (would need sysctl KERN_PROC_PID → e_tdev), so -1; cpu time is nanoseconds,
/// rescaled to /100-seconds so the shared aggregation's `/hz` yields seconds.
#[cfg(target_os = "macos")]
fn read_stat(pid: u32) -> Option<(u32, i32, u64)> {
    let info = libproc::proc_pid::pidinfo::<libproc::task_info::TaskAllInfo>(pid as i32, 0).ok()?;
    let ppid = info.pbsd.pbi_ppid;
    let cpu_ns = info.ptinfo.pti_total_user + info.ptinfo.pti_total_system;
    // ns → centi-seconds (÷1e7) so downstream ÷100 gives whole seconds.
    Some((ppid, -1, cpu_ns / 10_000_000))
}

#[cfg(target_os = "linux")]
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

/// macOS: resident size straight from libproc TaskAllInfo (already bytes).
#[cfg(target_os = "macos")]
fn read_rss(pid: u32) -> u64 {
    libproc::proc_pid::pidinfo::<libproc::task_info::TaskAllInfo>(pid as i32, 0)
        .map(|info| info.ptinfo.pti_resident_size)
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
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

/// macOS: full argv requires sysctl KERN_PROCARGS2; the executable path (or
/// short name) from libproc is a good-enough label for `cmux top`.
#[cfg(target_os = "macos")]
fn read_cmdline(pid: u32) -> String {
    libproc::proc_pid::pidpath(pid as i32)
        .or_else(|_| libproc::proc_pid::name(pid as i32))
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn read_cmux_pane(pid: u32) -> Option<String> {
    let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    environ
        .split(|&c| c == 0)
        .filter_map(|kv| std::str::from_utf8(kv).ok())
        .find_map(|kv| kv.strip_prefix("CMUX_PANE=").map(String::from))
}

/// macOS: reading another process's environment needs sysctl KERN_PROCARGS2
/// (privileged/awkward). Degrade to None — agent panes then fall back to the
/// creation-order heuristic instead of the exact CMUX_PANE match.
///
/// PORT STATUS: real KERN_PROCARGS2 env read is deferred; needs macOS work.
#[cfg(target_os = "macos")]
fn read_cmux_pane(_pid: u32) -> Option<String> {
    None
}

/// Scan the process table once: pid → ppid for every process.
#[cfg(target_os = "linux")]
fn process_tree() -> Vec<(u32, u32)> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_string_lossy().parse::<u32>().ok())
        .filter_map(|pid| read_stat(pid).map(|(ppid, _, _)| (pid, ppid)))
        .collect()
}

/// macOS: enumerate pids via libproc, read each ppid from BSDInfo.
#[cfg(target_os = "macos")]
fn process_tree() -> Vec<(u32, u32)> {
    use libproc::processes::{pids_by_type, ProcFilter};
    let Ok(pids) = pids_by_type(ProcFilter::All) else {
        return Vec::new();
    };
    pids.into_iter()
        .filter_map(|pid| {
            libproc::proc_pid::pidinfo::<libproc::bsd_info::BSDInfo>(pid as i32, 0)
                .ok()
                .map(|info| (pid, info.pbi_ppid))
        })
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

/// Match each pane uuid to its shell's pts number, using the same env/order
/// heuristic as `cmux top` (see this module's doc comment) — exact via
/// CMUX_PANE for agent panes, creation order for plain shells. Returns only
/// panes that resolved to a live process with a real controlling tty.
///
/// Used by `surface.close` to detect "you're asking to close the pane
/// running this very command" (its shell — and the `cmux close` process
/// itself — dies mid-call, before a response can be sent) and warn instead
/// of silently doing it.
pub fn match_pane_pts(panes: &[(String, String)]) -> std::collections::HashMap<String, i32> {
    let procs = child_process_stats();
    let mut used = vec![false; procs.len()];
    let mut result = std::collections::HashMap::new();
    let mut unmatched: Vec<&str> = Vec::new();
    for (uuid, _ws) in panes {
        match procs
            .iter()
            .position(|p| p.cmux_pane.as_deref() == Some(uuid.as_str()))
        {
            Some(i) => {
                used[i] = true;
                if procs[i].pts >= 0 {
                    result.insert(uuid.clone(), procs[i].pts);
                }
            }
            None => unmatched.push(uuid.as_str()),
        }
    }
    let free: Vec<usize> = (0..procs.len())
        .filter(|&i| !used[i] && procs[i].pts >= 0)
        .collect();
    for (n, &uuid) in unmatched.iter().enumerate() {
        if let Some(&i) = free.get(n) {
            result.insert(uuid.to_string(), procs[i].pts);
        }
    }
    result
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
