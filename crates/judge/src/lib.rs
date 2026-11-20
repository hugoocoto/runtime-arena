//! Runs submitted executables against a problem's tests inside isolate sandboxes.
//!
//! Every test runs in a freshly initialised box (so memory usage from one run
//! does not leak into the next) and, when accepted, is repeated to take the best
//! time and memory. All tests run even after a failure, so the report says
//! exactly which ones fail.

mod checker;
mod problem;
mod sandbox;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use arena_protocol::{JudgeReport, TestResult, Verdict};

pub use checker::{CheckResult, compare_tokens};
pub use problem::{Problem, TestCase};
pub use sandbox::{Limits, RunMeta, Sandbox};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid problem {dir}: {reason}")]
    Problem { dir: PathBuf, reason: String },
    #[error("sandbox: {0}")]
    Sandbox(String),
    #[error("checker: {0}")]
    Checker(String),
}

impl Error {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Error::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

const SIGXFSZ: i32 = 25;

#[derive(Debug, Clone)]
pub struct JudgeConfig {
    /// Path of the isolate binary.
    pub isolate: PathBuf,
    /// Times an accepted test is run; the best time and memory are kept.
    pub runs_per_test: u32,
    /// Largest output a program may write, in KiB.
    pub output_limit_kib: u64,
}

impl Default for JudgeConfig {
    fn default() -> Self {
        JudgeConfig {
            isolate: "isolate".into(),
            runs_per_test: 3,
            output_limit_kib: 64 * 1024,
        }
    }
}

/// Judges submissions in one isolate box. Use one `Judge` per box at a time.
pub struct Judge {
    sandbox: Sandbox,
    config: JudgeConfig,
}

impl Judge {
    pub fn new(config: JudgeConfig, box_id: u32) -> Self {
        Judge {
            sandbox: Sandbox::new(&config.isolate, box_id),
            config,
        }
    }

    /// Runs `executable` on every test of `problem`.
    ///
    /// Errors mean the judge could not do its job (sandbox failure, unreadable
    /// files); problems with the submission are verdicts in the report.
    pub async fn judge(&self, problem: &Problem, executable: &Path) -> Result<JudgeReport, Error> {
        self.judge_with_progress(problem, executable, |_, _| {})
            .await
    }

    /// Like [`Judge::judge`], calling `on_test(index, result)` after each test.
    pub async fn judge_with_progress(
        &self,
        problem: &Problem,
        executable: &Path,
        mut on_test: impl FnMut(usize, &TestResult) + Send,
    ) -> Result<JudgeReport, Error> {
        let scratch = tempfile::tempdir().map_err(|e| Error::io(Path::new("tempdir"), e))?;
        let limits = Limits {
            time: problem.time_limit,
            memory_kib: problem.memory_limit_kib,
            output_kib: self.config.output_limit_kib,
        };
        let mut results = Vec::with_capacity(problem.tests.len());
        for test in &problem.tests {
            let result = self
                .judge_test(problem, test, executable, &limits, scratch.path())
                .await;
            // Leave no box behind, even when the test errored.
            self.sandbox.cleanup().await?;
            let result = result?;
            tracing::debug!(test = %result.name, verdict = ?result.verdict, "test judged");
            on_test(results.len(), &result);
            results.push(result);
        }
        Ok(JudgeReport::from_tests(results))
    }

    async fn judge_test(
        &self,
        problem: &Problem,
        test: &TestCase,
        executable: &Path,
        limits: &Limits,
        scratch: &Path,
    ) -> Result<TestResult, Error> {
        let mut result = TestResult {
            name: test.name.clone(),
            verdict: Verdict::Accepted,
            time_ms: u64::MAX,
            wall_time_ms: u64::MAX,
            memory_kib: u64::MAX,
            message: None,
        };
        for _ in 0..self.config.runs_per_test.max(1) {
            let box_dir = self.sandbox.init().await?;
            copy(executable, &box_dir.join("sol"), 0o755).await?;
            copy(&test.input, &box_dir.join("in"), 0o644).await?;
            let meta_file = scratch.join("meta");
            let meta = self
                .sandbox
                .run("sol", "in", "out", &meta_file, limits)
                .await?;

            result.time_ms = result.time_ms.min(meta.time.as_millis() as u64);
            result.wall_time_ms = result.wall_time_ms.min(meta.wall_time.as_millis() as u64);
            result.memory_kib = result.memory_kib.min(meta.max_rss_kib);

            let failure = match classify(&meta, limits.time) {
                Some(failure) => Some(failure),
                None => {
                    self.check_output(problem, test, &box_dir.join("out"), scratch)
                        .await?
                }
            };
            if let Some((verdict, message)) = failure {
                // Report the failing run's own numbers, not the best of earlier runs.
                result.time_ms = meta.time.as_millis() as u64;
                result.wall_time_ms = meta.wall_time.as_millis() as u64;
                result.memory_kib = meta.max_rss_kib;
                result.verdict = verdict;
                result.message = Some(message);
                break;
            }
        }
        Ok(result)
    }

