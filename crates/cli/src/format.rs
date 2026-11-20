//! Human-readable numbers.

use arena_protocol::Verdict;

pub fn memory(kib: u64) -> String {
    if kib >= 1024 * 10 {
        format!("{:.0} MiB", kib as f64 / 1024.0)
    } else if kib >= 1024 {
        format!("{:.1} MiB", kib as f64 / 1024.0)
    } else {
        format!("{kib} KiB")
    }
}

pub fn time(ms: u64) -> String {
    if ms >= 1000 {
        format!("{:.2} s", ms as f64 / 1000.0)
    } else {
        format!("{ms} ms")
    }
}

/// "1h 05m", "4m 09s", "12s".
pub fn duration(secs: i64) -> String {
    let s = secs.max(0);
    if s >= 3600 {
        format!("{}h {:02}m", s / 3600, s % 3600 / 60)
    } else if s >= 60 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

pub fn verdict(v: Verdict) -> &'static str {
    match v {
        Verdict::Accepted => "accepted",
        Verdict::WrongAnswer => "wrong answer",
        Verdict::TimeLimitExceeded => "time limit exceeded",
        Verdict::MemoryLimitExceeded => "memory limit exceeded",
        Verdict::OutputLimitExceeded => "output limit exceeded",
        Verdict::RuntimeError => "runtime error",
        Verdict::JudgeError => "judge error",
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(memory(900), "900 KiB");
        assert_eq!(memory(1536), "1.5 MiB");
        assert_eq!(memory(262_144), "256 MiB");
        assert_eq!(time(12), "12 ms");
        assert_eq!(time(1593), "1.59 s");
        assert_eq!(duration(3900), "1h 05m");
        assert_eq!(duration(249), "4m 09s");
        assert_eq!(duration(-5), "0s");
    }
}
