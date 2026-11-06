//! Runtime unit state published by init under /run/mavex-init/units for systemctl.
use std::fs;

pub const RUN: &str = "/run/mavex-init";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub active: String,
    pub sub: String,
    pub pid: u32,
    pub since_ms: u64,
    pub result: String,
    pub restarts: u32,
    pub code: i32,
}

impl State {
    pub fn serialize(&self) -> String {
        format!(
            "ActiveState={}\nSubState={}\nMainPID={}\nActiveEnterTimestamp={}\nResult={}\nNRestarts={}\nExecMainStatus={}\n",
            self.active, self.sub, self.pid, self.since_ms, self.result, self.restarts, self.code
        )
    }

    pub fn parse(text: &str) -> State {
        let mut s = State::default();
        for l in text.lines() {
            let Some((k, v)) = l.split_once('=') else { continue };
            match k {
                "ActiveState" => s.active = v.into(),
                "SubState" => s.sub = v.into(),
                "MainPID" => s.pid = v.parse().unwrap_or(0),
                "ActiveEnterTimestamp" => s.since_ms = v.parse().unwrap_or(0),
                "Result" => s.result = v.into(),
                "NRestarts" => s.restarts = v.parse().unwrap_or(0),
                "ExecMainStatus" => s.code = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        s
    }
}

pub fn read(name: &str) -> Option<State> {
    fs::read_to_string(format!("{RUN}/units/{name}")).ok().map(|t| State::parse(&t))
}

pub fn loaded_units() -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(format!("{RUN}/units")).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let s = State { active: "active".into(), sub: "running".into(), pid: 7, since_ms: 5, result: "success".into(), restarts: 2, code: 0 };
        assert_eq!(State::parse(&s.serialize()), s);
    }
}
