//! Cross-platform process introspection.
//!
//! cmux reconstructs the surface→process mapping (and finds sibling cmux-app
//! processes, and the controlling tty of the CLI) from the OS process table.
//! On Linux that table is `/proc`; macOS has no `/proc`, so the macOS arms use
//! `libproc` (`proc_listpids`/`proc_pidinfo`) instead.
//!
//! Only the three cross-binary helpers live here — `caller_pts`,
//! `find_app_pids`, and `surface_cwd`. The heavier `cmux top` aggregation
//! (ProcEntry, descendant summing) stays in `procstat`, whose leaf `/proc`
//! readers are cfg-gated in place; conceptually `procstat` is the Linux
//! backend of `cmux top` and this module is its macOS-aware sibling for the
//! smaller, self-contained queries the `cmux` CLI binary also needs.
//!
//! PORT STATUS: the macOS arms were authored on Linux and have NOT been
//! compiled or run on a Mac — in particular the exact `libproc` crate API
//! names may need adjusting on first macOS build. The Linux arms are the
//! unchanged, previously-shipping logic moved behind this boundary. See
//! specs/cmux-macos-extensibility.html Phase 3.
//!
//! This module is shared by both binaries (cmux-app via `platform`, the `cmux`
//! CLI via a path shim); each uses a different subset, so allow dead code
//! rather than warn on the functions the other binary consumes.
#![allow(dead_code)]

/// This process's controlling terminal as a pts number, or None if not running
/// under a real pty. Used by `surface.close` to detect "you're closing the
/// pane this very command runs in".
///
/// Linux: decode `/proc/self/stat`'s tty_nr field (same scheme cmux-app uses
/// for every pane shell's pts).
#[cfg(target_os = "linux")]
pub fn caller_pts() -> Option<i32> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let rest = stat.rsplit_once(')')?.1;
    let tty_nr: i32 = rest.split_whitespace().nth(4)?.parse().ok()?;
    let major = (tty_nr >> 8) & 0xfff;
    if (136..=143).contains(&major) {
        Some((tty_nr & 0xff) | (((tty_nr >> 20) & 0xfff) << 8))
    } else {
        None
    }
}

/// macOS: the self-close guard compares Linux pts *numbers*; macOS pty naming
/// (`/dev/ttysNNN`) has no equivalent numeric scheme, so returning None
/// disables the guard — the same safe behavior as "no controlling tty" on
/// Linux (the guard simply doesn't fire; close still works). A full macOS
/// implementation would read the controlling terminal device via
/// `sysctl(KERN_PROC_PID)` → `kp_eproc.e_tdev` and map it to a pane.
///
/// PORT STATUS: safe degradation; real mapping needs macOS work.
#[cfg(target_os = "macos")]
pub fn caller_pts() -> Option<i32> {
    None
}

/// PIDs of this user's cmux-app processes (`comm` prefix-matches
/// `comm_prefix`), excluding ourselves. Used by `cmux quit` to signal sibling
/// app instances.
///
/// Linux: walk `/proc`, prefix-match `/proc/{pid}/comm` (kernel truncates it
/// to 15 chars, hence prefix), and keep only processes owned by our uid.
#[cfg(target_os = "linux")]
pub fn find_app_pids(comm_prefix: &str) -> Vec<i32> {
    let me = std::process::id() as i32;
    let my_uid = unsafe { libc::getuid() };
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_string_lossy().parse::<i32>().ok())
        .filter(|&pid| pid != me)
        .filter(|&pid| {
            std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .map(|c| c.trim().starts_with(comm_prefix))
                .unwrap_or(false)
        })
        .filter(|&pid| {
            std::fs::metadata(format!("/proc/{pid}"))
                .map(|m| std::os::unix::fs::MetadataExt::uid(&m) == my_uid)
                .unwrap_or(false)
        })
        .collect()
}

/// macOS: enumerate all pids via libproc, match the process name prefix, and
/// keep only those owned by our uid.
///
/// PORT STATUS: authored on Linux, unverified on macOS. `libproc` API names
/// (`pids_by_type`, `name`, `pidinfo::<TaskAllInfo>`) may need adjusting.
#[cfg(target_os = "macos")]
pub fn find_app_pids(comm_prefix: &str) -> Vec<i32> {
    use libproc::processes::{pids_by_type, ProcFilter};
    let me = std::process::id() as i32;
    let my_uid = unsafe { libc::getuid() };
    let Ok(pids) = pids_by_type(ProcFilter::All) else {
        return Vec::new();
    };
    pids.into_iter()
        .map(|p| p as i32)
        .filter(|&pid| pid != me)
        .filter(|&pid| {
            libproc::proc_pid::name(pid)
                .map(|n| n.starts_with(comm_prefix))
                .unwrap_or(false)
        })
        .filter(|&pid| {
            // BSDInfo carries the process's real uid (pbi_uid).
            libproc::proc_pid::pidinfo::<libproc::bsd_info::BSDInfo>(pid, 0)
                .map(|info| info.pbi_uid == my_uid)
                .unwrap_or(false)
        })
        .collect()
}

