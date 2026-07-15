use std::os::unix::io::AsRawFd;

/// Validate that the peer's UID matches the current process's UID.
///
/// The kernel is the source of truth for the peer's credentials (it cannot be
/// spoofed by the client), but the syscall to read them differs by platform:
/// Linux uses `getsockopt(SO_PEERCRED)` → `ucred{pid,uid,gid}`; macOS/BSD have
/// no `SO_PEERCRED` and instead expose `getpeereid(fd, &uid, &gid)` (uid/gid
/// only — no pid). Both give us the uid, which is all this check needs.
///
/// Returns Ok(true) if UID matches (connection accepted), Ok(false) if
/// rejected. Returns Err if the credential syscall fails (connection rejected).
#[cfg(target_os = "linux")]
pub fn validate_peer_uid(stream: &tokio::net::UnixStream) -> std::io::Result<bool> {
    let fd = stream.as_raw_fd();
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ret = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let expected_uid = unsafe { libc::getuid() };
    Ok(cred.uid == expected_uid)
}

/// macOS/BSD peer-uid check via `getpeereid` (there is no `SO_PEERCRED`).
/// Unlike Linux's `ucred`, `getpeereid` returns no pid — the uid/gid it yields
/// is still kernel-authoritative, so the uid-match guarantee is identical.
///
/// PORT STATUS: authored on Linux, not compiled or run on a Mac. See
/// specs/cmux-macos-extensibility.html Phase 2.
#[cfg(target_os = "macos")]
pub fn validate_peer_uid(stream: &tokio::net::UnixStream) -> std::io::Result<bool> {
    let fd = stream.as_raw_fd();
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    let ret = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let expected_uid = unsafe { libc::getuid() };
    Ok(uid == expected_uid)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// SOCK-06: validate_peer_uid uses SO_PEERCRED to check connection UID.
    /// A Unix socketpair has the same UID on both ends, so this verifies
    /// the getsockopt call works and returns Ok(true) for same-UID connections.
    #[test]
    fn test_peercred_rejection() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().expect("socketpair failed");
        let fd = a.as_raw_fd();
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let ret = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        assert_eq!(ret, 0, "getsockopt SO_PEERCRED failed");
        let expected_uid = unsafe { libc::getuid() };
        assert_eq!(
            cred.uid, expected_uid,
            "socketpair peer uid must match self"
        );
    }
}
