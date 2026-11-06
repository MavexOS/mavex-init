use crate::{journal, util};

pub fn main(args: &[String]) -> i32 {
    let (mut unit, mut n, mut follow, mut all) = (None, None::<usize>, false, true);
    let mut raw = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-u" | "--unit" => { i += 1; unit = args.get(i).map(|s| crate::unit::normalize_name(s)); }
            "-n" | "--lines" => { i += 1; n = args.get(i).and_then(|s| s.parse().ok()); }
            "-f" | "--follow" => { follow = true; if n.is_none() { n = Some(10); } }
            "-o" => { i += 1; raw = args.get(i).is_some_and(|s| s == "cat"); }
            "--no-pager" | "-q" | "--quiet" | "-a" => {}
            "--version" => { println!("journalctl (mavex-init {})", env!("CARGO_PKG_VERSION")); return 0; }
            "--help" | "-h" => { println!("journalctl [-u UNIT] [-n N] [-f] [-o cat] [--no-pager]"); return 0; }
            _ if a.starts_with("--unit=") => unit = Some(crate::unit::normalize_name(&a[7..])),
            _ if a.starts_with("-n") && a[2..].parse::<usize>().is_ok() => n = a[2..].parse().ok(),
            _ if a.starts_with("--lines=") => n = a[8..].parse().ok(),
            _ => { eprintln!("journalctl: unknown option {a}"); return 1; }
        }
        i += 1;
    }
    let _ = &mut all;
    let host = util::hostname();
    let show = |e: &journal::Entry| if raw { println!("{}", e.msg) } else { println!("{}", journal::format_short(e, &host)) };
    let mut entries: Vec<journal::Entry> = journal::read_all().into_iter().filter(|e| unit.as_ref().is_none_or(|u| &e.unit == u)).collect();
    if let Some(n) = n {
        let drop = entries.len().saturating_sub(n);
        entries.drain(..drop);
    }
    if entries.is_empty() && !follow {
        println!("-- No entries --");
    }
    let mut seen = journal::read_all().len();
    for e in &entries {
        show(e);
    }
    if follow {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let all_e = journal::read_all();
            if all_e.len() < seen {
                seen = 0;
            }
            for e in all_e.iter().skip(seen) {
                if unit.as_ref().is_none_or(|u| &e.unit == u) {
                    show(e);
                }
            }
            seen = all_e.len();
        }
    }
    0
}
