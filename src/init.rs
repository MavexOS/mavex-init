//! PID 1: starts units from the default target, supervises services, captures their output
//! into the journal and serves systemctl requests dropped into /run/mavex-init/requests.
use crate::journal::Writer;
use crate::state::{State, RUN};
use crate::unit::{self, Restart, SvcType, Unit};
use crate::{sys, util};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::os::fd::AsRawFd;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    Inactive,
    Activating,
    Active,
    Deactivating,
    Failed,
}

impl Act {
    fn word(self) -> &'static str {
        match self {
            Act::Inactive => "inactive",
            Act::Activating => "activating",
            Act::Active => "active",
            Act::Deactivating => "deactivating",
            Act::Failed => "failed",
        }
    }
}

struct Svc {
    unit: Unit,
    act: Act,
    sub: &'static str,
    pid: Option<u32>,
    rd: Option<File>,
    buf: Vec<u8>,
    since: u64,
    restarts: u32,
    result: String,
    code: i32,
    restart_at: Option<u64>,
    stopping: bool,
    stop_deadline: u64,
    killed9: bool,
    starts: Vec<u64>,
    published: String,
}

enum Phase {
    Stopping,
    Starting,
    Done,
}

struct Job {
    id: String,
    op: String,
    names: Vec<String>,
    txn: Vec<String>,
    phase: Phase,
}

pub struct Init {
    svcs: BTreeMap<String, Svc>,
    pending: Vec<String>,
    jobs: Vec<Job>,
    j: Writer,
    started: u32,
}

const DEFAULT_UNITS: &[(&str, &str)] = &[
    ("basic.target", include_str!("../units/basic.target")),
    ("multi-user.target", include_str!("../units/multi-user.target")),
    ("mavex-heartbeat.service", include_str!("../units/mavex-heartbeat.service")),
];

const BASE_ENV: &[&str] = &["PATH=/bin", "HOME=/root", "LANG=en_US.UTF-8", "TERM=dumb"];

fn ensure_dirs() {
    for d in [
        "/run/mavex-init/requests",
        "/run/mavex-init/replies",
        "/run/mavex-init/units",
        "/var/log/journal",
        "/etc/systemd/system",
        "/usr/lib/systemd/system",
    ] {
        let _ = fs::create_dir_all(d);
    }
}

fn install_defaults() {
    for (name, text) in DEFAULT_UNITS {
        let p = format!("/usr/lib/systemd/system/{name}");
        if fs::symlink_metadata(&p).is_err() {
            let _ = fs::write(&p, text);
        }
    }
    let default = "/etc/systemd/system/default.target";
    if fs::symlink_metadata(default).is_err() {
        let _ = util::symlink("/usr/lib/systemd/system/multi-user.target", default);
    }
    let wants = "/etc/systemd/system/multi-user.target.wants";
    if fs::symlink_metadata(wants).is_err() {
        let _ = fs::create_dir_all(wants);
        let _ = util::symlink("/usr/lib/systemd/system/mavex-heartbeat.service", &format!("{wants}/mavex-heartbeat.service"));
    }
}

fn default_target() -> String {
    fs::read_link("/etc/systemd/system/default.target")
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "multi-user.target".into())
}

fn new_svc(unit: Unit) -> Svc {
    Svc {
        unit,
        act: Act::Inactive,
        sub: "dead",
        pid: None,
        rd: None,
        buf: Vec::new(),
        since: 0,
        restarts: 0,
        result: "success".into(),
        code: 0,
        restart_at: None,
        stopping: false,
        stop_deadline: 0,
        killed9: false,
        starts: Vec::new(),
        published: String::new(),
    }
}

fn settled(s: &Svc) -> bool {
    match s.act {
        Act::Active | Act::Inactive | Act::Failed => true,
        Act::Activating => s.sub == "auto-restart",
        Act::Deactivating => false,
    }
}

impl Init {
    pub fn new() -> Init {
        ensure_dirs();
        install_defaults();
        Init { svcs: BTreeMap::new(), pending: Vec::new(), jobs: Vec::new(), j: Writer::open(), started: 0 }
    }

    fn log(&mut self, prio: u8, unit: &str, pid: u32, msg: &str) {
        self.j.log(prio, unit, pid, msg);
    }

    fn ilog(&mut self, msg: &str) {
        self.j.log(6, "mavex-init", 1, msg);
    }

