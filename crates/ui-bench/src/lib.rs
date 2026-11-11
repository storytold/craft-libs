//! Performance scenarios of the UI library (spec 22.7) and the harness that measures them.
//!
//! The scenario list, the measurement and the report are shared. A library supplies a
//! [`Library`]: the foundation binary runs [`run_all`] over it and writes `baseline/foundation.json`. It reads
//! `baseline/legacy.json`, the recorded baseline of the old library, as its speed gate.

mod alloc;
mod harness;
mod report;
mod scenario;

pub use alloc::{CountingAlloc, allocations};
pub use harness::{Library, MEASURED_FRAMES, Scene, SceneBuild, WARMUP_FRAMES, run_all, screen_rect};
pub use report::{Measurement, Report, ScenarioResult, gate_failures, print_table, read_json, write_json};
pub use scenario::{SCENARIOS, Scenario};
