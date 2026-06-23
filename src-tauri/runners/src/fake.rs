//! FakeRunner — a Runner implementation Runtime tests use to drive the worker
//! loop deterministically with no subprocess. It returns a pre-seeded
//! RunnerOutput (or error) and records the requests it received.

use crate::output::{InvocationRequest, LogSink, Runner, RunnerError, RunnerOutput};
use async_trait::async_trait;
use std::sync::Mutex;

pub struct FakeRunner {
    /// The next outputs to return, in order. When exhausted, the last is reused.
    responses: Vec<Result<RunnerOutput, RunnerError>>,
    /// Per-call scripted prose deltas to forward via invoke_stream before
    /// returning that call's output. A call with no entry (index >= len)
    /// forwards nothing. Indexed by the same cursor as `responses`.
    deltas: Vec<Vec<String>>,
    cursor: Mutex<usize>,
    pub received: Mutex<Vec<InvocationRequest>>,
}

impl FakeRunner {
    pub fn new(responses: Vec<Result<RunnerOutput, RunnerError>>) -> Self {
        Self { responses, deltas: vec![], cursor: Mutex::new(0), received: Mutex::new(Vec::new()) }
    }

    /// Seed responses AND, per call, the ordered prose deltas invoke_stream
    /// forwards before returning that call's output.
    pub fn with_deltas(
        responses: Vec<Result<RunnerOutput, RunnerError>>,
        deltas: Vec<Vec<String>>,
    ) -> Self {
        Self { responses, deltas, cursor: Mutex::new(0), received: Mutex::new(Vec::new()) }
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

    async fn invoke_stream(
        &self,
        req: &InvocationRequest,
        sink: &LogSink,
    ) -> Result<RunnerOutput, RunnerError> {
        self.received.lock().unwrap().push(req.clone());
        let idx = {
            let mut cur = self.cursor.lock().unwrap();
            let i = (*cur).min(self.responses.len() - 1);
            *cur += 1;
            i
        };
        // Forward this call's scripted deltas (if any) before returning.
        if let Some(call_deltas) = self.deltas.get(idx) {
            for d in call_deltas {
                sink(d);
            }
        }
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

    #[tokio::test]
    async fn scripted_deltas_are_forwarded_then_output_returned() {
        use crate::output::LogSink;
        use std::sync::Arc;
        let fake = FakeRunner::with_deltas(
            vec![Ok(output(Verdict::Approve))],
            vec![vec!["Analy".into(), "sing.".into()]],
        );
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let sink: LogSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let out = fake.invoke_stream(&req(), &sink).await.unwrap();
        assert_eq!(out.verdict, Verdict::Approve);
        assert_eq!(*seen.lock().unwrap(), vec!["Analy".to_string(), "sing.".to_string()]);
    }

    #[tokio::test]
    async fn deltas_default_to_empty_for_new_constructor() {
        use crate::output::LogSink;
        use std::sync::Arc;
        let fake = FakeRunner::always(output(Verdict::Approve));
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let sink: LogSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let _ = fake.invoke_stream(&req(), &sink).await.unwrap();
        assert!(seen.lock().unwrap().is_empty());
    }
}
