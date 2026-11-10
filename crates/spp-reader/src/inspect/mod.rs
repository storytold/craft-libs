//! Human-readable reports about one project ([`ProjectReport`]), its document ([`DocumentDump`]) and its compiled graphs
//! ([`compiled_graph_dump`]).

mod cache_summary;
pub mod compiled_graph_dump;
pub mod document_dump;
mod report;
mod resource_summary;
mod tally;
mod text;
mod units;

pub use cache_summary::CacheSummary;
pub use document_dump::DocumentDump;
pub use report::{InspectOptions, Outcome, ProjectReport};
pub use resource_summary::ResourceSummary;
pub use tally::{Tally, Verification};
pub use text::TextReport;
