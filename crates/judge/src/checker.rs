//! Deciding whether a program's output answers a test.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::Error;

/// How long a custom checker may run before it counts as a judge error.
const CHECKER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    Accepted,
    WrongAnswer(String),
}

/// Compares whitespace-separated tokens, so spacing and trailing newlines don't matter.
pub fn compare_tokens(output: &[u8], answer: &[u8]) -> CheckResult {
    let mut out = output
        .split(|b| b.is_ascii_whitespace())
        .filter(|t| !t.is_empty());
    let mut ans = answer
        .split(|b| b.is_ascii_whitespace())
        .filter(|t| !t.is_empty());
    let mut index = 1;
    loop {
        match (out.next(), ans.next()) {
            (None, None) => return CheckResult::Accepted,
            (Some(o), Some(a)) if o == a => {}
            (Some(o), Some(a)) => {
                return CheckResult::WrongAnswer(format!(
                    "token {index}: expected {:?}, found {:?}",
                    clip(a),
                    clip(o)
                ));
            }
            (None, Some(_)) => {
                return CheckResult::WrongAnswer(format!("output ends before token {index}"));
            }
            (Some(_), None) => {
                return CheckResult::WrongAnswer(format!("extra output from token {index}"));
            }
        }
        index += 1;
    }
}

fn clip(token: &[u8]) -> String {
    let s = String::from_utf8_lossy(token);
    if s.chars().count() > 40 {
        format!("{}…", s.chars().take(40).collect::<String>())
    } else {
        s.into_owned()
    }
}

/// Runs a testlib-style checker: `checker INPUT OUTPUT ANSWER`.
///
/// Exit 0 accepts, 1 (wrong answer) and 2 (presentation error) reject; anything
/// else, a crash or a timeout is a judge error. The checker is the problem
/// author's code and runs outside the sandbox.
pub async fn run_checker(
    checker: &Path,
    input: &Path,
    output: &Path,
    answer: &Path,
) -> Result<CheckResult, Error> {
    let child = Command::new(checker)
        .arg(input)
        .arg(output)
        .arg(answer)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(CHECKER_TIMEOUT, child)
        .await
        .map_err(|_| Error::Checker(format!("timed out after {CHECKER_TIMEOUT:?}")))?
        .map_err(|e| Error::io(checker, e))?;

    // testlib writes its comment to stderr; some checkers use stdout.
    let comment = [out.stderr.as_slice(), out.stdout.as_slice()]
        .iter()
        .map(|b| String::from_utf8_lossy(b).trim().to_owned())
        .find(|s| !s.is_empty())
        .unwrap_or_default();
    match out.status.code() {
        Some(0) => Ok(CheckResult::Accepted),
        Some(1 | 2) => Ok(CheckResult::WrongAnswer(comment)),
        _ => Err(Error::Checker(format!(
            "exited with {}: {comment}",
            out.status
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_ignore_whitespace_layout() {
        assert_eq!(
            compare_tokens(b"1  2\n3", b"1 2 3\n\n"),
            CheckResult::Accepted
        );
        assert_eq!(compare_tokens(b"", b"\n"), CheckResult::Accepted);
    }

    #[test]
    fn tokens_report_first_difference() {
        assert_eq!(
            compare_tokens(b"1 5 3", b"1 2 3"),
            CheckResult::WrongAnswer("token 2: expected \"2\", found \"5\"".into())
        );
        assert_eq!(
            compare_tokens(b"1", b"1 2"),
            CheckResult::WrongAnswer("output ends before token 2".into())
        );
        assert_eq!(
            compare_tokens(b"1 2 3", b"1 2"),
            CheckResult::WrongAnswer("extra output from token 3".into())
        );
    }

    #[test]
    fn tokens_are_compared_exactly() {
        assert!(matches!(
            compare_tokens(b"7.0", b"7"),
            CheckResult::WrongAnswer(_)
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn custom_checker_exit_codes() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let file = |name: &str, content: &str| {
            let p = d.path().join(name);
            std::fs::write(&p, content).unwrap();
            p
        };
        let (input, output, answer) = (file("in", "x"), file("out", "x"), file("ans", "x"));
        let checker = |code: i32| {
            let p = file(
                &format!("check{code}"),
                &format!("#!/bin/sh\necho \"comment {code}\" >&2\nexit {code}\n"),
            );
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };

        let r = run_checker(&checker(0), &input, &output, &answer)
            .await
            .unwrap();
        assert_eq!(r, CheckResult::Accepted);
        let r = run_checker(&checker(1), &input, &output, &answer)
            .await
            .unwrap();
        assert_eq!(r, CheckResult::WrongAnswer("comment 1".into()));
        let e = run_checker(&checker(3), &input, &output, &answer)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("comment 3"), "{e}");
    }
}
