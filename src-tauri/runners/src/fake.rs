//! FakeRunner — a Runner implementation Runtime tests use to drive the worker
//! loop deterministically with no subprocess. It returns a pre-seeded
//! RunnerOutput (or error) and records the requests it received.

use crate::output::{InvocationRequest, Runner, RunnerError, RunnerOutput};
use async_trait::async_trait;
use std::sync::Mutex;

pub struct FakeRunner {
    /// The next outputs to return, in order. When exhausted, the last is reused.
    responses: Vec<Result<RunnerOutput, RunnerError>>,
    cursor: Mutex<usize>,
    pub received: Mutex<Vec<InvocationRequest>>,
}

impl FakeRunner {
    pub fn new(responses: Vec<Result<RunnerOutput, RunnerError>>) -> Self {
        Self { responses, cursor: Mutex::new(0), received: Mutex::new(Vec::new()) }
    }

    /// Convenience: always returns the given output.
    pub fn always(output: RunnerOutput) -> Self {
        Self::new(vec![Ok(output)])
    }
}

#[async_trait]
impl Runner for FakeRunner {
    async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
        self.received.lock().unwrap().push(req.clone());
        let mut cur = self.cursor.lock().unwrap();
        let idx = (*cur).min(self.responses.len() - 1);
        *cur += 1;
        match &self.responses[idx] {
            Ok(o) => Ok(o.clone()),
            Err(e) => Err(clone_err(e)),
        }
    }
}

fn clone_err(e: &RunnerError) -> RunnerError {
    match e {
        RunnerError::RateLimited(s) => RunnerError::RateLimited(s.clone()),
        RunnerError::Spawn(s) => RunnerError::Spawn(s.clone()),
        RunnerError::NoResult => RunnerError::NoResult,
        RunnerError::Other(s) => RunnerError::Other(s.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::Verdict;

    fn output(v: Verdict) -> RunnerOutput {
        RunnerOutput {
            verdict: v,
            artifact_path: Some("a.md".into()),
            final_text: "x".into(),
            usage: Default::default(),
        }
    }

    fn req() -> InvocationRequest {
        InvocationRequest {
            task_id: "T".into(),
            team_id: "t".into(),
            model: "m".into(),
            thinking_budget: 0,
            system_prompt: String::new(),
            user_message: String::new(),
            settings_path: String::new(),
            add_dirs: vec![],
        }
    }

    #[tokio::test]
    async fn returns_seeded_responses_in_order_then_repeats_last() {
        let fake = FakeRunner::new(vec![Ok(output(Verdict::Approve)), Ok(output(Verdict::Revise))]);
        assert_eq!(fake.invoke(&req()).await.unwrap().verdict, Verdict::Approve);
        assert_eq!(fake.invoke(&req()).await.unwrap().verdict, Verdict::Revise);
        // exhausted -> last repeats
        assert_eq!(fake.invoke(&req()).await.unwrap().verdict, Verdict::Revise);
        assert_eq!(fake.received.lock().unwrap().len(), 3);
    }
}
