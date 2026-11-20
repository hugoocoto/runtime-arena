//! Types shared by the server and the CLI.

use serde::{Deserialize, Serialize};

/// Response of `GET /api/info`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerInfo {
    pub version: String,
    /// Where the source of the running server can be obtained (AGPL-3.0 section 13).
    pub source_url: String,
}

/// Outcome of running a submission on one test, or on all of them.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Accepted,
    WrongAnswer,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    OutputLimitExceeded,
    RuntimeError,
    /// The judge itself failed (sandbox or checker error); not the submission's fault.
    JudgeError,
}

impl Verdict {
    pub fn is_accepted(self) -> bool {
        self == Verdict::Accepted
    }
}

/// Result of one test. Time and memory are the best over the runs of that test.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestResult {
    /// Test name, e.g. "01" for `tests/01.in`.
    pub name: String,
    pub verdict: Verdict,
    /// CPU time in milliseconds.
    pub time_ms: u64,
    pub wall_time_ms: u64,
    /// Peak resident memory of the program, in KiB.
    pub memory_kib: u64,
    /// Detail for non-accepted verdicts (exit code, signal, checker comment).
    pub message: Option<String>,
}

/// Result of judging a submission against every test of a problem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JudgeReport {
    /// Accepted if every test is accepted, otherwise the verdict of the first failed test.
    pub verdict: Verdict,
    /// Largest CPU time over all tests, in milliseconds.
    pub time_ms: u64,
    /// Largest peak memory over all tests, in KiB.
    pub memory_kib: u64,
    pub tests: Vec<TestResult>,
}

impl JudgeReport {
    pub fn from_tests(tests: Vec<TestResult>) -> Self {
        let verdict = tests
            .iter()
            .map(|t| t.verdict)
            .find(|v| !v.is_accepted())
            .unwrap_or(Verdict::Accepted);
        JudgeReport {
            verdict,
            time_ms: tests.iter().map(|t| t.time_ms).max().unwrap_or(0),
            memory_kib: tests.iter().map(|t| t.memory_kib).max().unwrap_or(0),
            tests,
        }
    }

    /// The first test that was not accepted.
    pub fn first_failure(&self) -> Option<&TestResult> {
        self.tests.iter().find(|t| !t.verdict.is_accepted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test(name: &str, verdict: Verdict, time_ms: u64, memory_kib: u64) -> TestResult {
        TestResult {
            name: name.into(),
            verdict,
            time_ms,
            wall_time_ms: time_ms,
            memory_kib,
            message: None,
        }
    }

    #[test]
    fn report_of_all_accepted_takes_maxima() {
        let r = JudgeReport::from_tests(vec![
            test("01", Verdict::Accepted, 10, 900),
            test("02", Verdict::Accepted, 30, 500),
        ]);
        assert_eq!(r.verdict, Verdict::Accepted);
        assert_eq!((r.time_ms, r.memory_kib), (30, 900));
        assert!(r.first_failure().is_none());
    }

    #[test]
    fn report_verdict_is_first_failure() {
        let r = JudgeReport::from_tests(vec![
            test("01", Verdict::Accepted, 1, 1),
            test("02", Verdict::WrongAnswer, 1, 1),
            test("03", Verdict::TimeLimitExceeded, 1, 1),
        ]);
        assert_eq!(r.verdict, Verdict::WrongAnswer);
        assert_eq!(r.first_failure().unwrap().name, "02");
    }

    #[test]
    fn verdict_serializes_snake_case() {
        let json = serde_json::to_string(&Verdict::TimeLimitExceeded).unwrap();
        assert_eq!(json, "\"time_limit_exceeded\"");
    }
}
