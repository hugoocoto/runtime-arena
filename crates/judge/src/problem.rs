//! Problem directories: `problem.toml`, `statement.md` and `tests/NAME.{in,out}`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::Error;

#[derive(Debug, Clone)]
pub struct Problem {
    pub dir: PathBuf,
    pub title: String,
    pub time_limit: Duration,
    /// Memory limit in KiB.
    pub memory_limit_kib: u64,
    /// Custom checker executable (testlib convention); `None` compares tokens.
    pub checker: Option<PathBuf>,
    /// Names of the tests shown to participants.
    pub samples: Vec<String>,
    /// All tests, sorted by name.
    pub tests: Vec<TestCase>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCase {
    pub name: String,
    pub input: PathBuf,
    pub answer: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    title: String,
    time_limit_ms: u64,
    memory_limit_mb: u64,
    checker: Option<PathBuf>,
    #[serde(default)]
    samples: Vec<String>,
}

impl Problem {
    pub fn load(dir: impl AsRef<Path>) -> Result<Problem, Error> {
        let dir = dir.as_ref().to_path_buf();
        let manifest_path = dir.join("problem.toml");
        let text =
            std::fs::read_to_string(&manifest_path).map_err(|e| Error::io(&manifest_path, e))?;
        let m: Manifest = toml::from_str(&text).map_err(|e| Error::Problem {
            dir: dir.clone(),
            reason: format!("problem.toml: {e}"),
        })?;
        let bad = |reason: String| Error::Problem {
            dir: dir.clone(),
            reason,
        };

        if m.time_limit_ms == 0 || m.memory_limit_mb == 0 {
            return Err(bad(
                "time_limit_ms and memory_limit_mb must be positive".into()
            ));
        }
        let checker = match m.checker {
            Some(rel) => {
                let path = dir.join(rel);
                if !path.is_file() {
                    return Err(bad(format!("checker {} not found", path.display())));
                }
                Some(path)
            }
            None => None,
        };

        let tests = load_tests(&dir.join("tests")).map_err(bad)?;
        if tests.is_empty() {
            return Err(bad("no tests in tests/".into()));
        }
        for s in &m.samples {
            if !tests.iter().any(|t| &t.name == s) {
                return Err(bad(format!("sample {s:?} is not a test")));
            }
        }

        Ok(Problem {
            title: m.title,
            time_limit: Duration::from_millis(m.time_limit_ms),
            memory_limit_kib: m.memory_limit_mb * 1024,
            checker,
            samples: m.samples,
            tests,
            dir,
        })
    }
}

/// Pairs `NAME.in` with `NAME.out`; every input needs an answer and vice versa.
fn load_tests(tests_dir: &Path) -> Result<Vec<TestCase>, String> {
    let entries = std::fs::read_dir(tests_dir)
        .map_err(|e| format!("cannot read {}: {e}", tests_dir.display()))?;
    let mut inputs = Vec::new();
    let mut answers = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        let (Some(stem), Some(ext)) = (path.file_stem(), path.extension()) else {
            continue;
        };
        let stem = stem.to_string_lossy().into_owned();
        match ext.to_str() {
            Some("in") => inputs.push(stem),
            Some("out") => answers.push(stem),
            _ => {}
        }
    }
    inputs.sort();
    answers.sort();
    if let Some(n) = inputs.iter().find(|n| !answers.contains(n)) {
        return Err(format!("tests/{n}.in has no {n}.out"));
    }
    if let Some(n) = answers.iter().find(|n| !inputs.contains(n)) {
        return Err(format!("tests/{n}.out has no {n}.in"));
    }
    Ok(inputs
        .into_iter()
        .map(|name| TestCase {
            input: tests_dir.join(format!("{name}.in")),
            answer: tests_dir.join(format!("{name}.out")),
            name,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn problem_dir(manifest: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "problem.toml", manifest);
        write(d.path(), "tests/02.in", "1 2\n");
        write(d.path(), "tests/02.out", "3\n");
        write(d.path(), "tests/01.in", "3 4\n");
        write(d.path(), "tests/01.out", "7\n");
        d
    }

    const MANIFEST: &str = r#"
        title = "Sum"
        time_limit_ms = 1500
        memory_limit_mb = 256
        samples = ["01"]
    "#;

    #[test]
    fn loads_manifest_and_sorted_tests() {
        let d = problem_dir(MANIFEST);
        let p = Problem::load(d.path()).unwrap();
        assert_eq!(p.title, "Sum");
        assert_eq!(p.time_limit, Duration::from_millis(1500));
        assert_eq!(p.memory_limit_kib, 256 * 1024);
        assert_eq!(p.checker, None);
        let names: Vec<_> = p.tests.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["01", "02"]);
        assert_eq!(p.tests[0].answer, d.path().join("tests/01.out"));
    }

    #[test]
    fn rejects_input_without_answer() {
        let d = problem_dir(MANIFEST);
        write(d.path(), "tests/03.in", "");
        let e = Problem::load(d.path()).unwrap_err().to_string();
        assert!(e.contains("03.in has no 03.out"), "{e}");
    }

    #[test]
    fn rejects_unknown_sample_and_missing_checker() {
        let d = problem_dir(&MANIFEST.replace(r#"["01"]"#, r#"["09"]"#));
        assert!(
            Problem::load(d.path())
                .unwrap_err()
                .to_string()
                .contains("\"09\"")
        );

        let d = problem_dir(&format!("{MANIFEST}\nchecker = \"check\""));
        assert!(
            Problem::load(d.path())
                .unwrap_err()
                .to_string()
                .contains("checker")
        );
    }

    #[test]
    fn rejects_unknown_manifest_keys() {
        let d = problem_dir(&format!("{MANIFEST}\ntime_limit = 3"));
        assert!(Problem::load(d.path()).is_err());
    }
}
