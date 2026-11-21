//! Budget classes and fault handling.
//!
//! - [`BudgetConfig`] turns a budget class into an engine [`Budget`].
//! - [`Limits`] turns `manifest.limits` into the memory and stack a module is compiled and run with.
//! - [`FaultTracker`] counts faults per contribution and opens a circuit after repeated ones.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_plugin_abi::manifest::{Budget as ManifestBudget, Contribution, Kind, Manifest};

use crate::engine::{Budget, Trap};

const MIB: u64 = 1024 * 1024;

/// How long a call may run and how much it may burn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BudgetClass {
    /// UI thread work. Default 250 ms wall, hard cap 2 s.
    Interactive,
    /// Pixel or sample work. Fuel scales with the work, 2 s per band at most.
    Render,
    /// Long work off the UI thread, or a link call. Cancelable. Finite: `batch_cap` wall time and
    /// `batch_fuel_cap` fuel.
    Batch,
}

impl BudgetClass {
    /// The class a contribution runs under: its `budget` field, else the default of its kind.
    pub fn of(c: &Contribution) -> BudgetClass {
        match c.budget {
            Some(ManifestBudget::Interactive) => BudgetClass::Interactive,
            Some(ManifestBudget::Render) => BudgetClass::Render,
            // Defence in depth: `Manifest::validate` already refuses batch where `may_run_batch` is false.
            // A hook, panel or menu never runs unbounded on the UI thread, whatever the manifest says.
            Some(ManifestBudget::Batch) if matches!(c.kind, Kind::Hook | Kind::Panel | Kind::Menu | Kind::Unknown(_)) => BudgetClass::Interactive,
            Some(ManifestBudget::Batch) => BudgetClass::Batch,
            None if c.kind.is_data_plane() => BudgetClass::Render,
            // A file can be large and a decode is not a frame. The app's deadline (`FormatBook`) bounds it.
            None if c.kind == Kind::Format => BudgetClass::Batch,
            None => BudgetClass::Interactive,
        }
    }
}

/// Host wide numbers. The defaults are the ABI numbers.
#[derive(Clone, Debug)]
pub struct BudgetConfig {
    pub interactive_default: Duration,
    /// An interactive call never runs longer than this.
    pub interactive_cap: Duration,
    /// Hard wall time of a `batch` call. Finite on purpose.
    pub batch_cap: Duration,
    /// Hard fuel of a `batch` call. Finite on purpose.
    pub batch_fuel_cap: u64,
    /// Per band, per block.
    pub render_cap: Duration,
    /// Fuel per pixel or per sample for `render` calls.
    pub render_fuel_per_unit: u64,
    /// Memory when `limits.memory_mb` is absent.
    pub default_memory_mb: u32,
    /// Largest memory any plugin gets. The gate caps the declared maximum at the same number.
    pub host_memory_cap_mb: u32,
    /// Stack when `limits.stack_kb` is absent.
    pub default_stack_kb: u32,
    pub host_stack_cap_kb: u32,
    /// Consecutive faults that open a circuit.
    pub circuit_threshold: u32,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        BudgetConfig {
            interactive_default: Duration::from_millis(250),
            interactive_cap: Duration::from_secs(2),
            batch_cap: Duration::from_secs(300),
            batch_fuel_cap: 200_000_000_000,
            render_cap: Duration::from_secs(2),
            render_fuel_per_unit: 50_000,
            default_memory_mb: 256,
            host_memory_cap_mb: 1024,
            default_stack_kb: 1024,
            host_stack_cap_kb: 8192,
            circuit_threshold: 3,
        }
    }
}

/// Memory and stack of one plugin, after the manifest and the host caps agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub memory_bytes: u64,
    pub stack_bytes: u32,
}

impl BudgetConfig {
    /// `limits.memory_mb` and `limits.stack_kb` clamped to the host caps.
    pub fn limits_for(&self, manifest: &Manifest) -> Limits {
        let limits = manifest.limits.as_ref();
        let memory_mb = limits.and_then(|l| l.memory_mb).unwrap_or(self.default_memory_mb).min(self.host_memory_cap_mb);
        let stack_kb = limits.and_then(|l| l.stack_kb).unwrap_or(self.default_stack_kb).min(self.host_stack_cap_kb);
        Limits { memory_bytes: u64::from(memory_mb) * MIB, stack_bytes: stack_kb * 1024 }
    }

    /// The engine budget for one call. `units` is the pixel or sample count for `Render`, and is
    /// read as 1 when it is 0 so an empty band still gets a floor of fuel.
    pub fn budget(&self, class: BudgetClass, limits: Limits, units: u64, cancel: Arc<AtomicBool>) -> Budget {
        let mut b = Budget::new(limits.memory_bytes, limits.stack_bytes);
        b.cancel = cancel;
        match class {
            BudgetClass::Interactive => b.deadline = Some(self.interactive_default.min(self.interactive_cap)),
            BudgetClass::Render => {
                b.deadline = Some(self.render_cap);
                b.work = Some(units.max(1).saturating_mul(self.render_fuel_per_unit));
            }
            BudgetClass::Batch => {
                b.deadline = Some(self.batch_cap);
                b.work = Some(self.batch_fuel_cap);
            }
        }
        b
    }
}

/// What a fault tells the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultVerdict {
    pub consecutive: u32,
    /// True on the fault that opened the circuit. The caller tells the user once.
    pub tripped_now: bool,
}

#[derive(Default)]
struct Entry {
    consecutive: u32,
    open: bool,
}

/// Counts faults per (plugin, contribution). A success clears the count. An open circuit stays
/// open until `reset_plugin`, which a reload or the user calls.
pub struct FaultTracker {
    threshold: u32,
    entries: Mutex<HashMap<(String, String), Entry>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl FaultTracker {
    pub fn new(threshold: u32) -> FaultTracker {
        FaultTracker { threshold: threshold.max(1), entries: Mutex::new(HashMap::new()) }
    }

    pub fn is_open(&self, plugin: &str, contribution: &str) -> bool {
        lock(&self.entries).get(&(plugin.to_owned(), contribution.to_owned())).is_some_and(|e| e.open)
    }

    /// A completed call. It clears the count unless the circuit is already open.
    pub fn record_success(&self, plugin: &str, contribution: &str) {
        if let Some(e) = lock(&self.entries).get_mut(&(plugin.to_owned(), contribution.to_owned()))
            && !e.open
        {
            e.consecutive = 0;
        }
    }

    pub fn record_fault(&self, plugin: &str, contribution: &str) -> FaultVerdict {
        let mut map = lock(&self.entries);
        let e = map.entry((plugin.to_owned(), contribution.to_owned())).or_default();
        e.consecutive = e.consecutive.saturating_add(1);
        let tripped_now = !e.open && e.consecutive >= self.threshold;
        if tripped_now {
            e.open = true;
        }
        FaultVerdict { consecutive: e.consecutive, tripped_now }
    }

    /// Forget every count and close every circuit of one plugin.
    pub fn reset_plugin(&self, plugin: &str) {
        lock(&self.entries).retain(|(p, _), _| p != plugin);
    }
}

/// True when a trap is the guest's or the budget's doing, and so counts toward a circuit. A user
/// cancel does not.
pub fn is_fault(trap: &Trap) -> bool {
    !matches!(trap, Trap::Cancelled)
}
