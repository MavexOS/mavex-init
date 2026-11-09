//! systemd-style unit files (a practical subset: [Unit], [Service], [Install]).
use std::fs;

pub const UNIT_DIRS: [&str; 2] = ["/etc/systemd/system", "/usr/lib/systemd/system"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Restart {
    #[default]
    No,
    OnFailure,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SvcType {
    #[default]
    Simple,
    Oneshot,
}

#[derive(Clone, Debug, Default)]
pub struct Unit {
    pub name: String,
    pub path: String,
    pub description: String,
    pub after: Vec<String>,
    pub before: Vec<String>,
    pub requires: Vec<String>,
    pub wants: Vec<String>,
    pub wanted_by: Vec<String>,
    pub required_by: Vec<String>,
    pub exec_start: Vec<String>,
    pub typ: SvcType,
    pub restart: Restart,
    pub restart_ms: u64,
    pub remain_after_exit: bool,
    pub working_dir: String,
    pub env: Vec<String>,
    pub masked: bool,
    pub user: String,
    pub group: String,
    pub trigger: String,
    pub on_boot_ms: Option<u64>,
    pub on_active_ms: Option<u64>,
    pub on_unit_active_ms: Option<u64>,
    pub calendar: Vec<String>,
}

impl Unit {
    pub fn is_target(&self) -> bool {
        self.name.ends_with(".target")
    }
    pub fn is_timer(&self) -> bool {
        self.name.ends_with(".timer")
    }
    pub fn trigger_unit(&self) -> String {
        if self.trigger.is_empty() { format!("{}.service", self.name.trim_end_matches(".timer")) } else { self.trigger.clone() }
    }
    pub fn is_service(&self) -> bool {
        self.name.ends_with(".service")
    }
}

pub fn normalize_name(n: &str) -> String {
    if n.contains('.') { n.to_string() } else { format!("{n}.service") }
}

pub fn split_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut have = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), '\\') => {
                if let Some(n) = it.next() {
                    cur.push(match n { 'n' => '\n', 't' => '\t', o => o });
                }
            }
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                have = true;
            }
            (None, '\\') => {
                if let Some(n) = it.next() {
                    cur.push(n);
                    have = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if have || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if have || !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn parse_duration_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mult = match unit.trim() {
        "" | "s" | "sec" | "second" | "seconds" => 1000.0,
        "ms" | "msec" => 1.0,
        "min" | "m" | "minute" | "minutes" => 60_000.0,
        "h" | "hr" | "hour" | "hours" => 3_600_000.0,
        _ => return None,
    };
    Some((n * mult) as u64)
}

fn truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "yes" | "true" | "on")
}

