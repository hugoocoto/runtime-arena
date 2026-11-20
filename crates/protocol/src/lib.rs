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

/// Error body of every failed API request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiError {
    pub error: String,
}

/// A participant, as shown to admins and to the participant themself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserInfo {
    pub name: String,
    pub display_name: String,
}

/// Body of `POST /api/admin/users`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateUser {
    /// Login name: lowercase letters, digits and `-`.
    pub name: String,
    /// Shown on leaderboards; defaults to `name`.
    pub display_name: Option<String>,
}

/// A participant's token. Returned only when it is created; the server keeps a hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserToken {
    pub name: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProblemSummary {
    pub slug: String,
    pub title: String,
    /// Increases every time the problem is uploaded again.
    pub version: i64,
    pub time_limit_ms: u64,
    pub memory_limit_mb: u64,
    pub test_count: u64,
}

/// What participants see of a problem: statement and sample tests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProblemStatement {
    pub slug: String,
    pub title: String,
    pub time_limit_ms: u64,
    pub memory_limit_mb: u64,
    /// Markdown from `statement.md`.
    pub statement: String,
    pub samples: Vec<Sample>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sample {
    pub name: String,
    pub input: String,
    pub output: String,
}

/// Body of `POST /api/admin/rounds`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateRound {
    pub name: String,
    pub duration_minutes: u32,
    /// Leaderboards stop updating this many minutes before the end (0 = never).
    #[serde(default)]
    pub freeze_minutes: u32,
    /// Minimum time between two submissions of one participant.
    #[serde(default = "default_cooldown")]
    pub cooldown_seconds: u32,
    /// Problem slugs, in display order.
    pub problems: Vec<String>,
}

fn default_cooldown() -> u32 {
    30
}

/// Body of `POST /api/admin/rounds/{id}/extend`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtendRound {
    pub minutes: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoundState {
    /// Created, not started yet.
    Draft,
    Running,
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoundInfo {
    pub id: i64,
    pub name: String,
    pub state: RoundState,
    pub duration_minutes: u32,
    pub freeze_minutes: u32,
    pub cooldown_seconds: u32,
    /// Unix time in seconds.
    pub started_at: Option<i64>,
    /// Unix time in seconds.
    pub ends_at: Option<i64>,
    /// Hidden from participants until the round starts.
    pub problems: Vec<ProblemSummary>,
    /// The server's clock (Unix seconds) when this was sent, so clients can show
    /// the time left correctly even if their own clock is off.
    pub server_time: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionStatus {
    Queued,
    Judging,
    /// Judged; see the verdict.
    Done,
    /// The judge could not run it (verdict `judge_error`); an admin may rejudge.
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubmissionInfo {
    pub id: i64,
    pub user: String,
    pub round_id: i64,
    pub problem: String,
    pub problem_version: i64,
    pub status: SubmissionStatus,
    pub verdict: Option<Verdict>,
    pub time_ms: Option<u64>,
    pub memory_kib: Option<u64>,
    /// Unix time in seconds.
    pub submitted_at: i64,
    pub judged_at: Option<i64>,
    /// Per-test results once judged.
    pub report: Option<JudgeReport>,
}

/// One participant's best accepted result on a problem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardEntry {
    pub rank: u32,
    pub user: String,
    pub display_name: String,
    pub time_ms: u64,
    pub memory_kib: u64,
    /// Submission that set this result.
    pub submission_id: i64,
    pub submitted_at: i64,
    /// All submissions of this participant on this problem (counted up to the freeze).
    pub attempts: u32,
}

/// The two leaderboards of one problem in one round.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Board {
    pub round_id: i64,
    pub problem: String,
    /// True when submissions after the freeze are hidden from this view.
    pub frozen: bool,
    /// Fastest first: CPU time, then memory, then earliest submission.
    pub by_time: Vec<BoardEntry>,
    /// Leanest first: memory, then CPU time, then earliest submission.
    pub by_memory: Vec<BoardEntry>,
}

/// An accepted submission as shown publicly (for rank-over-time charts).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedSubmission {
    pub submission_id: i64,
    pub user: String,
    pub display_name: String,
    pub time_ms: u64,
    pub memory_kib: u64,
    pub submitted_at: i64,
}

/// Messages the server sends over `GET /ws`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// First message: who the connection belongs to (`None` = anonymous viewer).
    Hello { user: Option<String> },
    /// The current round changed (started, stopped, extended).
    Round { round: RoundInfo },
    /// A problem's boards changed.
    Board { board: Board },
    /// One test of your submission was judged.
    Progress {
        submission_id: i64,
        test_index: u32,
        test_count: u32,
        result: TestResult,
    },
    /// Your submission finished judging.
    Submission { submission: SubmissionInfo },
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
