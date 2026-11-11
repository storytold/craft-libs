//! The package loader. It runs the policy gate before any engine sees the bytes.

use std::sync::Arc;

use craft_plugin_abi::{HostPolicy, PolicyReport, check_package};

use super::{CompileOptions, Engine, LoadError, Module, require_deadline_enforcement};

/// Gate a package and compile it on `engine`.
///
/// Order: the deadline rule, then the gate, then compile. No guest code runs before the gate
/// accepts. `policy.simd128` is cleared when the engine does not report `SIMD128`, so a manifest
/// cannot ask for a feature the engine lacks. A legacy v1 package skips the gate by design and
/// loads through its shim, so it is refused here with `LoadError::LegacyNeedsShim`.
pub async fn load_package(
    engine: &dyn Engine,
    bytes: &[u8],
    policy: &HostPolicy,
    opts: CompileOptions,
) -> Result<(Arc<dyn Module>, PolicyReport), LoadError> {
    let caps = engine.caps();
    require_deadline_enforcement(engine.id(), caps, opts.trust)?;
    let policy = HostPolicy { simd128: policy.simd128 && caps.contains(super::Caps::SIMD128), ..*policy };
    let report = check_package(bytes, &policy);
    if let Some(abi) = report.legacy {
        return Err(LoadError::LegacyNeedsShim(abi));
    }
    if !report.is_accepted() {
        return Err(LoadError::Gate(report.violations));
    }
    let module = engine.compile(bytes, &report, opts).await?;
    Ok((module, report))
}
