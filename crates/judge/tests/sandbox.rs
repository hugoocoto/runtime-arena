//! Judges real programs with isolate. Needs root and a cgroup set up for isolate,
//! so it only runs when ARENA_ISOLATE_TESTS=1 (see scripts/judge-tests.sh).

use std::path::{Path, PathBuf};
use std::process::Command;

use arena_judge::{Judge, JudgeConfig, Problem};
use arena_protocol::{JudgeReport, Verdict};

fn enabled() -> bool {
    if std::env::var_os("ARENA_ISOLATE_TESTS").is_some() {
        return true;
    }
    eprintln!("skipped: set ARENA_ISOLATE_TESTS=1 to run sandbox tests (scripts/judge-tests.sh)");
    false
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn problem(name: &str) -> Problem {
    Problem::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// Compiles tests/sandbox/NAME.c, or returns NAME as is for scripts.
fn program(name: &str, dir: &Path) -> PathBuf {
    let src = repo().join("tests/sandbox");
    if name.ends_with(".py") {
        return src.join(name);
    }
    let out = dir.join(name);
    let status = Command::new("cc")
        .args(["-O2", "-o"])
        .arg(&out)
        .arg(src.join(format!("{name}.c")))
        .status()
        .unwrap();
    assert!(status.success(), "compiling {name}");
    out
}

async fn judge(
    box_id: u32,
    problem_name: &str,
    program_name: &str,
    config: JudgeConfig,
) -> JudgeReport {
    let dir = tempfile::tempdir().unwrap();
    let exe = program(program_name, dir.path());
    let report = Judge::new(config, box_id)
        .judge(&problem(problem_name), &exe)
        .await
        .unwrap();
    eprintln!("{program_name} on {problem_name}: {report:#?}");
    report
}

fn once() -> JudgeConfig {
    JudgeConfig {
        runs_per_test: 1,
        ..JudgeConfig::default()
    }
}

fn verdicts(r: &JudgeReport) -> Vec<Verdict> {
    r.tests.iter().map(|t| t.verdict).collect()
}

#[tokio::test]
async fn accepted_with_repeated_runs() {
    if !enabled() {
        return;
    }
    let r = judge(10, "sum", "ok", JudgeConfig::default()).await;
    assert_eq!(r.verdict, Verdict::Accepted);
    assert_eq!(verdicts(&r), [Verdict::Accepted, Verdict::Accepted]);
    assert!(r.memory_kib > 0);
    assert!(r.tests.iter().all(|t| t.message.is_none()));
}

#[tokio::test]
async fn shebang_script_accepted() {
    if !enabled() {
        return;
    }
    let r = judge(11, "sum", "ok.py", once()).await;
    assert_eq!(verdicts(&r), [Verdict::Accepted, Verdict::Accepted]);
}

#[tokio::test]
async fn wrong_answer_names_the_failing_test() {
    if !enabled() {
        return;
    }
    let r = judge(12, "sum", "wa", once()).await;
    assert_eq!(r.verdict, Verdict::WrongAnswer);
    assert_eq!(verdicts(&r), [Verdict::Accepted, Verdict::WrongAnswer]);
    let failed = r.first_failure().unwrap();
    assert_eq!(failed.name, "02");
    assert_eq!(
        failed.message.as_deref(),
        // Test 02 is hidden, so its answer is not quoted.
        Some("token 1: found \"31\", which is wrong")
    );
}

#[tokio::test]
async fn custom_checker_accepts_within_tolerance() {
    if !enabled() {
        return;
    }
    let r = judge(13, "sum-tolerant", "wa", once()).await;
    assert_eq!(verdicts(&r), [Verdict::Accepted, Verdict::Accepted]);
}

#[tokio::test]
async fn time_limit() {
    if !enabled() {
        return;
    }
    let r = judge(14, "sum", "tle", once()).await;
    assert_eq!(verdicts(&r), [Verdict::TimeLimitExceeded; 2]);
    // Killed only after the extra time (limit + 0.5 s), so the time is past the limit.
    assert!(r.time_ms >= 1400, "{}", r.time_ms);
}

#[tokio::test]
async fn memory_limit() {
    if !enabled() {
        return;
    }
    let r = judge(15, "sum", "mle", once()).await;
    assert_eq!(verdicts(&r), [Verdict::MemoryLimitExceeded; 2]);
}

#[tokio::test]
async fn runtime_error() {
    if !enabled() {
        return;
    }
    let r = judge(16, "sum", "re", once()).await;
    assert_eq!(verdicts(&r), [Verdict::RuntimeError; 2]);
    assert_eq!(r.tests[0].message.as_deref(), Some("killed by signal 11"));
}

#[tokio::test]
async fn output_limit() {
    if !enabled() {
        return;
    }
    let config = JudgeConfig {
        output_limit_kib: 1024,
        ..once()
    };
    let r = judge(17, "sum", "ole", config).await;
    assert_eq!(verdicts(&r), [Verdict::OutputLimitExceeded; 2]);
}

#[tokio::test]
async fn output_replaced_by_a_symlink_is_not_trusted() {
    if !enabled() {
        return;
    }
    // The program links its output to the real answer file on the host; the
    // judge must not follow it. isolate passes no environment, so the path is
    // compiled in instead.
    let dir = tempfile::tempdir().unwrap();
    let answer = problem("sum").tests[0].answer.canonicalize().unwrap();
    let src = repo().join("tests/sandbox/symlink.c");
    let exe = dir.path().join("symlink");
    let status = Command::new("cc")
        .args(["-O2", "-o"])
        .arg(&exe)
        .arg(format!("-DANSWER=\"{}\"", answer.display()))
        .arg(&src)
        .status()
        .unwrap();
    assert!(status.success());
    let r = Judge::new(once(), 18)
        .judge(&problem("sum"), &exe)
        .await
        .unwrap();
    eprintln!("{r:#?}");
    assert_ne!(r.tests[0].verdict, Verdict::Accepted);
}
