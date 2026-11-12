use serde::{Deserialize, Serialize};

/// Cloud-side identity for tier-store streaming (aligned with craft-cloud file refs; no IO here).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudBlobRef {
    pub file_id: String,
    pub etag: Option<String>,
    pub content_type: Option<String>,
    pub byte_length: Option<u64>,
}