    fn get(&mut self, name: &str) -> Option<&mut Svc> {
        if !self.svcs.contains_key(name) {
            let u = unit::load(name)?;
            self.svcs.insert(name.to_string(), new_svc(u));
        }
        self.svcs.get_mut(name)
    }

    fn desc(&self, name: &str) -> String {
        match self.svcs.get(name) {
            Some(s) if !s.unit.description.is_empty() => format!("{name} - {}", s.unit.description),
            _ => name.to_string(),
        }
    }

    fn deps(&mut self, name: &str) -> Vec<String> {
        let Some(s) = self.get(name) else { return Vec::new() };
        let (mut v, req) = (s.unit.wants.clone(), s.unit.requires.clone());
        let (dw, dr) = unit::dir_deps(name);
        v.extend(req);
        v.extend(dw);
        v.extend(dr);
        v
    }

    fn collect(&mut self, name: &str, out: &mut Vec<String>, stack: &mut Vec<String>) {
        if out.iter().any(|n| n == name) || stack.iter().any(|n| n == name) {
            return;
        }
        if self.get(name).is_none() {
            self.ilog(&format!("Dependency {name} not found, ignoring."));
            return;
        }
        stack.push(name.to_string());
        for d in self.deps(name) {
            self.collect(&d, out, stack);
        }
        stack.pop();
        out.push(name.to_string());
    }

    fn enqueue_start(&mut self, names: &[String]) -> Vec<String> {
        let mut order = Vec::new();
        for n in names {
            self.collect(n, &mut order, &mut Vec::new());
        }
        for n in &order {
            let queue = match self.svcs.get(n) {
                Some(s) => !s.unit.masked && matches!(s.act, Act::Inactive | Act::Failed) && !self.pending.contains(n),
                None => false,
            };
            if queue {
                if let Some(s) = self.svcs.get_mut(n) {
                    s.act = Act::Activating;
                    s.sub = "start-pending";
                    s.result = "success".into();
                }
                self.pending.push(n.clone());
            }
        }
        order
    }

    fn blocked(&self, name: &str) -> bool {
        let Some(u) = self.svcs.get(name).map(|s| &s.unit) else { return false };
        self.pending.iter().filter(|p| *p != name).any(|p| {
            u.after.contains(p) || self.svcs.get(p).is_some_and(|s| s.unit.before.iter().any(|b| b == name))
        }) || u.after.iter().any(|a| self.svcs.get(a).is_some_and(|s| s.act == Act::Activating && s.sub != "auto-restart" && s.sub != "start-pending"))
    }

    fn schedule_starts(&mut self) {
        let mut i = 0;
        while i < self.pending.len() {
            let name = self.pending[i].clone();
            if self.blocked(&name) {
                i += 1;
                continue;
            }
            self.pending.remove(i);
            let failed_dep = self.svcs.get(&name).and_then(|s| s.unit.requires.iter().find(|r| self.svcs.get(*r).is_some_and(|d| d.act == Act::Failed)).cloned());
            if let Some(dep) = failed_dep {
                if let Some(s) = self.svcs.get_mut(&name) {
                    s.act = Act::Failed;
                    s.sub = "dependency";
                    s.result = "dependency".into();
                }
                self.log(4, &name, 0, &format!("Dependency failed for {name}."));
                let _ = dep;
                continue;
            }
            self.start_unit(&name);
        }
    }

