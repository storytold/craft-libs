//! The provider trait: the one interface an adapter implements.
//!
//! This is the interim contract. It is deliberately small (list models, estimate, submit, poll,
//! cancel) so it can wrap whichever library the provider comparison picks, or direct adapters.
//! It is synchronous: a blocking driver calls it from a worker thread, and an async driver can
//! run it on a blocking pool, so the contract stays free of any runtime.
//!
//! Credentials are not passed here. A provider is built with its credentials already resolved
//! (the value held in a [`crate::SecretString`]), so the model layer never handles a secret.

use crate::error::AiError;
use crate::ids::{JobId, ModelId};
use crate::job::{CostEstimate, JobSnapshot};
use crate::model::ModelInfo;
use crate::request::Request;

/// An adapter to one connection.
pub trait Provider: Send + Sync {
    /// The models this connection offers. Doubles as the launcher's "test connection" call.
    fn models(&self) -> Result<Vec<ModelInfo>, AiError>;

    /// A price estimate for the request, when the provider can give one.
    fn estimate(&self, _model: &ModelId, _request: &Request) -> Result<Option<CostEstimate>, AiError> {
        Ok(None)
    }

    /// Validates the request against the model's spec and starts the job. Must return
    /// `Unsupported` for an input or feature the model lacks, before sending anything.
    fn submit(&self, model: &ModelId, request: &Request) -> Result<JobId, AiError>;

    /// The job's state and the events since the last poll. Terminal states are stable.
    fn poll(&self, job: &JobId) -> Result<JobSnapshot, AiError>;

    /// Asks the job to stop. Idempotent: cancelling a finished job is not an error.
    fn cancel(&self, job: &JobId) -> Result<(), AiError>;
}
