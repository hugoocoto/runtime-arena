//! Thin wrapper around the `isolate` command line.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::Error;

/// Limits for one run inside the sandbox.
#[derive(Debug, Clone)]
pub struct Limits {
    pub time: Duration,
    pub memory_kib: u64,
    /// Largest file the program may write (its stdout included), in KiB.
    pub output_kib: u64,
}

/// What isolate reported about one run (its meta file).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunMeta {
    /// Two-letter isolate status: RE, SG, TO or XX. `None` means a normal exit with code 0.
    pub status: Option<String>,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub oom_killed: bool,
    pub time: Duration,
    pub wall_time: Duration,
    pub cg_mem_kib: u64,
    pub message: Option<String>,
}

impl RunMeta {
    pub fn parse(text: &str) -> Result<RunMeta, String> {
        let kv: HashMap<&str, &str> = text.lines().filter_map(|l| l.split_once(':')).collect();
        let num = |k: &str| -> Result<Option<i64>, String> {
            kv.get(k)
                .map(|v| v.parse::<i64>().map_err(|e| format!("meta {k}={v}: {e}")))
                .transpose()
        };
        let secs = |k: &str| -> Result<Duration, String> {
            kv.get(k).map_or(Ok(Duration::ZERO), |v| {
                v.parse::<f64>()
                    .map(Duration::from_secs_f64)
                    .map_err(|e| format!("meta {k}={v}: {e}"))
            })
        };
        Ok(RunMeta {
            status: kv.get("status").map(|s| s.to_string()),
            exit_code: num("exitcode")?.map(|v| v as i32),
            exit_signal: num("exitsig")?.map(|v| v as i32),
            oom_killed: kv.contains_key("cg-oom-killed"),
            time: secs("time")?,
            wall_time: secs("time-wall")?,
            cg_mem_kib: num("cg-mem")?.unwrap_or(0) as u64,
            message: kv.get("message").map(|s| s.to_string()),
        })
    }
}

/// One isolate box. Each box must be used by one judge at a time.
#[derive(Debug, Clone)]
pub struct Sandbox {
    isolate: PathBuf,
    box_id: u32,
}

impl Sandbox {
    pub fn new(isolate: impl Into<PathBuf>, box_id: u32) -> Self {
        Sandbox {
            isolate: isolate.into(),
            box_id,
        }
    }

    fn command(&self) -> Command {
        let mut c = Command::new(&self.isolate);
        c.arg("--cg")
            .arg(format!("--box-id={}", self.box_id))
            .stdin(Stdio::null())
            .kill_on_drop(true);
        c
    }

    /// Creates a fresh box and returns the directory the program sees as its working directory.
    pub async fn init(&self) -> Result<PathBuf, Error> {
        // A box left over from a crashed run would make --init fail.
        self.cleanup().await?;
        let out = self
            .command()
            .arg("--init")
            .output()
            .await
            .map_err(|e| Error::io(&self.isolate, e))?;
        if !out.status.success() {
            return Err(Error::Sandbox(format!(
                "isolate --init failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let root = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Ok(Path::new(&root).join("box"))
    }

    pub async fn cleanup(&self) -> Result<(), Error> {
        let out = self
            .command()
            .arg("--cleanup")
            .output()
            .await
            .map_err(|e| Error::io(&self.isolate, e))?;
        if !out.status.success() {
            return Err(Error::Sandbox(format!(
                "isolate --cleanup failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }

    /// Runs `./program` in the box with `stdin_file` as input, writing stdout to `stdout_file`.
    /// Both file names are relative to the box directory. `meta_file` is a path outside the box.
    pub async fn run(
        &self,
        program: &str,
        stdin_file: &str,
        stdout_file: &str,
        meta_file: &Path,
        limits: &Limits,
    ) -> Result<RunMeta, Error> {
        let time = limits.time.as_secs_f64();
        let out = self
            .command()
            .arg("--silent")
            .arg(format!("--meta={}", meta_file.display()))
            .arg(format!("--time={time:.3}"))
            // Keep running a bit past the limit so the reported time is real.
            .arg(format!("--extra-time={:.3}", (time * 0.5).max(0.2)))
            .arg(format!("--wall-time={:.3}", time * 3.0 + 1.0))
            .arg(format!("--cg-mem={}", limits.memory_kib))
            .arg(format!("--fsize={}", limits.output_kib))
            .arg("--processes=1")
            .arg("--env=PATH=/usr/local/bin:/usr/bin:/bin")
            .arg(format!("--stdin={stdin_file}"))
            .arg(format!("--stdout={stdout_file}"))
            .arg("--stderr=/dev/null")
            .arg("--run")
            .arg("--")
            .arg(format!("./{program}"))
            .output()
            .await
            .map_err(|e| Error::io(&self.isolate, e))?;

        // 0 and 1 mean the program finished (well or badly); anything else is isolate failing.
        match out.status.code() {
            Some(0 | 1) => {}
            _ => {
                return Err(Error::Sandbox(format!(
                    "isolate --run failed ({}): {}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                )));
            }
        }
        let text = tokio::fs::read_to_string(meta_file)
            .await
            .map_err(|e| Error::io(meta_file, e))?;
        RunMeta::parse(&text).map_err(Error::Sandbox)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_normal_exit() {
        let m =
            RunMeta::parse("time:0.219\ntime-wall:0.224\nmax-rss:1524\ncg-mem:1484\nexitcode:0\n")
                .unwrap();
        assert_eq!(m.status, None);
        assert_eq!(m.exit_code, Some(0));
        assert_eq!(m.time, Duration::from_millis(219));
        assert_eq!(m.cg_mem_kib, 1484);
        assert!(!m.oom_killed);
    }

    #[test]
    fn parses_oom_kill() {
        let m = RunMeta::parse(
            "time:0.098\ntime-wall:0.105\nmax-rss:261228\ncg-mem:262144\ncg-oom-killed:1\nexitsig:9\nstatus:SG\nmessage:Caught fatal signal 9\n",
        )
        .unwrap();
        assert!(m.oom_killed);
        assert_eq!(m.status.as_deref(), Some("SG"));
        assert_eq!(m.exit_signal, Some(9));
        assert_eq!(m.message.as_deref(), Some("Caught fatal signal 9"));
    }

    #[test]
    fn rejects_garbage_numbers() {
        assert!(RunMeta::parse("time:abc\n").is_err());
    }
}