    fn start_unit(&mut self, name: &str) {
        if let Some(fresh) = unit::load(name) {
            if let Some(s) = self.svcs.get_mut(name) {
                s.unit = fresh;
            }
        }
        let now = util::now_ms();
        let Some(s) = self.svcs.get_mut(name) else { return };
        s.restart_at = None;
        s.stopping = false;
        s.killed9 = false;
        s.rd = None;
        s.buf.clear();
        if s.unit.is_target() || !s.unit.is_service() {
            s.act = Act::Active;
            s.sub = "active";
            s.since = now;
            let d = self.desc(name);
            self.started += 1;
            self.log(6, "mavex-init", 1, &format!("Reached target {d}."));
            return;
        }
        if s.unit.exec_start.is_empty() {
            s.act = Act::Failed;
            s.sub = "failed";
            s.result = "exec-failed".into();
            self.log(3, name, 0, "Service has no ExecStart= setting, refusing.");
            return;
        }
        s.starts.retain(|t| now.saturating_sub(*t) < 10_000);
        s.starts.push(now);
        let argv = s.unit.exec_start.clone();
        let mut env: Vec<String> = BASE_ENV.iter().map(|e| e.to_string()).collect();
        env.extend(s.unit.env.clone());
        let cwd = if s.unit.working_dir.is_empty() { "/".to_string() } else { s.unit.working_dir.clone() };
        let spawned = sys::pipe_new().map_err(|e| e).and_then(|(r, w)| {
            let fd = w.as_raw_fd();
            let res = sys::spawn_proc(&argv[0], &argv, &env, &cwd, [-2, fd, fd]);
            drop(w);
            res.map(|pid| (pid, r))
        });
        match spawned {
            Ok((pid, r)) => {
                s.pid = Some(pid);
                s.rd = Some(File::from(r));
                s.since = now;
                if s.unit.typ == SvcType::Oneshot {
                    s.act = Act::Activating;
                    s.sub = "start";
                } else {
                    s.act = Act::Active;
                    s.sub = "running";
                    self.started += 1;
                }
                let d = self.desc(name);
                let verb = if self.svcs.get(name).is_some_and(|s| s.unit.typ == SvcType::Oneshot) { "Starting" } else { "Started" };
                self.log(6, "mavex-init", 1, &format!("{verb} {d}."));
            }
            Err(e) => {
                s.act = Act::Failed;
                s.sub = "failed";
                s.result = "exec-failed".into();
                s.code = 203;
                self.log(3, name, 0, &format!("Failed to execute {}: errno {}", argv[0], -e));
                self.log(3, "mavex-init", 1, &format!("{name}: Failed with result 'exec-failed'."));
            }
        }
    }

    fn stop_unit(&mut self, name: &str) {
        self.pending.retain(|p| p != name);
        let now = util::now_ms();
        let Some(s) = self.svcs.get_mut(name) else { return };
        match s.act {
            Act::Inactive | Act::Failed | Act::Deactivating => {}
            _ => match s.pid {
                Some(pid) => {
                    s.act = Act::Deactivating;
                    s.sub = "stop-sigterm";
                    s.stopping = true;
                    s.stop_deadline = now + 5000;
                    s.restart_at = None;
                    sys::kill_tree(pid, 15);
                    let d = self.desc(name);
                    self.log(6, "mavex-init", 1, &format!("Stopping {d}..."));
                }
                None => {
                    s.act = Act::Inactive;
                    s.sub = "dead";
                    s.restart_at = None;
                    s.since = now;
                    let d = self.desc(name);
                    self.log(6, "mavex-init", 1, &format!("Stopped {d}."));
                }
            },
        }
    }

    fn on_exit(&mut self, name: &str, code: i32) {
        let now = util::now_ms();
        let Some(s) = self.svcs.get_mut(name) else { return };
        let pid = s.pid.take().unwrap_or(0);
        s.code = code;
        let (typ, restart, restart_ms, remain) = (s.unit.typ, s.unit.restart, s.unit.restart_ms, s.unit.remain_after_exit);
        if s.stopping {
            s.stopping = false;
            s.act = Act::Inactive;
            s.sub = "dead";
            s.since = now;
            let d = self.desc(name);
            self.log(6, "mavex-init", 1, &format!("Stopped {d}."));
            return;
        }
        if typ == SvcType::Oneshot {
            if code == 0 {
                s.act = if remain { Act::Active } else { Act::Inactive };
                s.sub = if remain { "exited" } else { "dead" };
                s.since = now;
                if remain {
                    self.started += 1;
                }
                let d = self.desc(name);
                self.log(6, "mavex-init", 1, &format!("Finished {d}."));
            } else {
                s.act = Act::Failed;
                s.sub = "failed";
                s.result = "exit-code".into();
                self.log(3, "mavex-init", 1, &format!("{name}: Main process exited, code=exited, status={code}"));
                self.log(3, "mavex-init", 1, &format!("{name}: Failed with result 'exit-code'."));
            }
            return;
        }
        let want = restart == Restart::Always || (restart == Restart::OnFailure && code != 0);
        if want {
            if s.starts.len() >= 5 {
                s.act = Act::Failed;
                s.sub = "failed";
                s.result = "start-limit-hit".into();
                self.log(3, "mavex-init", 1, &format!("{name}: Start request repeated too quickly."));
                self.log(3, "mavex-init", 1, &format!("{name}: Failed with result 'start-limit-hit'."));
            } else {
                s.restarts += 1;
                s.act = Act::Activating;
                s.sub = "auto-restart";
                s.restart_at = Some(now + restart_ms);
                let n = s.restarts;
                self.log(6, "mavex-init", 1, &format!("{name}: Main process exited, code=exited, status={code} (pid {pid})"));
                self.log(6, "mavex-init", 1, &format!("{name}: Scheduled restart job, restart counter is at {n}."));
            }
        } else if code == 0 {
            s.act = Act::Inactive;
            s.sub = "dead";
            s.since = now;
            self.log(6, "mavex-init", 1, &format!("{name}: Deactivated successfully."));
        } else {
            s.act = Act::Failed;
            s.sub = "failed";
            s.result = "exit-code".into();
            self.log(3, "mavex-init", 1, &format!("{name}: Main process exited, code=exited, status={code}"));
            self.log(3, "mavex-init", 1, &format!("{name}: Failed with result 'exit-code'."));
        }
    }