pub fn parse(name: &str, path: &str, text: &str) -> Unit {
    let mut u = Unit { name: name.into(), path: path.into(), restart_ms: 100, ..Default::default() };
    let mut section = String::new();
    let mut joined = String::new();
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        if let Some(stripped) = raw.strip_suffix('\\') {
            joined.push_str(stripped);
            joined.push(' ');
            continue;
        }
        joined.push_str(raw);
        lines.push(std::mem::take(&mut joined));
    }
    for line in lines {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
            continue;
        }
        if l.starts_with('[') && l.ends_with(']') {
            section = l[1..l.len() - 1].to_string();
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let list = |dst: &mut Vec<String>| dst.extend(split_words(v).into_iter().map(|w| normalize_name(&w)));
        match (section.as_str(), k) {
            ("Unit", "Description") => u.description = v.into(),
            ("Unit", "After") => list(&mut u.after),
            ("Unit", "Before") => list(&mut u.before),
            ("Unit", "Requires") => list(&mut u.requires),
            ("Unit", "Wants") => list(&mut u.wants),
            ("Install", "WantedBy") => list(&mut u.wanted_by),
            ("Install", "RequiredBy") => list(&mut u.required_by),
            ("Service", "ExecStart") => {
                let mut w = split_words(v);
                if let Some(first) = w.first_mut() {
                    *first = first.trim_start_matches(['-', '@', '+', '!', ':']).to_string();
                }
                u.exec_start = w;
            }
            ("Service", "Type") => u.typ = if v == "oneshot" { SvcType::Oneshot } else { SvcType::Simple },
            ("Service", "Restart") => {
                u.restart = match v {
                    "always" | "on-success" | "on-abnormal" | "on-abort" | "on-watchdog" => Restart::Always,
                    "on-failure" => Restart::OnFailure,
                    _ => Restart::No,
                }
            }
            ("Service", "RestartSec") => u.restart_ms = parse_duration_ms(v).unwrap_or(100),
            ("Service", "RemainAfterExit") => u.remain_after_exit = truthy(v),
            ("Service", "WorkingDirectory") => u.working_dir = v.into(),
            ("Service", "User") => u.user = v.into(),
            ("Service", "Group") => u.group = v.into(),
            ("Timer", "OnBootSec") | ("Timer", "OnStartupSec") => u.on_boot_ms = parse_duration_ms(v),
            ("Timer", "OnActiveSec") => u.on_active_ms = parse_duration_ms(v),
            ("Timer", "OnUnitActiveSec") | ("Timer", "OnUnitInactiveSec") => u.on_unit_active_ms = parse_duration_ms(v),
            ("Timer", "OnCalendar") => u.calendar.push(v.into()),
            ("Timer", "Unit") => u.trigger = normalize_name(v),
            ("Service", "Environment") => u.env.extend(split_words(v).into_iter().filter(|w| w.contains('='))),
            _ => {}
        }
    }
    u
}

pub fn find_file(name: &str) -> Option<String> {
    UNIT_DIRS.iter().map(|d| format!("{d}/{name}")).find(|p| fs::symlink_metadata(p).is_ok())
}

pub fn is_masked(path: &str) -> bool {
    fs::read_link(path).map(|t| t.to_string_lossy() == "/dev/null").unwrap_or(false)
}

pub fn load(name: &str) -> Option<Unit> {
    let path = find_file(name)?;
    if is_masked(&path) {
        return Some(Unit { name: name.into(), path, masked: true, ..Default::default() });
    }
    let text = fs::read_to_string(&path).ok()?;
    Some(parse(name, &path, &text))
}

/// Units pulled in through `<unit>.wants/` and `<unit>.requires/` directories.
pub fn dir_deps(name: &str) -> (Vec<String>, Vec<String>) {
    let read = |suffix: &str| {
        let mut v: Vec<String> = UNIT_DIRS
            .iter()
            .filter_map(|d| fs::read_dir(format!("{d}/{name}.{suffix}")).ok())
            .flat_map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect();
        v.sort();
        v.dedup();
        v
    };
    (read("wants"), read("requires"))
}

pub fn all_unit_names() -> Vec<String> {
    let mut v: Vec<String> = UNIT_DIRS
        .iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flat_map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.ends_with(".service") || n.ends_with(".target") || n.ends_with(".timer"))
        .collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_service() {
        let u = parse("x.service", "/x", "[Unit]\nDescription=Test thing\nAfter=network.target basic.target\n[Service]\nType=oneshot\nExecStart=-/bin/echo \"hello world\" 'a b'\nRestart=on-failure\nRestartSec=2s\nEnvironment=A=1 \"B=two words\"\n[Install]\nWantedBy=multi-user.target\n");
        assert_eq!(u.description, "Test thing");
        assert_eq!(u.after, ["network.target", "basic.target"]);
        assert_eq!(u.exec_start, ["/bin/echo", "hello world", "a b"]);
        assert_eq!(u.typ, SvcType::Oneshot);
        assert_eq!(u.restart, Restart::OnFailure);
        assert_eq!(u.restart_ms, 2000);
        assert_eq!(u.env, ["A=1", "B=two words"]);
        assert_eq!(u.wanted_by, ["multi-user.target"]);
    }
    #[test]
    fn durations() {
        assert_eq!(parse_duration_ms("500ms"), Some(500));
        assert_eq!(parse_duration_ms("1min"), Some(60_000));
        assert_eq!(parse_duration_ms("3"), Some(3000));
        assert_eq!(normalize_name("foo"), "foo.service");
    }
}
