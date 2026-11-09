//! mavex-init: PID 1 + systemctl + journalctl for MavexOS, one multicall binary.
mod calendar;
mod init;
mod journal;
mod journalctl;
mod state;
mod sys;
mod systemctl;
mod unit;
mod util;

fn main() {
    if let Ok(d) = std::env::var("PWD") {
        let _ = std::env::set_current_dir(d);
    } else {
        let _ = std::env::set_current_dir("/");
    }
    let mut args: Vec<String> = std::env::args().collect();
    let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    let mut name = base(&args[0]);
    if name == "mavex-init" && args.len() > 1 && ["init", "systemctl", "journalctl", "reboot", "poweroff", "halt", "shutdown"].contains(&args[1].as_str()) {
        args.remove(0);
        name = base(&args[0]);
    }
    let rest = &args[1..];
    let code = match name.as_str() {
        "systemctl" => systemctl::main(rest),
        "journalctl" => journalctl::main(rest),
        "reboot" | "poweroff" | "halt" | "shutdown" => systemctl::power_main(&name, rest),
        _ => {
            init::Init::new().run();
        }
    };
    std::process::exit(code);
}
