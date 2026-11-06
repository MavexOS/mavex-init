//! Bridge to the MavexOS host imports (process spawning, pipes, polling, kill).
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

#[cfg(target_os = "wasi")]
mod imp {
    #[link(wasm_import_module = "mavex")]
    unsafe extern "C" {
        pub fn pipe(out: *mut u32) -> i32;
        pub fn spawn(prog: *const u8, prog_n: usize, argv: *const u8, argv_n: usize, env: *const u8, env_n: usize, cwd: *const u8, cwd_n: usize, s0: i32, s1: i32, s2: i32) -> i32;
        pub fn poll_child(pid: u32) -> i32;
        pub fn kill(pid: u32, sig: i32) -> i32;
        pub fn getcreds(out: *mut u32) -> i32;
    }
    #[link(wasm_import_module = "wasi_snapshot_preview1")]
    unsafe extern "C" {
        pub fn poll_oneoff(inp: *const u8, out: *mut u8, n: usize, nev: *mut usize) -> i32;
    }
}

#[cfg(not(target_os = "wasi"))]
mod imp {
    pub unsafe fn pipe(_: *mut u32) -> i32 { -1 }
    pub unsafe fn spawn(_: *const u8, _: usize, _: *const u8, _: usize, _: *const u8, _: usize, _: *const u8, _: usize, _: i32, _: i32, _: i32) -> i32 { -1 }
    pub unsafe fn poll_child(_: u32) -> i32 { -1 }
    pub unsafe fn kill(_: u32, _: i32) -> i32 { -1 }
    pub unsafe fn getcreds(_: *mut u32) -> i32 { -1 }
    pub unsafe fn poll_oneoff(_: *const u8, _: *mut u8, _: usize, _: *mut usize) -> i32 { -1 }
}

pub fn pipe_new() -> Result<(OwnedFd, OwnedFd), i32> {
    let mut out = [0u32; 2];
    let r = unsafe { imp::pipe(out.as_mut_ptr()) };
    if r < 0 {
        return Err(r);
    }
    unsafe { Ok((OwnedFd::from_raw_fd(out[0] as RawFd), OwnedFd::from_raw_fd(out[1] as RawFd))) }
}

fn join(v: &[String]) -> Vec<u8> {
    let mut b = Vec::new();
    for s in v {
        b.extend_from_slice(s.as_bytes());
        b.push(0);
    }
    b
}

/// stdio entries: >= 0 duplicates that fd of this process, -1 terminal, -2 null.
pub fn spawn_proc(prog: &str, argv: &[String], env: &[String], cwd: &str, stdio: [i32; 3]) -> Result<u32, i32> {
    let (a, e) = (join(argv), join(env));
    let r = unsafe { imp::spawn(prog.as_ptr(), prog.len(), a.as_ptr(), a.len(), e.as_ptr(), e.len(), cwd.as_ptr(), cwd.len(), stdio[0], stdio[1], stdio[2]) };
    if r < 0 { Err(r) } else { Ok(r as u32) }
}

/// Reaps the child if it has exited and returns its exit code.
pub fn poll_child(pid: u32) -> Option<i32> {
    let r = unsafe { imp::poll_child(pid) };
    if r == -1 { None } else { Some(r) }
}

/// Signals a process and all of its descendants (0 on success).
pub fn kill_tree(pid: u32, sig: i32) -> i32 {
    unsafe { imp::kill(pid, sig) }
}

/// (uid, gid, euid, egid) of the calling process.
pub fn creds() -> (u32, u32, u32, u32) {
    let mut o = [0u32; 4];
    unsafe { imp::getcreds(o.as_mut_ptr()) };
    (o[0], o[1], o[2], o[3])
}

/// Waits until one of `fds` is readable (or hung up) or `timeout_ms` passes; returns the ready indices.
pub fn poll_read(fds: &[RawFd], timeout_ms: u64) -> Vec<usize> {
    let n = fds.len() + 1;
    let mut subs = vec![0u8; n * 48];
    for (i, fd) in fds.iter().enumerate() {
        let s = &mut subs[i * 48..(i + 1) * 48];
        s[0..8].copy_from_slice(&(i as u64).to_le_bytes());
        s[8] = 1;
        s[16..20].copy_from_slice(&(*fd as u32).to_le_bytes());
    }
    let c = &mut subs[fds.len() * 48..];
    c[0..8].copy_from_slice(&u64::MAX.to_le_bytes());
    c[8] = 0;
    c[16..20].copy_from_slice(&1u32.to_le_bytes());
    c[24..32].copy_from_slice(&(timeout_ms * 1_000_000).to_le_bytes());
    let mut out = vec![0u8; n * 32];
    let mut nev = 0usize;
    let r = unsafe { imp::poll_oneoff(subs.as_ptr(), out.as_mut_ptr(), n, &mut nev) };
    if r != 0 {
        std::thread::sleep(std::time::Duration::from_millis(timeout_ms));
        return Vec::new();
    }
    let mut ready = Vec::new();
    for k in 0..nev {
        let ud = u64::from_le_bytes(out[k * 32..k * 32 + 8].try_into().unwrap());
        if (ud as usize) < fds.len() {
            ready.push(ud as usize);
        }
    }
    ready
}
