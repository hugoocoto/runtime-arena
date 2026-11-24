//! What participants can learn about the judge machine: `GET /api/host`.

use std::sync::OnceLock;

use arena_protocol::{CacheInfo, HostInfo, JudgeSettings};
use axum::Json;
use axum::extract::State;

use crate::AppState;
use crate::auth::Caller;

/// Extensions worth knowing about when choosing compiler flags, as `/proc/cpuinfo` names them.
const NOTABLE_FLAGS: &[&str] = &[
    "popcnt",
    "sse4_2",
    "avx",
    "avx2",
    "fma",
    "bmi1",
    "bmi2",
    "adx",
    "aes",
    "pclmulqdq",
    "sha_ni",
    "avx512f",
    "avx512bw",
    "avx512dq",
    "avx512vl",
    "avx512_vnni",
];

/// Flags each x86-64 level adds to the previous one (psABI; `/proc/cpuinfo` names).
const X86_64_LEVELS: &[&[&str]] = &[
    &[
        "cx16", "lahf_lm", "popcnt", "pni", "sse4_1", "sse4_2", "ssse3",
    ],
    &[
        "avx", "avx2", "bmi1", "bmi2", "f16c", "fma", "abm", "movbe", "xsave",
    ],
    &["avx512f", "avx512bw", "avx512cd", "avx512dq", "avx512vl"],
];

/// The hardware part, read once: it does not change while the server runs.
struct Hardware {
    cpu_model: Option<String>,
    cpus: Option<u32>,
    x86_64_level: Option<u8>,
    cpu_flags: Vec<String>,
    caches: Vec<CacheInfo>,
    memory_kib: Option<u64>,
    kernel: Option<String>,
}

fn hardware() -> &'static Hardware {
    static HARDWARE: OnceLock<Hardware> = OnceLock::new();
    HARDWARE.get_or_init(|| {
        let read = |path: &str| std::fs::read_to_string(path).ok();
        let cpuinfo = read("/proc/cpuinfo").unwrap_or_default();
        let flags = first_value(&cpuinfo, "flags")
            .map(|f| f.split_whitespace().collect::<Vec<_>>())
            .unwrap_or_default();
        Hardware {
            cpu_model: first_value(&cpuinfo, "model name").map(str::to_owned),
            cpus: Some(count_key(&cpuinfo, "processor")).filter(|n| *n > 0),
            x86_64_level: (std::env::consts::ARCH == "x86_64" && !flags.is_empty())
                .then(|| x86_64_level(&flags)),
            cpu_flags: NOTABLE_FLAGS
                .iter()
                .filter(|f| flags.contains(f))
                .map(|f| (*f).to_owned())
                .collect(),
            caches: caches(judge_cpu()),
            memory_kib: read("/proc/meminfo").and_then(|m| mem_total_kib(&m)),
            kernel: read("/proc/sys/kernel/osrelease").map(|k| k.trim().to_owned()),
        }
    })
}

/// The CPU the first sandbox is pinned to (`JUDGE_CPUS`), or CPU 0 when unpinned.
fn judge_cpu() -> u32 {
    std::env::var("JUDGE_CPUS")
        .ok()
        .and_then(|cpus| cpus.split(',').next()?.trim().parse().ok())
        .unwrap_or(0)
}

/// `GET /api/host`, for anyone logged in.
pub async fn info(_: Caller, State(state): State<AppState>) -> Json<HostInfo> {
    let hw = hardware();
    let config = &state.config;
    Json(HostInfo {
        arch: std::env::consts::ARCH.to_owned(),
        cpu_model: hw.cpu_model.clone(),
        cpus: hw.cpus,
        x86_64_level: hw.x86_64_level,
        cpu_flags: hw.cpu_flags.clone(),
        caches: hw.caches.clone(),
        memory_kib: hw.memory_kib,
        kernel: hw.kernel.clone(),
        judge: JudgeSettings {
            workers: config.judge_boxes.len() as u32,
            runs_per_test: config.runs_per_test,
            processes: config.processes,
            max_failures: config.max_failures,
            microsecond_timing: config.cg_root.is_dir(),
        },
    })
}

