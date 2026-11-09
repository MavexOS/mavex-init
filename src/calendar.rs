//! OnCalendar= subset: shortcuts (minutely, hourly, daily, weekly, monthly), `[Wkd[,Wkd]] [*-*-DD] HH:MM[:SS]`,
//! with `*`, `*/N`, `A/N`, numbers, lists (`0,30`) and ranges (`1..5`) per time field. UTC only.
use crate::util::civil;

#[derive(Clone, Debug, PartialEq)]
pub struct Cal {
    weekdays: Option<Vec<u32>>, // 0 = Mon .. 6 = Sun
    day: Option<Vec<u32>>,
    hour: Vec<u32>,
    min: Vec<u32>,
    sec: Vec<u32>,
}

fn field(s: &str, max: u32) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let (base, step) = match part.split_once('/') {
            Some((b, st)) => (b, Some(st.parse::<u32>().ok().filter(|v| *v > 0)?)),
            None => (part, None),
        };
        let (lo, hi) = if base == "*" {
            (0, max)
        } else if let Some((a, b)) = base.split_once("..") {
            (a.parse().ok()?, b.parse().ok()?)
        } else {
            let v: u32 = base.parse().ok()?;
            (v, if step.is_some() { max } else { v })
        };
        if hi > max || lo > hi {
            return None;
        }
        let mut v = lo;
        while v <= hi {
            out.push(v);
            v += step.unwrap_or(1);
        }
    }
    out.sort();
    out.dedup();
    Some(out)
}

pub fn parse(spec: &str) -> Option<Cal> {
    let spec = match spec.trim() {
        "minutely" => "*-*-* *:*:00",
        "hourly" => "*-*-* *:00:00",
        "daily" => "*-*-* 00:00:00",
        "weekly" => "Mon *-*-* 00:00:00",
        "monthly" => "*-*-01 00:00:00",
        s => s,
    };
    let mut weekdays = None;
    let mut day = None;
    let mut time = "00:00:00".to_string();
    let mut have_time = false;
    for w in spec.split_whitespace() {
        if w.contains(':') {
            time = w.to_string();
            have_time = true;
        } else if w.contains('-') && w.chars().any(|c| c.is_ascii_digit() || c == '*') {
            let p: Vec<&str> = w.split('-').collect();
            if p.len() != 3 || p[0] != "*" || p[1] != "*" {
                return None;
            }
            day = if p[2] == "*" { None } else { Some(field(p[2], 31)?) };
        } else {
            let names = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
            let mut v = Vec::new();
            for d in w.split(',') {
                let l = d.to_ascii_lowercase();
                v.push(names.iter().position(|n| l.starts_with(n))? as u32);
            }
            weekdays = Some(v);
        }
    }
    if !have_time && weekdays.is_none() && day.is_none() {
        return None;
    }
    let t: Vec<&str> = time.split(':').collect();
    if t.len() < 2 || t.len() > 3 {
        return None;
    }
    Some(Cal { weekdays, day, hour: field(t[0], 23)?, min: field(t[1], 59)?, sec: field(t.get(2).copied().unwrap_or("00"), 59)? })
}

/// First elapse strictly after `after_ms` (epoch milliseconds), searching up to ~400 days ahead.
pub fn next(c: &Cal, after_ms: u64) -> Option<u64> {
    let start = after_ms / 1000 + 1;
    let first_min = start / 60;
    for m in first_min..first_min + 400 * 24 * 60 {
        let days = (m / 1440) as i64;
        let (hour, min) = ((m % 1440 / 60) as u32, (m % 60) as u32);
        if !c.hour.contains(&hour) || !c.min.contains(&min) {
            continue;
        }
        let wd = (days + 3).rem_euclid(7) as u32;
        if c.weekdays.as_ref().is_some_and(|w| !w.contains(&wd)) {
            continue;
        }
        if let Some(d) = &c.day {
            let (_, _, dom) = civil(days);
            if !d.contains(&dom) {
                continue;
            }
        }
        for s in &c.sec {
            let t = m * 60 + *s as u64;
            if t >= start {
                return Some(t * 1000);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    const T0: u64 = 1_700_000_000_000; // Tue 2023-11-14 22:13:20 UTC

    #[test]
    fn shortcuts() {
        assert_eq!(next(&parse("minutely").unwrap(), T0), Some(1_700_000_040_000));
        assert_eq!(next(&parse("hourly").unwrap(), T0), Some(1_700_002_800_000));
        assert_eq!(next(&parse("daily").unwrap(), T0), Some(1_700_006_400_000));
    }
    #[test]
    fn steps_and_weekday() {
        assert_eq!(next(&parse("*:0/15").unwrap(), T0), Some(1_700_000_100_000));
        assert_eq!(next(&parse("Mon *-*-* 03:30").unwrap(), T0), Some(1_700_451_000_000));
        assert!(parse("2026-01-01 00:00").is_none());
    }
}