    fn read_output(&mut self, name: &str) {
        let Some(s) = self.svcs.get_mut(name) else { return };
        let Some(rd) = s.rd.as_mut() else { return };
        let mut chunk = [0u8; 4096];
        let n = rd.read(&mut chunk).unwrap_or(0);
        let pid = s.pid.unwrap_or(0);
        let mut lines = Vec::new();
        if n == 0 {
            s.rd = None;
            if !s.buf.is_empty() {
                lines.push(String::from_utf8_lossy(&s.buf).into_owned());
                s.buf.clear();
            }
        } else {
            s.buf.extend_from_slice(&chunk[..n]);
            while let Some(i) = s.buf.iter().position(|b| *b == b'\n') {
                let l: Vec<u8> = s.buf.drain(..=i).collect();
                lines.push(String::from_utf8_lossy(&l[..l.len() - 1]).trim_end_matches('\r').to_string());
            }
        }
        for l in lines {
            self.j.log(6, name, pid, &l);
        }
    }

    fn reply(&self, id: &str, text: &str) {
        let _ = fs::write(format!("{RUN}/replies/{id}"), text);
    }

    fn process_requests(&mut self) {
        let Ok(rd) = fs::read_dir(format!("{RUN}/requests")) else { return };
        let mut files: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".req")).collect();
        files.sort();
        for f in files {
            let path = format!("{RUN}/requests/{f}");
            let text = fs::read_to_string(&path).unwrap_or_default();
            let _ = fs::remove_file(&path);
            let id = f.trim_end_matches(".req").to_string();
            let mut w = text.split_whitespace();
            let op = w.next().unwrap_or("").to_string();
            let names: Vec<String> = w.map(unit::normalize_name).collect();
            self.handle(&id, &op, names);
        }
    }

    fn handle(&mut self, id: &str, op: &str, names: Vec<String>) {
        if op == "daemon-reload" {
            self.ilog("Reloading unit files.");
            return self.reply(id, "ok");
        }
        for n in &names {
            match self.get(n) {
                None => return self.reply(id, &format!("error: Unit {n} not found.")),
                Some(s) if s.unit.masked && op != "stop" => return self.reply(id, &format!("error: Unit {n} is masked.")),
                _ => {}
            }
        }
        match op {
            "start" => {
                let txn = self.enqueue_start(&names);
                self.jobs.push(Job { id: id.into(), op: op.into(), names, txn, phase: Phase::Starting });
            }
            "stop" => {
                for n in &names {
                    self.stop_unit(n);
                }
                self.jobs.push(Job { id: id.into(), op: op.into(), names: names.clone(), txn: names, phase: Phase::Stopping });
            }
            "restart" | "try-restart" => {
                let names: Vec<String> = if op == "try-restart" {
                    names.into_iter().filter(|n| self.svcs.get(n).is_some_and(|s| s.act != Act::Inactive && s.act != Act::Failed)).collect()
                } else {
                    names
                };
                for n in &names {
                    self.stop_unit(n);
                }
                self.jobs.push(Job { id: id.into(), op: op.into(), names: names.clone(), txn: names, phase: Phase::Stopping });
            }
            _ => self.reply(id, &format!("error: Unsupported request '{op}'.")),
        }
    }

    fn advance_jobs(&mut self) {
        let mut jobs = std::mem::take(&mut self.jobs);
        for job in jobs.iter_mut() {
            match job.phase {
                Phase::Stopping => {
                    let all_stopped = job.names.iter().all(|n| self.svcs.get(n).is_none_or(|s| matches!(s.act, Act::Inactive | Act::Failed)));
                    if !all_stopped {
                        continue;
                    }
                    if job.op == "stop" {
                        self.reply(&job.id, "ok");
                        job.phase = Phase::Done;
                    } else {
                        job.txn = self.enqueue_start(&job.names);
                        job.phase = Phase::Starting;
                    }
                }
                Phase::Starting => {
                    let done = job.txn.iter().all(|n| !self.pending.contains(n) && self.svcs.get(n).is_none_or(settled));
                    if !done {
                        continue;
                    }
                    let failed: Vec<&String> = job.names.iter().filter(|n| self.svcs.get(*n).is_some_and(|s| s.act == Act::Failed || s.act == Act::Inactive && s.unit.typ != SvcType::Oneshot && !s.unit.is_target())).collect();
                    if job.id == "boot" {
                        let n = self.started;
                        self.ilog(&format!("Startup finished; {n} units active."));
                        let _ = fs::write(format!("{RUN}/booted"), format!("{n}\n"));
                    } else if failed.is_empty() {
                        self.reply(&job.id, "ok");
                    } else {
                        let f = failed[0].clone();
                        self.reply(&job.id, &format!("error: Job for {f} failed. See 'systemctl status {f}' and 'journalctl -u {f}' for details."));
                    }
                    job.phase = Phase::Done;
                }
                Phase::Done => {}
            }
        }
        jobs.retain(|j| !matches!(j.phase, Phase::Done));
        jobs.append(&mut self.jobs);
        self.jobs = jobs;
    }

    fn publish(&mut self) {
        for (name, s) in self.svcs.iter_mut() {
            let st = State { active: s.act.word().into(), sub: s.sub.into(), pid: s.pid.unwrap_or(0), since_ms: s.since, result: s.result.clone(), restarts: s.restarts, code: s.code };
            let text = st.serialize();
            if text != s.published {
                if fs::write(format!("{RUN}/units/{name}"), &text).is_ok() {
                    s.published = text;
                }
            }
        }
    }

    pub fn run(&mut self) -> ! {
        let me = util::now_ms();
        let _ = fs::write(format!("{RUN}/pid"), "1\n");
        self.ilog(&format!("mavex-init {} starting (systemd-style units, wasm32).", env!("CARGO_PKG_VERSION")));
        let target = default_target();
        self.ilog(&format!("Default target is {target}."));
        let txn = self.enqueue_start(&[target.clone()]);
        self.jobs.push(Job { id: "boot".into(), op: "start".into(), names: vec![target], txn, phase: Phase::Starting });
        let _ = me;
        loop {
            self.process_requests();
            self.schedule_starts();
            let now = util::now_ms();
            let due: Vec<String> = self.svcs.iter().filter(|(_, s)| s.restart_at.is_some_and(|t| t <= now)).map(|(n, _)| n.clone()).collect();
            for n in due {
                self.start_unit(&n);
            }
            let names: Vec<String> = self.svcs.keys().cloned().collect();
            for n in &names {
                let (pid, deadline_hit) = match self.svcs.get(n) {
                    Some(s) => (s.pid, s.act == Act::Deactivating && !s.killed9 && now > s.stop_deadline),
                    None => (None, false),
                };
                if deadline_hit {
                    if let Some(pid) = pid {
                        sys::kill_tree(pid, 9);
                    }
                    if let Some(s) = self.svcs.get_mut(n) {
                        s.killed9 = true;
                    }
                }
                if let Some(pid) = pid {
                    if let Some(code) = sys::poll_child(pid) {
                        self.read_output_if_ready(n);
                        self.on_exit(n, code);
                    }
                }
            }
            self.advance_jobs();
            self.publish();
            let mut fds = Vec::new();
            let mut owners = Vec::new();
            for (n, s) in &self.svcs {
                if let Some(rd) = &s.rd {
                    fds.push(rd.as_raw_fd());
                    owners.push(n.clone());
                }
            }
            let mut timeout = 100u64;
            for s in self.svcs.values() {
                if let Some(t) = s.restart_at {
                    timeout = timeout.min(t.saturating_sub(now).max(5));
                }
            }
            for i in sys::poll_read(&fds, timeout) {
                self.read_output(&owners[i]);
            }
        }
    }

    fn read_output_if_ready(&mut self, name: &str) {
        let fd = match self.svcs.get(name).and_then(|s| s.rd.as_ref()) {
            Some(rd) => rd.as_raw_fd(),
            None => return,
        };
        for _ in 0..8 {
            if sys::poll_read(&[fd], 0).is_empty() {
                break;
            }
            self.read_output(name);
            if self.svcs.get(name).is_none_or(|s| s.rd.is_none()) {
                break;
            }
        }
    }
}

