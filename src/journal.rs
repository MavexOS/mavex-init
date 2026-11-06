//! A small text journal: one `ms \t priority \t unit \t pid \t message` line per entry.
use crate::util;
use std::fs::{self, File, OpenOptions};
use std::io::Write;

pub const DIR: &str = "/var/log/journal";
pub const FILE: &str = "/var/log/journal/system.journal";
const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub struct Entry {
    pub ms: u64,
    pub prio: u8,
    pub unit: String,
    pub pid: u32,
    pub msg: String,
}

pub struct Writer {
    file: Option<File>,
    written: u64,
}

impl Writer {
    pub fn open() -> Writer {
        let _ = crate::util::mkdir_p(DIR);
        let file = OpenOptions::new().create(true).append(true).open(FILE).ok();
        let written = fs::metadata(FILE).map(|m| m.len()).unwrap_or(0);
        Writer { file, written }
    }

    pub fn log(&mut self, prio: u8, unit: &str, pid: u32, msg: &str) {
        let line = format!("{}\t{}\t{}\t{}\t{}\n", util::now_ms(), prio, unit, pid, msg.replace(['\n', '\t'], " "));
        if let Some(f) = self.file.as_mut() {
            if f.write_all(line.as_bytes()).is_ok() {
                self.written += line.len() as u64;
            }
        }
        if self.written > MAX_BYTES {
            self.rotate();
        }
    }

    fn rotate(&mut self) {
        let text = fs::read_to_string(FILE).unwrap_or_default();
        let keep = &text[text.len() / 2..];
        let keep = keep.find('\n').map(|i| &keep[i + 1..]).unwrap_or("");
        if fs::write(FILE, keep).is_ok() {
            self.file = OpenOptions::new().append(true).open(FILE).ok();
            self.written = keep.len() as u64;
        }
    }
}

pub fn parse_line(l: &str) -> Option<Entry> {
    let mut f = l.splitn(5, '\t');
    Some(Entry { ms: f.next()?.parse().ok()?, prio: f.next()?.parse().ok()?, unit: f.next()?.into(), pid: f.next()?.parse().ok()?, msg: f.next()?.into() })
}

pub fn read_all() -> Vec<Entry> {
    fs::read_to_string(FILE).unwrap_or_default().lines().filter_map(parse_line).collect()
}

pub fn read_unit(unit: &str, n: usize) -> Vec<Entry> {
    let mut v: Vec<Entry> = read_all().into_iter().filter(|e| e.unit == unit).collect();
    let drop = v.len().saturating_sub(n);
    v.drain(..drop);
    v
}

pub fn format_short(e: &Entry, host: &str) -> String {
    let tag = if e.pid > 0 { format!("{}[{}]", e.unit, e.pid) } else { e.unit.clone() };
    format!("{} {} {}: {}", util::short(e.ms), host, tag, e.msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let e = parse_line("1762425131000\t6\tfoo.service\t7\thello\tworld").unwrap();
        assert_eq!((e.prio, e.pid, e.unit.as_str()), (6, 7, "foo.service"));
        assert_eq!(e.msg, "hello\tworld");
        assert_eq!(format_short(&e, "mavex"), "Nov 06 10:32:11 mavex foo.service[7]: hello\tworld");
    }
}
