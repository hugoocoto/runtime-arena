//! What the workers call to judge a submission. Tests swap in a fake.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use arena_judge::{Judge, JudgeConfig, Problem};
use arena_protocol::{JudgeReport, TestResult};

pub type ProgressFn<'a> = Box<dyn FnMut(usize, &TestResult) + Send + 'a>;
pub type JudgeFuture<'a> = Pin<Box<dyn Future<Output = Result<JudgeReport, String>> + Send + 'a>>;

pub trait JudgeBackend: Send + Sync + 'static {
    /// Judges `executable` in isolate box `box_id`. `Err` means the judge failed,
    /// not the submission.
    fn judge<'a>(
        &'a self,
        box_id: u32,
        problem: &'a Problem,
        executable: &'a Path,
        on_test: ProgressFn<'a>,
    ) -> JudgeFuture<'a>;
}

/// The real judge: isolate sandboxes.
pub struct IsolateBackend {
    pub config: JudgeConfig,
}

impl JudgeBackend for IsolateBackend {
    fn judge<'a>(
        &'a self,
        box_id: u32,
        problem: &'a Problem,
        executable: &'a Path,
        on_test: ProgressFn<'a>,
    ) -> JudgeFuture<'a> {
        Box::pin(async move {
            Judge::new(self.config.clone(), box_id)
                .judge_with_progress(problem, executable, on_test)
                .await
                .map_err(|e| e.to_string())
        })
    }
}
