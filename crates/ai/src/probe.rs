//! Machine capability detection — runs once, cache the result.
//!
//! GPU/VRAM is deliberately *not* probed here: apps already link wgpu, so
//! they read their own adapter info and fill `MachineCaps::vram_gb` after
//! calling [`probe`]. Re-probing the GPU would double the surface for a
//! bug that reports nonsense VRAM on hybrid laptops.

use std::path::Path;

/// What this machine can comfortably run.
#[derive(Clone, Debug, PartialEq)]
pub struct MachineCaps {
    /// Total physical RAM, in GiB.
    pub ram_gb: f32,
    /// Free space on the volume holding the model cache, in GiB.
    pub disk_free_gb: f32,
    /// Logical CPU threads.
    pub threads: usize,
    /// AVX2 detected — without it quantized GGUF inference crawls.
    pub avx2: bool,
    /// Dedicated VRAM in GiB, when the app supplied it from its wgpu adapter.
    /// `None` means "unknown or no discrete GPU" — never guessed.
    pub vram_gb: Option<f32>,
}

impl MachineCaps {
    /// Rough throughput hint for a model of `size_mb`: tokens/sec on CPU.
    /// Deliberately conservative — an honest underestimate beats an
    /// optimistic one that makes the app look slow.
    pub fn est_tokens_per_sec(&self, size_mb: u64) -> f32 {
        let base = if self.avx2 { 6.0 } else { 1.2 };
        // ~0.5B params ≈ 350 MiB Q4; speed scales inversely with size.
        let scale = 350.0 / (size_mb.max(1) as f32);
        let thread_boost = (self.threads as f32 / 4.0).min(1.5);
        (base * scale * thread_boost).max(0.3)
    }
}

/// Probe the real machine once. `cache_dir` is the future model-cache
/// location — disk space is measured on *its* volume, which may not be
/// the OS volume.
pub fn probe(cache_dir: &Path) -> MachineCaps {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let ram_gb = sys.total_memory() as f32 / (1 << 30) as f32;

    let disk_free_gb = disk_free(cache_dir);

    MachineCaps { ram_gb, disk_free_gb, threads: std::thread::available_parallelism().map_or(1, |n| n.get()), avx2: avx2(), vram_gb: None }
}

#[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
fn avx2() -> bool {
    std::arch::is_x86_feature_detected!("avx2")
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "x86")))]
fn avx2() -> bool {
    // AArch64 NEON is mandatory, so the honest answer is "fast enough".
    true
}

/// Free bytes on the volume containing `dir`, walking up to the nearest
/// existing ancestor if `dir` doesn't exist yet (the common case).
fn disk_free(dir: &Path) -> f32 {
    let probe: &Path = dir.ancestors().find(|p| p.exists()).unwrap_or_else(|| Path::new("."));
    let disks = sysinfo::Disks::new_with_refreshed_list();
    // The longest mount point that prefixes our dir owns it.
    let best = disks.iter().filter(|d| probe.starts_with(d.mount_point())).max_by_key(|d| d.mount_point().as_os_str().len());
    best.map_or(0.0, |d| d.available_space() as f32 / (1 << 30) as f32)
}

/// The suite-shared model cache (`%LOCALAPPDATA%\artcraft\models` etc.).
/// `None` on platforms without a cache dir — the caller should refuse
/// local models rather than splashing files into the cwd.
pub fn models_dir() -> Option<std::path::PathBuf> {
    dirs::cache_dir().map(|d| d.join("artcraft").join("models"))
}
