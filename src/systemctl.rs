//! systemctl: talks to PID 1 through request/reply files; reads state files directly.
use crate::state::{self, RUN};
use crate::unit;
use crate::{journal, util};
use std::fs;


fn pid() -> u32 {
    fs::read_to_string("/proc/self/stat").ok().and_then(|t| t.split_whitespace().next().and_then(|p| p.parse().ok())).unwrap_or(0)
}

fn request(op: &str, names: &[String]) -> Result<(), String> {
    if !fs::metadata(format!("{RUN}/pid")).is_ok() {
        return Err("System has not been booted with mavex-init (PID 1 not running).".into());
    }
    let id = format!("{:x}-{}", util::now_ms(), pid());
    let _ = fs::write(format!("{RUN}/requests/{id}.tmp"), format!("{op} {}\n", names.join(" ")));
    fs::rename(format!("{RUN}/requests/{id}.tmp"), format!("{RUN}/requests/{id}.req")).map_err(|e| format!("cannot send request: {e}"))?;
    let reply = format!("{RUN}/replies/{id}");
    let start = util::now_ms();
    while util::now_ms() - start < 30_000 {
        if let Ok(t) = fs::read_to_string(&reply) {
            let _ = fs::remove_file(&reply);
            return match t.strip_prefix("error: ") {
                Some(e) => Err(e.trim().to_string()),
                None => Ok(()),
            };
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    Err("Connection timed out waiting for init.".into())
}

fn dot(active: &str) -> &'static str {
    match active {
        "active" => "\x1b[1;32m●\x1b[0m",
        "failed" => "\x1b[1;31m●\x1b[0m",
        "activating" | "deactivating" => "\x1b[1;32m●\x1b[0m",
        _ => "○",
    }
}

fn status(name: &str) -> i32 {
    let Some(u) = unit::load(name) else {
        eprintln!("Unit {name} could not be found.");
        return 4;
    };
    let st = state::read(name).unwrap_or_else(|| state::State { active: "inactive".into(), sub: "dead".into(), ..Default::default() });
    let enabled = if u.masked { "masked" } else if is_enabled(name) { "enabled" } else { "disabled" };
    println!("{} {}{}", dot(&st.active), name, if u.description.is_empty() { String::new() } else { format!(" - {}", u.description) });
    println!("     Loaded: loaded ({}; {})", u.path, enabled);
    let since = if st.since_ms > 0 { format!(" since {}; {} ago", util::full(st.since_ms), util::ago(util::now_ms().saturating_sub(st.since_ms))) } else { String::new() };
    println!("     Active: {} ({}){}", st.active, st.sub, since);
    if st.restarts > 0 {
        println!("   Restarts: {}", st.restarts);
    }
    if u.is_timer() {
        let now = util::now_ms();
        if st.next_ms > 0 { println!("    Trigger: {}; {} left", util::full(st.next_ms), util::ago(st.next_ms.saturating_sub(now))); }
        println!("   Triggers: {}", u.trigger_unit());
    }
    if st.pid > 0 {
        println!("   Main PID: {}", st.pid);
    }
    let logs = journal::read_unit(name, 10);
    if !logs.is_empty() {
        println!();
        let host = util::hostname();
        for e in logs {
            println!("{}", journal::format_short(&e, &host));
        }
    }
    match st.active.as_str() {
        "active" => 0,
        "failed" => 3,
        _ => 3,
    }
}

fn is_enabled(name: &str) -> bool {
    let Some(u) = unit::load(name) else { return false };
    let mut hits = u.wanted_by.iter().chain(u.required_by.iter());
    hits.any(|t| {
        ["wants", "requires"].iter().any(|k| {
            UNIT_DIRS.iter().any(|d| fs::symlink_metadata(format!("{d}/{t}.{k}/{name}")).is_ok())
        })
    })
}

use crate::unit::UNIT_DIRS;

fn list_units(all: bool) {
    println!("{:<34} {:<7} {:<11} {:<8} DESCRIPTION", "UNIT", "LOAD", "ACTIVE", "SUB");
    let mut n = 0;
    for name in state::loaded_units() {
        let st = state::read(&name).unwrap_or_default();
        if !all && st.active == "inactive" {
            continue;
        }
        let d = unit::load(&name).map(|u| u.description).unwrap_or_default();
        println!("{} {:<32} {:<7} {:<11} {:<8} {}", if st.active == "failed" { "\x1b[1;31m●\x1b[0m" } else { " " }, name, "loaded", st.active, st.sub, d);
        n += 1;
    }
    println!("\n{n} loaded units listed.");
}

fn list_files() {
    println!("{:<34} {}", "UNIT FILE", "STATE");
    let names = unit::all_unit_names();
    for n in &names {
        let s = match unit::load(n) {
            Some(u) if u.masked => "masked",
            Some(_) if is_enabled(n) => "enabled",
            Some(u) if u.wanted_by.is_empty() && u.required_by.is_empty() => "static",
            _ => "disabled",
        };
        println!("{n:<34} {s}");
    }
    println!("\n{} unit files listed.", names.len());
}

fn enable(names: &[String], on: bool) -> i32 {
    for n in names {
        let Some(u) = unit::load(n) else {
            eprintln!("Failed to enable unit: Unit file {n} does not exist.");
            return 1;
        };
        for t in &u.wanted_by {
            let dir = format!("/etc/systemd/system/{t}.wants");
            let link = format!("{dir}/{n}");
            if on {
                let _ = util::mkdir_p(&dir);
                if fs::symlink_metadata(&link).is_err() && util::symlink(&u.path, &link).is_ok() {
                    println!("Created symlink {link} → {}.", u.path);
                }
            } else if fs::remove_file(&link).is_ok() {
                println!("Removed \"{link}\".");
            }
        }
    }
    0
}

fn mask(names: &[String], on: bool) -> i32 {
    for n in names {
        let link = format!("/etc/systemd/system/{n}");
        if on {
            let _ = fs::remove_file(&link);
            if util::symlink("/dev/null", &link).is_err() {
                eprintln!("Failed to mask {n}");
                return 1;
            }
            println!("Created symlink {link} → /dev/null.");
        } else if fs::read_link(&link).is_ok_and(|p| p.to_string_lossy() == "/dev/null") {
            let _ = fs::remove_file(&link);
            println!("Removed \"{link}\".");
        }
    }
    0
}

fn list_timers() {
    println!("{:<26} {:<12} {:<26} {:<12} {:<22} ACTIVATES", "NEXT", "LEFT", "LAST", "PASSED", "UNIT");
    let now = util::now_ms();
    let mut n = 0;
    for name in state::loaded_units().into_iter().filter(|n| n.ends_with(".timer")) {
        let st = state::read(&name).unwrap_or_default();
        if st.active != "active" {
            continue;
        }
        let u = unit::load(&name).unwrap_or_default();
        let (next, left) = if st.next_ms > 0 { (util::full(st.next_ms), util::ago(st.next_ms.saturating_sub(now))) } else { ("-".into(), "-".into()) };
        let (last, passed) = if st.last_ms > 0 { (util::full(st.last_ms), format!("{} ago", util::ago(now.saturating_sub(st.last_ms)))) } else { ("-".into(), "-".into()) };
        println!("{next:<26} {left:<12} {last:<26} {passed:<12} {name:<22} {}", u.trigger_unit());
        n += 1;
    }
    println!("\n{n} timers listed.");
}

/// reboot / poweroff / halt / shutdown: stops every service, then asks the host to power off or reload.
pub fn power_main(name: &str, args: &[String]) -> i32 {
    let mut op = match name { "reboot" => "reboot", "halt" => "halt", _ => "poweroff" };
    for a in args {
        match a.as_str() {
            "-r" | "--reboot" => op = "reboot",
            "-h" | "-P" | "--poweroff" => op = "poweroff",
            "-H" | "--halt" => op = "halt",
            "-c" => { println!("shutdown: nothing is scheduled"); return 0; }
            "--help" => { println!("{name} [-r|-h|-P|-H] [now]\nStops all services and powers the machine off (or reloads it for reboot)."); return 0; }
            a if name == "shutdown" && !a.starts_with('-') && !matches!(a, "now" | "+0" | "0") && a.parse::<f64>().is_err() && !a.starts_with('+') => {}
            a if name == "shutdown" && !a.starts_with('-') && !matches!(a, "now" | "+0" | "0") && (a.starts_with('+') || a.parse::<f64>().is_ok()) => {
                eprintln!("shutdown: only 'now' is supported (no delayed shutdown yet)");
                return 1;
            }
            _ => {}
        }
    }
    if crate::sys::creds().2 != 0 {
        eprintln!("Failed to {op} system via mavex-init: Access denied (try sudo).");
        return 1;
    }
    match request(op, &[]) {
        Ok(()) => 0,
        Err(e) => { eprintln!("{e}"); 1 }
    }
}

pub fn main(args: &[String]) -> i32 {
    let mut flags = Vec::new();
    let mut pos = Vec::new();
    for a in args {
        if a.starts_with('-') && a.len() > 1 { flags.push(a.as_str()) } else { pos.push(a.clone()) }
    }
    if flags.contains(&"--version") {
        println!("systemd-compatible mavex-init {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    if flags.contains(&"--help") || flags.contains(&"-h") {
        println!("systemctl [COMMAND] [UNIT...]\n\nCommands: list-units, list-timers, list-unit-files, status, start, stop, restart, try-restart,\n  is-active, is-enabled, is-failed, enable, disable, mask, unmask, cat, show,\n  daemon-reload, get-default, reboot, poweroff\n\nOptions: -a/--all  --no-pager  --now  -q/--quiet");
        return 0;
    }
    let all = flags.iter().any(|f| *f == "-a" || *f == "--all");
    let cmd = if pos.is_empty() { "list-units".to_string() } else { pos.remove(0) };
    let names: Vec<String> = pos.iter().map(|n| unit::normalize_name(n)).collect();
    let need = |n: &[String]| -> bool {
        if n.is_empty() {
            eprintln!("Too few arguments.");
            false
        } else {
            true
        }
    };
    let quiet = flags.iter().any(|f| *f == "-q" || *f == "--quiet");
    match cmd.as_str() {
        "list-units" => { list_units(all); 0 }
        "list-unit-files" => { list_files(); 0 }
        "start" | "stop" | "restart" | "try-restart" | "reload" => {
            if !need(&names) { return 1; }
            let op = if cmd == "reload" { "restart" } else { cmd.as_str() };
            match request(op, &names) {
                Ok(()) => 0,
                Err(e) => { eprintln!("{e}"); 1 }
            }
        }
        "status" => {
            if !need(&names) { list_units(false); return 0; }
            names.iter().map(|n| status(n)).max().unwrap_or(0)
        }
        "is-active" | "is-failed" | "is-enabled" => {
            if !need(&names) { return 1; }
            let mut rc = 0;
            for n in &names {
                let a = state::read(n).map(|s| s.active).unwrap_or_else(|| "inactive".into());
                let (word, ok) = match cmd.as_str() {
                    "is-active" => (a.clone(), a == "active"),
                    "is-failed" => (a.clone(), a == "failed"),
                    _ => {
                        let w = match unit::load(n) {
                            None => "not-found",
                            Some(u) if u.masked => "masked",
                            Some(_) if is_enabled(n) => "enabled",
                            Some(u) if u.wanted_by.is_empty() => "static",
                            Some(_) => "disabled",
                        };
                        (w.to_string(), w == "enabled" || w == "static")
                    }
                };
                if !quiet { println!("{word}"); }
                if !ok { rc = if cmd == "is-active" { 3 } else { 1 }; }
            }
            rc
        }
        "enable" | "disable" => { if !need(&names) { return 1; } let rc = enable(&names, cmd == "enable"); if rc == 0 && flags.contains(&"--now") { let _ = request(if cmd == "enable" { "start" } else { "stop" }, &names); } rc }
        "mask" | "unmask" => { if !need(&names) { return 1; } mask(&names, cmd == "mask") }
        "cat" => {
            if !need(&names) { return 1; }
            for n in &names {
                match unit::find_file(n).and_then(|p| fs::read_to_string(&p).ok().map(|t| (p, t))) {
                    Some((p, t)) => println!("# {p}\n{t}"),
                    None => { eprintln!("No files found for {n}."); return 1; }
                }
            }
            0
        }
        "show" => {
            for n in &names {
                let st = state::read(n).unwrap_or_default();
                print!("Id={n}\n{}", st.serialize());
            }
            0
        }
        "daemon-reload" | "daemon-reexec" => match request("daemon-reload", &[]) { Ok(()) => 0, Err(e) => { eprintln!("{e}"); 1 } },
        "get-default" => {
            println!("{}", fs::read_link("/etc/systemd/system/default.target").ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "multi-user.target".into()));
            0
        }
        "reboot" | "poweroff" | "halt" => power_main(&cmd, &[]),
        "list-timers" => { list_timers(); 0 }
        other => { eprintln!("Unknown command verb '{other}'."); 1 }
    }
}
