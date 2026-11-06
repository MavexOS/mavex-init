//! Time formatting and small helpers (UTC only).
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];

/// (year, month 1-12, day 1-31) for days since 1970-01-01.
pub fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn parts(ms: u64) -> (i64, u32, u32, u64, u64, u64, usize) {
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let (y, m, d) = civil(days);
    let sod = secs % 86_400;
    (y, m, d, sod / 3600, sod % 3600 / 60, sod % 60, (days % 7) as usize)
}

/// "Nov 06 10:32:11"
pub fn short(ms: u64) -> String {
    let (_, m, d, h, mi, s, _) = parts(ms);
    format!("{} {:02} {:02}:{:02}:{:02}", MONTHS[m as usize - 1], d, h, mi, s)
}

/// "2026-11-06T10:32:11+0000"
pub fn iso(ms: u64) -> String {
    let (y, m, d, h, mi, s, _) = parts(ms);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}+0000")
}

/// "Thu 2026-11-06 10:32:11 UTC"
pub fn full(ms: u64) -> String {
    let (y, m, d, h, mi, s, wd) = parts(ms);
    format!("{} {y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC", DAYS[wd])
}

/// "5s", "2min 3s", "1h 5min"
pub fn ago(delta_ms: u64) -> String {
    let s = delta_ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}min {}s", s / 60, s % 60)
    } else if s < 86_400 {
        format!("{}h {}min", s / 3600, s % 3600 / 60)
    } else {
        format!("{}d {}h", s / 86_400, s % 86_400 / 3600)
    }
}

pub fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "mavex".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates() {
        assert_eq!(full(0), "Thu 1970-01-01 00:00:00 UTC");
        assert_eq!(full(1_762_425_131_000), "Thu 2025-11-06 10:32:11 UTC");
        assert_eq!(short(1_762_425_131_000), "Nov 06 10:32:11");
        assert_eq!(iso(86_400_000), "1970-01-02T00:00:00+0000");
        assert_eq!(ago(125_000), "2min 5s");
    }
}

#[cfg(target_os = "wasi")]
pub fn symlink(target: &str, link: &str) -> std::io::Result<()> {
    #[link(wasm_import_module = "wasi_snapshot_preview1")]
    extern "C" {
        fn path_symlink(old: *const u8, old_n: usize, fd: u32, new: *const u8, new_n: usize) -> u16;
    }
    let rel = link.trim_start_matches('/');
    let e = unsafe { path_symlink(target.as_ptr(), target.len(), 3, rel.as_ptr(), rel.len()) };
    if e == 0 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(e as i32)) }
}
#[cfg(not(target_os = "wasi"))]
pub fn symlink(target: &str, link: &str) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

pub fn mkdir_p(path: &str) -> std::io::Result<()> {
    let mut cur = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        cur.push('/');
        cur.push_str(part);
        if std::fs::metadata(&cur).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        std::fs::create_dir(&cur)?;
    }
    Ok(())
}