    async fn check_output(
        &self,
        problem: &Problem,
        test: &TestCase,
        output: &Path,
        scratch: &Path,
    ) -> Result<Option<(Verdict, String)>, Error> {
        // The box is writable by the program: never follow a link it may have left
        // in place of its output (it could point at the answer file).
        let out = match tokio::fs::symlink_metadata(output).await {
            Ok(m) if m.file_type().is_file() => tokio::fs::read(output)
                .await
                .map_err(|e| Error::io(output, e))?,
            Ok(_) => {
                let msg = "the output is not a regular file".to_owned();
                return Ok(Some((Verdict::WrongAnswer, msg)));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(Error::io(output, e)),
        };
        let checked = match &problem.checker {
            Some(checker) => {
                // Hand the checker a private copy, not a path inside the box.
                let copy = scratch.join("output");
                tokio::fs::write(&copy, &out)
                    .await
                    .map_err(|e| Error::io(&copy, e))?;
                match checker::run_checker(checker, &test.input, &copy, &test.answer).await {
                    Ok(r) => r,
                    // A broken checker is the judge's fault, not the submission's.
                    Err(e) => return Ok(Some((Verdict::JudgeError, e.to_string()))),
                }
            }
            None => {
                let ans = tokio::fs::read(&test.answer)
                    .await
                    .map_err(|e| Error::io(&test.answer, e))?;
                compare_tokens(&out, &ans, problem.samples.contains(&test.name))
            }
        };
        Ok(match checked {
            CheckResult::Accepted => None,
            CheckResult::WrongAnswer(msg) => Some((Verdict::WrongAnswer, msg)),
        })
    }
}

/// Verdict for a run that did not finish cleanly, or `None` if its output should be checked.
fn classify(meta: &RunMeta, time_limit: Duration) -> Option<(Verdict, String)> {
    let status = meta.status.as_deref();
    if status == Some("XX") {
        let msg = meta.message.clone().unwrap_or_default();
        return Some((Verdict::JudgeError, format!("sandbox error: {msg}")));
    }
    if meta.oom_killed {
        return Some((
            Verdict::MemoryLimitExceeded,
            "killed at the memory limit".into(),
        ));
    }
    if status == Some("TO") || meta.time > time_limit {
        return Some((
            Verdict::TimeLimitExceeded,
            format!("ran {} ms", meta.time.as_millis()),
        ));
    }
    match (status, meta.exit_signal, meta.exit_code) {
        (_, Some(SIGXFSZ), _) => Some((Verdict::OutputLimitExceeded, "output too large".into())),
        (Some("SG"), Some(sig), _) => {
            Some((Verdict::RuntimeError, format!("killed by signal {sig}")))
        }
        (Some("RE"), _, Some(code)) => Some((Verdict::RuntimeError, format!("exit code {code}"))),
        (Some(other), _, _) => Some((
            Verdict::RuntimeError,
            meta.message
                .clone()
                .unwrap_or_else(|| format!("isolate status {other}")),
        )),
        (None, _, _) => None,
    }
}

async fn copy(from: &Path, to: &Path, mode: u32) -> Result<(), Error> {
    tokio::fs::copy(from, to)
        .await
        .map_err(|e| Error::io(from, e))?;
    tokio::fs::set_permissions(to, std::fs::Permissions::from_mode(mode))
        .await
        .map_err(|e| Error::io(to, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: Duration = Duration::from_secs(1);

    fn meta(text: &str) -> RunMeta {
        RunMeta::parse(text).unwrap()
    }

    #[test]
    fn clean_exit_goes_to_the_checker() {
        assert_eq!(classify(&meta("time:0.1\nexitcode:0\n"), LIMIT), None);
    }

    #[test]
    fn classifies_failures() {
        let v = |text| classify(&meta(text), LIMIT).map(|(v, _)| v);
        assert_eq!(v("status:TO\ntime:1.5\n"), Some(Verdict::TimeLimitExceeded));
        // Finished during the extra time: still too slow.
        assert_eq!(
            v("time:1.2\nexitcode:0\n"),
            Some(Verdict::TimeLimitExceeded)
        );
        assert_eq!(
            v("status:SG\nexitsig:9\ncg-oom-killed:1\ntime:0.1\n"),
            Some(Verdict::MemoryLimitExceeded)
        );
        assert_eq!(
            v("status:SG\nexitsig:11\ntime:0.1\n"),
            Some(Verdict::RuntimeError)
        );
        assert_eq!(
            v("status:SG\nexitsig:25\ntime:0.1\n"),
            Some(Verdict::OutputLimitExceeded)
        );
        assert_eq!(
            v("status:RE\nexitcode:3\ntime:0.1\n"),
            Some(Verdict::RuntimeError)
        );
        assert_eq!(v("status:XX\nmessage:boom\n"), Some(Verdict::JudgeError));
    }

    #[test]
    fn runtime_error_message_names_the_cause() {
        let (_, msg) = classify(&meta("status:RE\nexitcode:3\ntime:0.1\n"), LIMIT).unwrap();
        assert_eq!(msg, "exit code 3");
        let (_, msg) = classify(&meta("status:SG\nexitsig:11\ntime:0.1\n"), LIMIT).unwrap();
        assert_eq!(msg, "killed by signal 11");
    }
}
