//! Queue RPC method names, payload schemas, and `craft-ipc` client helpers.
//!
//! Defines the wire contract for `queue.submit`, `queue.status`, `queue.list`, `queue.cancel`,
//! `queue.subscribe`, `queue.start`, `queue.pause`, `queue.clear_finished`, and `queue.log`
//! carried over `craft-ipc` (`storytold/craft-libs#505`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::DispatchError;
#[cfg(all(feature = "ipc", not(target_arch = "wasm32")))]
use crate::job::CraftJobSpec;
use crate::job::{JobPriority, JobStatus};

/// Method name for submitting a [`crate::CraftJobSpec`] to the encoderCraft queue.
pub const METHOD_QUEUE_SUBMIT: &str = "queue.submit";
/// Method name for querying the status of a specific job.
pub const METHOD_QUEUE_STATUS: &str = "queue.status";
/// Method name for listing all queued and active jobs.
pub const METHOD_QUEUE_LIST: &str = "queue.list";
/// Method name for cancelling a queued or active job.
pub const METHOD_QUEUE_CANCEL: &str = "queue.cancel";
/// Method name for subscribing to live queue progress and status events.
pub const METHOD_QUEUE_SUBSCRIBE: &str = "queue.subscribe";
/// Method name for starting queue processing.
pub const METHOD_QUEUE_START: &str = "queue.start";
/// Method name for pausing an active job or the entire queue.
pub const METHOD_QUEUE_PAUSE: &str = "queue.pause";
/// Method name for clearing finished jobs from the queue.
pub const METHOD_QUEUE_CLEAR_FINISHED: &str = "queue.clear_finished";
/// Method name for retrieving the transcode history log.
pub const METHOD_QUEUE_LOG: &str = "queue.log";

/// Parameters for `queue.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStatusParams {
    pub job_id: String,
}

/// Parameters for `queue.cancel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueCancelParams {
    pub job_id: String,
}

/// Parameters for `queue.pause`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuePauseParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

/// Parameters for `queue.subscribe`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSubscribeParams {
    /// Optional job ID filter; when `None`, subscribes to events for all jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

/// Result returned by `queue.submit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSubmitResult {
    pub job_id: String,
}

impl QueueSubmitResult {
    /// Parses a `queue.submit` reply value (accepts either `{"jobId": "..."}` or a bare string).
    pub fn from_value(value: &Value) -> Result<Self, DispatchError> {
        if let Some(id) = value.as_str() {
            if id.trim().is_empty() {
                return Err(DispatchError::InvalidJobSpec("empty jobId in queue.submit response".into()));
            }
            return Ok(Self { job_id: id.to_string() });
        }
        let parsed: Self = serde_json::from_value(value.clone()).map_err(|e| DispatchError::Json(format!("malformed queue.submit response: {e}")))?;
        if parsed.job_id.trim().is_empty() {
            return Err(DispatchError::InvalidJobSpec("empty jobId in queue.submit response".into()));
        }
        Ok(parsed)
    }
}

/// Summary of one job in a `queue.list` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueJobSummary {
    pub job_id: String,
    pub status: JobStatus,
    #[serde(default)]
    pub priority: JobPriority,
    /// Completion progress in `0.0..=1.0`.
    #[serde(default)]
    pub progress: f32,
    #[serde(default)]
    pub client_app: Option<String>,
}

/// Result returned by `queue.list`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueListResult {
    pub jobs: Vec<QueueJobSummary>,
}

impl QueueListResult {
    /// Parses a `queue.list` reply value (accepts either `{"jobs": [...]}` or a bare array `[...]`).
    pub fn from_value(value: &Value) -> Result<Self, DispatchError> {
        if value.is_array() {
            let jobs: Vec<QueueJobSummary> =
                serde_json::from_value(value.clone()).map_err(|e| DispatchError::Json(format!("malformed queue.list array: {e}")))?;
            return Ok(Self { jobs });
        }
        serde_json::from_value(value.clone()).map_err(|e| DispatchError::Json(format!("malformed queue.list response: {e}")))
    }
}

/// Event payload pushed to subscribers of `queue.subscribe`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEvent {
    pub job_id: String,
    pub status: JobStatus,
    #[serde(default)]
    pub progress: f32,
    #[serde(default)]
    pub target_id: Option<String>,
}

/// Typed extension helpers for dispatching jobs over an active [`craft_ipc::client::Client`].
#[cfg(all(feature = "ipc", not(target_arch = "wasm32")))]
pub mod ipc_ext {
    use craft_ipc::client::Client;
    use serde_json::{Value, json};