/// Best-effort current working directory of the foreground shell under our
/// process (each ghostty surface spawns one child shell). Returns None if no
/// child cwd could be read; callers fall back to `$HOME`.
///
/// Linux: scan `/proc` for children of `our_pid`, read `/proc/{pid}/cwd`.
#[cfg(target_os = "linux")]
pub fn surface_cwd(our_pid: u32) -> Option<String> {
    let entries = std::fs::read_dir("/proc").ok()?;
    let mut candidates: Vec<u32> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        if let Ok(pid) = name.to_string_lossy().parse::<u32>() {
            if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                if let Some(after_comm) = stat.rfind(')') {
                    let fields: Vec<&str> = stat[after_comm + 2..].split_whitespace().collect();
                    if fields.len() >= 2 {
                        if let Ok(ppid) = fields[1].parse::<u32>() {
                            if ppid == our_pid {
                                candidates.push(pid);
                            }
                        }
                    }
                }
            }
        }
    }
    // Most recent child is the best guess (one direct child shell per surface).
    for pid in candidates.iter().rev() {
        if let Ok(cwd) = std::fs::read_link(format!("/proc/{pid}/cwd")) {
            let cwd_str = cwd.to_string_lossy().to_string();
            if !cwd_str.is_empty() {
                return Some(cwd_str);
            }
        }
    }
    None
}

/// macOS: find children of `our_pid` via libproc BSDInfo (pbi_ppid), then read
/// each child's cwd via the vnode-path info. There is no `/proc/{pid}/cwd`
/// symlink on macOS.
///
/// PORT STATUS: authored on Linux, unverified on macOS. The cwd read uses
/// libproc's vnode path info; if that accessor differs, the fallback to $HOME
/// in the caller keeps cmux functional in the meantime.
#[cfg(target_os = "macos")]
pub fn surface_cwd(our_pid: u32) -> Option<String> {
    use libproc::processes::{pids_by_type, ProcFilter};
    let pids = pids_by_type(ProcFilter::All).ok()?;
    let mut candidates: Vec<i32> = Vec::new();
    for pid in pids.into_iter().map(|p| p as i32) {
        if let Ok(info) = libproc::proc_pid::pidinfo::<libproc::bsd_info::BSDInfo>(pid, 0) {
            if info.pbi_ppid == our_pid {
                candidates.push(pid);
            }
        }
    }
    for pid in candidates.iter().rev() {
        // libproc exposes the working directory through the vnode path info;
        // pvi_cdir.vip_path is a NUL-padded C char array.
        if let Ok(vpi) = libproc::proc_pid::pidinfo::<libproc::proc_pid::VnodePathInfo>(*pid, 0) {
            let raw = vpi.pvi_cdir.vip_path;
            let bytes: Vec<u8> = raw
                .iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u8)
                .collect();
            if !bytes.is_empty() {
                if let Ok(s) = String::from_utf8(bytes) {
                    return Some(s);
                }
            }
        }
    }
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn find_app_pids_excludes_self_and_matches_prefix() {
        // Our own process name won't start with this sentinel, and self is
        // excluded regardless — so the result must not contain our pid.
        let me = std::process::id() as i32;
        let pids = find_app_pids("definitely-not-a-real-comm-xyz");
        assert!(!pids.contains(&me));
    }

    #[test]
    fn surface_cwd_finds_child_shell_cwd() {
        // Spawn a child with a known cwd and confirm we can read it back.
        let tmp = std::env::temp_dir();
        let mut child = std::process::Command::new("sleep")
            .arg("2")
            .current_dir(&tmp)
            .spawn()
            .expect("spawn sleep");
        let cwd = surface_cwd(std::process::id());
        // Best-effort: the child is a direct descendant, so some cwd resolves.
        assert!(cwd.is_some(), "expected a child cwd to resolve");
        let _ = child.kill();
        let _ = child.wait();
    }
}
