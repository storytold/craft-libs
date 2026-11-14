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
    Queued,
    /// `percent` is 0 to 100 when the provider reports it.
    Running {
        percent: Option<u8>,
    },
    Succeeded,
    Failed {
        error: AiError,
    },
    Cancelled,
}

impl JobState {
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
    TextDelta { text: String },
    ToolCall { name: String, arguments: String },
    Output(Output),
    Notice { message: String },
}

/// What a provider charged or counted. All fields optional: many providers report none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_units: Option<u64>,
    pub output_units: Option<u64>,
    /// Millionths of `currency`, so no floating point money.
    pub cost_micros: Option<u64>,
    pub currency: Option<String>,
}

/// A price estimate shown before a job runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostEstimate {
    pub micros: u64,
    pub currency: String,
    pub billing: Billing,
}

/// One poll's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub state: JobState,
    #[serde(default)]
    pub events: Vec<Event>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl JobSnapshot {
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

#[cfg(test)]
mod tests {
    use super::*;
    use JobState::{Cancelled, Failed, Queued, Running, Succeeded};

    fn failed() -> JobState {
        Failed { error: AiError::new(crate::error::ErrorKind::Provider, "boom") }
    }

    #[test]
    fn forward_moves_are_legal() {
        assert!(Queued.can_become(&Running { percent: None }));
        assert!(Running { percent: Some(10) }.can_become(&Running { percent: Some(10) }));
        assert!(Running { percent: Some(10) }.can_become(&Running { percent: Some(80) }));
        assert!(Running { percent: Some(80) }.can_become(&Succeeded));
        assert!(Running { percent: None }.can_become(&failed()));
        assert!(Queued.can_become(&Succeeded));
    }

    #[test]
    fn progress_cannot_go_backwards() {
        assert!(!Running { percent: Some(50) }.can_become(&Running { percent: Some(40) }));
        assert!(!Running { percent: Some(50) }.can_become(&Running { percent: None }));
        assert!(!Running { percent: Some(5) }.can_become(&Queued));
    }

    #[test]
    fn terminal_states_are_final() {
        for t in [Succeeded, Cancelled, failed()] {
            assert!(t.is_terminal());
            assert!(t.can_become(&t.clone()));
            assert!(!t.can_become(&Running { percent: None }));
            assert!(!t.can_become(&Queued));
        }
        assert!(!Succeeded.can_become(&Cancelled));
        assert!(!Cancelled.can_become(&Succeeded));
    }

    #[test]
    fn snapshot_collects_outputs_only() {
        let mut s = JobSnapshot::new(Succeeded);
        s.events.push(Event::TextDelta { text: "hi".into() });
        s.events.push(Event::Output(Output { payload: Payload::text("done"), label: None }));
        assert_eq!(s.outputs().count(), 1);
    }

    #[test]
    fn snapshot_json_round_trip() {
        let mut s = JobSnapshot::new(Running { percent: Some(40) });
        s.events.push(Event::ToolCall { name: "lookup".into(), arguments: "{}".into() });
        s.usage = Some(Usage { cost_micros: Some(1200), currency: Some("USD".into()), ..Usage::default() });
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<JobSnapshot>(&json).unwrap(), s);
    }
}
