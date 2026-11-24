//! A scripted provider for tests, in every app's CI, with no network.
//!
//! It validates requests against its models' specs exactly as a real adapter must, then plays a
//! [`Script`]: it can stream text deltas, run for some number of polls, then succeed with outputs
//! or fail. Cancelling mid-job moves it to `Cancelled`.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::error::AiError;
use crate::ids::{JobId, ModelId};
use crate::job::{Event, JobSnapshot, JobState, Output};
use crate::model::ModelInfo;
use crate::payload::Limits;
use crate::provider::Provider;
use crate::request::Request;

/// How a mock job ends.
#[derive(Clone, Debug)]
pub enum Outcome {
    Succeed(Vec<Output>),
    Fail(AiError),
}

/// What a mock job does.
#[derive(Clone, Debug)]
pub struct Script {
    /// Polls that report `Running` before the job finishes.
    pub running_polls: u32,
    /// One text delta is emitted per poll until these run out.
    pub text_chunks: Vec<String>,
    pub outcome: Outcome,
}

impl Default for Script {
    fn default() -> Self {
        Self { running_polls: 2, text_chunks: Vec::new(), outcome: Outcome::Succeed(Vec::new()) }
    }
}

struct Job {
    script: Script,
    polls: u32,
    chunks_sent: usize,
    cancelled: bool,
    done: Option<JobState>,
}

#[derive(Default)]
struct State {
    next: u64,
    jobs: HashMap<JobId, Job>,
}

/// The scripted provider.
pub struct MockProvider {
    models: Vec<ModelInfo>,
    script: Script,
    limits: Limits,
    state: Mutex<State>,
}

impl MockProvider {
    pub fn new(models: Vec<ModelInfo>, script: Script) -> Self {
        Self { models, script, limits: Limits::default(), state: Mutex::new(State::default()) }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Poison is ignored on purpose (never-crash): a panic on another thread must not make every later
        // call fail. This is a test double whose state is plain recorded values.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Provider for MockProvider {
    fn models(&self) -> Result<Vec<ModelInfo>, AiError> {
        Ok(self.models.clone())
    }

    fn submit(&self, model: &ModelId, request: &Request) -> Result<JobId, AiError> {
        let info = self.models.iter().find(|m| &m.id == model).ok_or_else(|| AiError::invalid(format!("unknown model {model}")))?;
        let spec =
            info.spec_for(&request.capability).ok_or_else(|| AiError::unsupported(format!("model {model} cannot do {}", request.capability)))?;
        request.validate(spec, &self.limits)?;
        let mut state = self.lock();
        state.next = state.next.saturating_add(1);
        let id = JobId::new(format!("mock-{}", state.next)).map_err(|e| AiError::new(crate::error::ErrorKind::Internal, e.to_string()))?;
        state.jobs.insert(id.clone(), Job { script: self.script.clone(), polls: 0, chunks_sent: 0, cancelled: false, done: None });
        Ok(id)
    }

    fn poll(&self, job: &JobId) -> Result<JobSnapshot, AiError> {
        let mut state = self.lock();
        let j = state.jobs.get_mut(job).ok_or_else(|| AiError::invalid(format!("unknown job {job}")))?;
        if let Some(done) = &j.done {
            return Ok(JobSnapshot::new(done.clone()));
        }
        if j.cancelled {
            j.done = Some(JobState::Cancelled);
            return Ok(JobSnapshot::new(JobState::Cancelled));
        }
        j.polls = j.polls.saturating_add(1);
        let mut events = Vec::new();
        if let Some(chunk) = j.script.text_chunks.get(j.chunks_sent) {
            events.push(Event::TextDelta { text: chunk.clone() });
            j.chunks_sent += 1;
        }
        let streaming_left = j.chunks_sent < j.script.text_chunks.len();
        if j.polls <= j.script.running_polls || streaming_left {
            let percent = u8::try_from((j.polls.saturating_mul(100) / j.script.running_polls.saturating_add(1)).min(99)).unwrap_or(99);
            return Ok(JobSnapshot { state: JobState::Running { percent: Some(percent) }, events, usage: None });
        }
        let state_now = match &j.script.outcome {
            Outcome::Succeed(outputs) => {
                events.extend(outputs.iter().cloned().map(Event::Output));
                JobState::Succeeded
            }
            Outcome::Fail(e) => JobState::Failed { error: e.clone() },
        };
        j.done = Some(state_now.clone());
        Ok(JobSnapshot { state: state_now, events, usage: None })
    }

    fn cancel(&self, job: &JobId) -> Result<(), AiError> {
        let mut state = self.lock();
        let j = state.jobs.get_mut(job).ok_or_else(|| AiError::invalid(format!("unknown job {job}")))?;
        if j.done.is_none() {
            j.cancelled = true;
        }
        Ok(())
    }
}