    use super::{
        CraftJobSpec, DispatchError, JobStatus, METHOD_QUEUE_CANCEL, METHOD_QUEUE_LIST, METHOD_QUEUE_STATUS, METHOD_QUEUE_SUBMIT,
        METHOD_QUEUE_SUBSCRIBE, QueueCancelParams, QueueListResult, QueueStatusParams, QueueSubmitResult, QueueSubscribeParams,
    };

    /// Submits a validated [`CraftJobSpec`] over an authenticated `craft-ipc` client session.
    pub fn submit_job(client: &mut Client, spec: &CraftJobSpec) -> Result<String, DispatchError> {
        spec.validate()?;
        let params = serde_json::to_value(spec).map_err(|e| DispatchError::Json(format!("failed to serialize CraftJobSpec: {e}")))?;
        let reply = client.call(METHOD_QUEUE_SUBMIT, params)?;
        Ok(QueueSubmitResult::from_value(&reply)?.job_id)
    }

    /// Queries the status of `job_id` over `craft-ipc`.
    pub fn get_job_status(client: &mut Client, job_id: &str) -> Result<JobStatus, DispatchError> {
        let params = serde_json::to_value(QueueStatusParams { job_id: job_id.to_string() })
            .map_err(|e| DispatchError::Json(format!("failed to serialize QueueStatusParams: {e}")))?;
        let reply = client.call(METHOD_QUEUE_STATUS, params)?;
        serde_json::from_value(reply).map_err(|e| DispatchError::Json(format!("invalid JobStatus response: {e}")))
    }

    /// Lists all queued and active jobs over `craft-ipc`.
    pub fn list_jobs(client: &mut Client) -> Result<QueueListResult, DispatchError> {
        let reply = client.call(METHOD_QUEUE_LIST, json!({}))?;
        QueueListResult::from_value(&reply)
    }

    /// Cancels `job_id` over `craft-ipc`.
    pub fn cancel_job(client: &mut Client, job_id: &str) -> Result<(), DispatchError> {
        let params = serde_json::to_value(QueueCancelParams { job_id: job_id.to_string() })
            .map_err(|e| DispatchError::Json(format!("failed to serialize QueueCancelParams: {e}")))?;
        let _ = client.call(METHOD_QUEUE_CANCEL, params)?;
        Ok(())
    }

    /// Subscribes to queue status/progress events over `craft-ipc`.
    pub fn subscribe_queue(client: &mut Client, job_id: Option<&str>) -> Result<Value, DispatchError> {
        let params = serde_json::to_value(QueueSubscribeParams { job_id: job_id.map(ToString::to_string) })
            .map_err(|e| DispatchError::Json(format!("failed to serialize QueueSubscribeParams: {e}")))?;
        Ok(client.call(METHOD_QUEUE_SUBSCRIBE, params)?)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::{JobStatus, QueueEvent, QueueListResult, QueueSubmitResult};

    #[test]
    fn test_queue_submit_result_parsing() {
        let obj = json!({ "jobId": "job-42" });
        assert_eq!(QueueSubmitResult::from_value(&obj).unwrap().job_id, "job-42");

        let bare = json!("job-43");
        assert_eq!(QueueSubmitResult::from_value(&bare).unwrap().job_id, "job-43");

        assert!(QueueSubmitResult::from_value(&json!("")).is_err());
        assert!(QueueSubmitResult::from_value(&json!({ "jobId": "   " })).is_err());
    }

    #[test]
    fn test_queue_list_result_parsing() {
        let wrapped = json!({
            "jobs": [{
                "jobId": "j1",
                "status": "Processing",
                "priority": "High",
                "progress": 0.5,
                "clientApp": "FilmCraft"
            }]
        });
        let res = QueueListResult::from_value(&wrapped).unwrap();
        assert_eq!(res.jobs.len(), 1);
        assert_eq!(res.jobs[0].job_id, "j1");
        assert_eq!(res.jobs[0].status, JobStatus::Processing);

        let bare = json!([{
            "jobId": "j2",
            "status": "Completed"
        }]);
        let res2 = QueueListResult::from_value(&bare).unwrap();
        assert_eq!(res2.jobs.len(), 1);
        assert_eq!(res2.jobs[0].job_id, "j2");
        assert_eq!(res2.jobs[0].status, JobStatus::Completed);
    }

    #[test]
    fn test_queue_event_roundtrip() {
        let ev = QueueEvent { job_id: "job-99".into(), status: JobStatus::Processing, progress: 0.75, target_id: Some("t1".into()) };
        let val = serde_json::to_value(&ev).unwrap();
        let back: QueueEvent = serde_json::from_value(val).unwrap();
        assert_eq!(ev, back);
    }
}
