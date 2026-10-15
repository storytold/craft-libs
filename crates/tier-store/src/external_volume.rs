use serde::{Deserialize, Serialize};

/// Persisted cold-tier handle identity (File System Access directory picker).
///
/// Host runtimes map `handle_id` to a stored permission grant; this crate only carries metadata.
/// Path policy: [`TierCoordinator::cold_relative_path`](crate::TierCoordinator) in [`coordinator`](crate::coordinator).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalVolume {
    pub handle_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl ExternalVolume {
    pub fn new(handle_id: impl Into<String>) -> Self {
        Self { handle_id: handle_id.into(), label: None }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}
