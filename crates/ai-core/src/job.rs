//! Jobs: how work in flight is reported.
//!
//! The provider contract is submit, poll, cancel. Each poll returns a [`JobSnapshot`]: the
//! current [`JobState`] plus the [`Event`]s that arrived since the last poll (text deltas, tool
//! calls, finished outputs). This shape works for a two-second image edit, a two-minute video job
//! and a streaming chat reply, and a blocking driver and an async driver can both sit on it.

use serde::{Deserialize, Serialize};

use crate::connection::Billing;
use crate::error::AiError;
use crate::payload::Payload;

/// Where a job is in its life. Terminal states never change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    /// Accepted by the provider but not started.
    Queued,
    /// `percent` is 0 to 100 when the provider reports it.
    Running {
        /// Progress from 0 to 100, when the provider reports it.
        percent: Option<u8>,
    },
    /// Finished with outputs. Final.
    Succeeded,
    /// Ended with an error. Final.
    Failed {
        /// Why the job failed.
        error: AiError,
    },
    /// Stopped on request. Final.
    Cancelled,
}

impl JobState {
    /// True for succeeded, failed and cancelled, the states that never change.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed { .. } | Self::Cancelled)
    }

    /// Whether `next` is a legal observation after `self`. Observing the same state again is
    /// legal (nothing changed), and progress may not go backwards. Terminal states are final.
    pub fn can_become(&self, next: &Self) -> bool {
        use JobState::{Cancelled, Failed, Queued, Running, Succeeded};
        match (self, next) {
            (Queued, Queued | Running { .. } | Succeeded | Failed { .. } | Cancelled) => true,
            (Running { percent: a }, Running { percent: b }) => match (a, b) {
                (Some(a), Some(b)) => b >= a,
                (Some(_), None) => false,
                (None, _) => true,
            },
            (Running { .. }, Succeeded | Failed { .. } | Cancelled) => true,
            (a, b) if a.is_terminal() => a == b,
            _ => false,
        }
    }
}

/// A finished result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    /// The result data.
    pub payload: Payload,
    /// Optional note such as "variation 2". Structured output arrives as an `application/json`
    /// inline payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Something that happened since the last poll.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A piece of generated text, to append to what came before.
    TextDelta {
        /// The new text.
        text: String,
    },
    /// The model asked to call a tool.
    ToolCall {
        /// Name of the tool.
        name: String,
        /// The arguments, as a JSON string.
        arguments: String,
    },
    /// A finished output.
    Output(Output),
    /// A message from the provider for the user, not part of the result.
    Notice {
        /// The message text.
        message: String,
    },
}

/// What a provider charged or counted. All fields optional: many providers report none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Input the provider counted, in its own units (for example tokens).
    pub input_units: Option<u64>,
    /// Output the provider counted, in its own units.
    pub output_units: Option<u64>,
    /// Millionths of `currency`, so no floating point money.
    pub cost_micros: Option<u64>,
    /// Currency code for `cost_micros`, such as `USD`.
    pub currency: Option<String>,
}

/// A price estimate shown before a job runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostEstimate {
    /// Estimated price in millionths of `currency`.
    pub micros: u64,
    /// Currency code, such as `USD`.
    pub currency: String,
    /// How the job will be billed.
    pub billing: Billing,
}

/// One poll's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSnapshot {
    /// Where the job is now.
    pub state: JobState,
    /// Events since the previous poll, oldest first.
    #[serde(default)]
    pub events: Vec<Event>,
    /// What the provider reported charging, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl JobSnapshot {
    /// A snapshot with the given state and no events or usage.
    pub fn new(state: JobState) -> Self {
        Self { state, events: Vec::new(), usage: None }
    }

    /// The finished outputs in this snapshot's events.
    pub fn outputs(&self) -> impl Iterator<Item = &Output> {
        self.events.iter().filter_map(|e| match e {
            Event::Output(o) => Some(o),
            _ => None,
        })
    }
}