/// The value of the first `key : value` line of a `/proc/cpuinfo`-style text.
fn first_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key)
            .then(|| v.trim())
            .filter(|v| !v.is_empty())
    })
}

fn count_key(text: &str, key: &str) -> u32 {
    text.lines()
        .filter(|line| line.split_once(':').is_some_and(|(k, _)| k.trim() == key))
        .count() as u32
}

/// The highest level whose flags, and those of every level below, are all present.
fn x86_64_level(flags: &[&str]) -> u8 {
    let mut level = 1;
    for added in X86_64_LEVELS {
        if !added.iter().all(|f| flags.contains(f)) {
            break;
        }
        level += 1;
    }
    level
}

fn mem_total_kib(meminfo: &str) -> Option<u64> {
    first_value(meminfo, "MemTotal")?
        .strip_suffix("kB")?
        .trim()
        .parse()
        .ok()
}

fn caches(cpu: u32) -> Vec<CacheInfo> {
    let dir = format!("/sys/devices/system/cpu/cpu{cpu}/cache");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut caches: Vec<CacheInfo> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("index"))
        .filter_map(|e| {
            let read = |name: &str| std::fs::read_to_string(e.path().join(name)).ok();
            Some(CacheInfo {
                level: read("level")?.trim().parse().ok()?,
                kind: read("type")?.trim().to_lowercase(),
                size_kib: parse_size_kib(&read("size")?)?,
            })
        })
        .collect();
    caches.sort_by(|a, b| (a.level, &a.kind).cmp(&(b.level, &b.kind)));
    caches
}

/// Sysfs cache sizes: "32K", "1024K", "32M".
fn parse_size_kib(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(k) = s.strip_suffix('K') {
        k.parse().ok()
    } else if let Some(m) = s.strip_suffix('M') {
        m.parse::<u64>().ok().map(|m| m * 1024)
    } else {
        s.parse::<u64>().ok().map(|b| b / 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPUINFO: &str = "processor\t: 0\nvendor_id\t: AuthenticAMD\n\
        model name\t: AMD Ryzen 7 5800X 8-Core Processor\n\
        flags\t\t: fpu sse2 avx avx2\n\nprocessor\t: 1\n\
        model name\t: AMD Ryzen 7 5800X 8-Core Processor\n";

    #[test]
    fn reads_cpuinfo() {
        assert_eq!(
            first_value(CPUINFO, "model name"),
            Some("AMD Ryzen 7 5800X 8-Core Processor")
        );
        assert_eq!(first_value(CPUINFO, "flags"), Some("fpu sse2 avx avx2"));
        assert_eq!(first_value(CPUINFO, "missing"), None);
        assert_eq!(count_key(CPUINFO, "processor"), 2);
    }

    #[test]
    fn levels_need_every_lower_level() {
        let v2 = X86_64_LEVELS[0];
        let v3: Vec<&str> = v2.iter().chain(X86_64_LEVELS[1]).copied().collect();
        let v4: Vec<&str> = v3.iter().chain(X86_64_LEVELS[2]).copied().collect();
        assert_eq!(x86_64_level(&["fpu", "sse2"]), 1);
        assert_eq!(x86_64_level(v2), 2);
        assert_eq!(x86_64_level(&v3), 3);
        assert_eq!(x86_64_level(&v4), 4);
        // AVX-512 without the v3 set still is v2.
        let gap: Vec<&str> = v2.iter().chain(X86_64_LEVELS[2]).copied().collect();
        assert_eq!(x86_64_level(&gap), 2);
    }

    #[test]
    fn reads_memory_and_cache_sizes() {
        assert_eq!(
            mem_total_kib("MemTotal:       32768000 kB\nMemFree: 1 kB\n"),
            Some(32_768_000)
        );
        assert_eq!(parse_size_kib("32K\n"), Some(32));
        assert_eq!(parse_size_kib("32M"), Some(32 * 1024));
        assert_eq!(parse_size_kib("bogus"), None);
    }
}
