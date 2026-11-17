//! Which adapter wgpu's own rule would draw with, and two corrections to it for drivers that crash.
//!
//! On Windows the Vulkan loader takes one driver from each display adapter's driver package. A PC
//! whose GPUs were installed from different packages of one vendor (a laptop with an integrated
//! and a discrete AMD GPU, the first on the manufacturer's old package and the second on a current
//! one) loads two versions of that driver, and the newer GPU is listed once through each. wgpu
//! takes the first entry of the device type it prefers, which is whichever version the loader
//! listed first; a window drawn through the stale one ends the process inside the driver
//! (`amdvlk64.dll`, 0xc0000005), where no Rust error handling runs.
//!
//! [`choose`] is wgpu's own rule (`Instance::request_adapter`: the adapters that can draw to the
//! surface, by device type for the power preference, then as enumerated) with one addition: when
//! the adapter that rule picks is listed again by the same driver, the entry exposing the most
//! features is used, which is the newer version. Without such a duplicate the choice is wgpu's own.
//! Entries of one GPU from different drivers (RADV and AMDVLK, a vendor's driver and Dozen) are
//! left alone, and wgpu's order between them stays.
//!
//! An Intel Gen 9 to Gen 11 integrated GPU draws through its DirectX 12 entry rather than Vulkan
//! ([`intel_gen9_to_gen11`]). Intel's Vulkan driver for them (`igvk64.dll`) ends the process with
//! 0xc0000005 when the device is created, after the adapters are listed.

use std::cmp::Reverse;
use std::fmt;

use wgpu::{Adapter, Backend, DeviceType, PowerPreference, Surface};

/// One enumerated adapter as plain data, so that [`choose`] and [`crate::select`] are pure
/// functions that tests run without a GPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub backend: Backend,
    pub device_type: DeviceType,
    /// PCI vendor and device ids; both zero when the backend reports none (OpenGL).
    pub vendor: u32,
    pub device: u32,
    /// PCI bus id such as `0000:03:00.0`; empty when the driver reports none.
    pub pci_bus_id: String,
    /// The driver's name (Vulkan) or version (DirectX 12).
    pub driver: String,
    /// More about the driver, for the log.
    pub driver_info: String,
    /// How many wgpu features the driver exposes; a newer version of one driver exposes more.
    pub features: u32,
    /// Whether the surface can be drawn through this adapter (true without a surface).
    pub usable: bool,
}

impl Candidate {
    pub fn of(adapter: &Adapter, surface: Option<&Surface<'_>>) -> Self {
        let info = adapter.get_info();
        Candidate {
            name: info.name,
            backend: info.backend,
            device_type: info.device_type,
            vendor: info.vendor,
            device: info.device,
            pci_bus_id: info.device_pci_bus_id,
            driver: info.driver,
            driver_info: info.driver_info,
            features: adapter.features().bits().0.iter().map(|word| word.count_ones()).sum(),
            usable: surface.is_none_or(|s| adapter.is_surface_supported(s)),
        }
    }

    /// Whether the adapter draws on the CPU (WARP, llvmpipe, lavapipe, SwiftShader).
    pub fn is_software(&self) -> bool {
        self.device_type == DeviceType::Cpu
    }
}

impl fmt::Display for Candidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.device_type {
            DeviceType::DiscreteGpu => "discrete GPU",
            DeviceType::IntegratedGpu => "integrated GPU",
            DeviceType::VirtualGpu => "virtual GPU",
            DeviceType::Cpu => "software",
            DeviceType::Other => "unknown type",
        };
        let driver = format!("{} {}", self.driver, self.driver_info);
        write!(f, "\"{}\" ({:?}, {kind}, driver {}, {} features", self.name, self.backend, driver.trim(), self.features)?;
        if !self.pci_bus_id.is_empty() {
            write!(f, ", bus {}", self.pci_bus_id)?;
        }
        if !self.usable {
            write!(f, ", cannot draw to the window")?;
        }
        write!(f, ")")
    }
}

/// Whether `a` and `b` are one GPU listed twice by one driver, through two installed versions of
/// it: the same backend, device type, PCI vendor and device and driver name, and the same bus id
/// when both report one. Adapters without PCI ids (OpenGL reports zeros) are never the same: two
/// of them cannot be told apart.
pub(crate) fn same_gpu_and_driver(a: &Candidate, b: &Candidate) -> bool {
    let identified = a.vendor != 0 || a.device != 0;
    let same_bus = a.pci_bus_id.is_empty() || b.pci_bus_id.is_empty() || a.pci_bus_id == b.pci_bus_id;
    identified
        && a.backend == b.backend
        && a.device_type == b.device_type
        && a.vendor == b.vendor
        && a.device == b.device
        && a.driver == b.driver
        && same_bus
}

/// wgpu's device type order for `power` (wgpu-core's `Instance::request_adapter`): lower first.
/// Without a preference the adapters stay as enumerated.
pub(crate) fn type_order(device_type: DeviceType, power: PowerPreference) -> u8 {
    let prefer_integrated = match power {
        PowerPreference::LowPower => true,
        PowerPreference::HighPerformance => false,
        PowerPreference::None => return 0,
    };
    match device_type {
        DeviceType::DiscreteGpu if prefer_integrated => 2,
        DeviceType::IntegratedGpu if prefer_integrated => 1,
        DeviceType::DiscreteGpu => 1,
        DeviceType::IntegratedGpu => 2,
        DeviceType::Other => 3,
        DeviceType::VirtualGpu => 4,
        DeviceType::Cpu => 5,
    }
}

/// The index of the adapter wgpu's `request_adapter` picks: the first usable one of the best
/// device type for `power`. `None` without a usable adapter.
pub(crate) fn wgpu_choice(candidates: &[Candidate], power: PowerPreference) -> Option<usize> {
    let usable = candidates.iter().enumerate().filter(|(_, c)| c.usable);
    usable.min_by_key(|(index, c)| (type_order(c.device_type, power), *index)).map(|(index, _)| index)
}

/// A wgpu instance over the native backends (Vulkan, Metal, DirectX 12), and OpenGL except on
/// Windows. On Windows, OpenGL starts a WGL context thread inside the vendor driver (NVIDIA's
/// `nvoglv64` is both its OpenGL driver and its Vulkan driver), and instances created and devices
/// dropped at the same time can then deadlock in that driver.
pub fn native_instance() -> wgpu::Instance {
    wgpu::Instance::new(native_descriptor())
}

/// [`native_instance`]'s descriptor. wgpu's environment variables apply to its flags and backend
/// options: `WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1` lists a Vulkan driver that reports no
/// conformance version, such as Mesa's Dozen (the only one on WSL2), and `WGPU_VALIDATION`,
/// `WGPU_DX12_COMPILER` and the others work too. The backends stay as above whatever
/// `WGPU_BACKEND` says.
///
/// The descriptor has no display handle, which a headless instance does not need. OpenGL on
/// Wayland needs one to draw to a window: add it with `InstanceDescriptor::with_display_handle`,
/// or pass the descriptor to egui-wgpu as `WgpuSetupCreateNew::instance_descriptor`, where eframe
/// fills in the window's display handle.
pub fn native_descriptor() -> wgpu::InstanceDescriptor {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = if cfg!(windows) { wgpu::Backends::PRIMARY } else { wgpu::Backends::PRIMARY | wgpu::Backends::GL };
    desc.flags = desc.flags.with_env();
    desc.backend_options = desc.backend_options.with_env();
    desc
}

/// The index of the adapter to draw with for `power`: [`wgpu_choice`], or the entry exposing the
/// most features among those that are the same GPU listed by the same driver (the first of them
/// when several expose as many). For an Intel Gen 9 to Gen 11 GPU on Intel's Windows Vulkan
/// driver it is the same GPU's DirectX 12 entry, or without one the best other usable adapter.
/// `None` without a usable adapter, and when that Intel Vulkan entry is the only one.
pub fn choose(candidates: &[Candidate], power: PowerPreference) -> Option<usize> {
    let first = wgpu_choice(candidates, power)?;
    let preferred = candidates.get(first)?;
    let index = corrected(candidates, first, preferred);
    if candidates.get(index).is_some_and(intel_vulkan_crashes) {
        // No usable DirectX 12 entry for that GPU: the best other adapter wgpu would take.
        let safe = candidates.iter().enumerate().filter(|(_, c)| c.usable && !intel_vulkan_crashes(c));
        let (other, c) = safe.min_by_key(|(i, c)| (type_order(c.device_type, power), *i))?;
        return Some(corrected(candidates, other, c));
    }
    Some(index)
}

/// `index` (the entry `preferred`), or the entry to draw with instead of it.
pub(crate) fn corrected(candidates: &[Candidate], index: usize, preferred: &Candidate) -> usize {
    if let Some(dx12) = dx12_instead(candidates, preferred) {
        return dx12;
    }
    let versions = candidates.iter().enumerate().filter(|(_, c)| c.usable && same_gpu_and_driver(preferred, c));
    versions.max_by_key(|(index, c)| (c.features, Reverse(*index))).map_or(index, |(index, _)| index)
}

/// Whether the entry at `index` is one [`choose`] never draws with: the Vulkan entry of an Intel
/// Gen 9 to Gen 11 GPU on Intel's Windows driver, or an older version of a driver that lists one
/// GPU twice. Creating a device or drawing through either can end the process, so it is never
/// tried as a fallback either. A second GPU of the same model on the same driver, when the
/// driver reports no bus id for either, looks like an older version; it is still listed through
/// the other backends (DirectX 12 on Windows, OpenGL elsewhere).
pub(crate) fn avoided(candidates: &[Candidate], index: usize) -> bool {
    let Some(c) = candidates.get(index) else {
        return true;
    };
    intel_vulkan_crashes(c) || corrected(candidates, index, c) != index
}

/// PCI vendor id of Intel.
pub(crate) const INTEL: u32 = 0x8086;

/// Whether `device` is an Intel Gen 9 to Gen 11 integrated GPU: 6th to 10th generation Core, and
/// the Atom, Pentium and Celeron graphics of those years (Skylake, Broxton, Gemini Lake, Kaby
/// Lake, Coffee Lake, Whiskey Lake, Comet Lake, Cannon Lake, Ice Lake, Elkhart Lake, Jasper Lake),
/// by the PCI device ids of Linux's `include/drm/intel/pciids.h`: these prefixes hold those
/// families and no others, except `0x0a84` (Broxton), whose prefix Haswell shares.
///
/// Their Windows Vulkan driver ends the process with 0xc0000005 in `igvk64.dll` when a device is
/// created, on every version reported, from 25.20.100.6576 (2019) to 31.0.101.2145, the legacy
/// branch that is the last driver Intel ships for them. DirectX 12 works on the same GPUs.
pub fn intel_gen9_to_gen11(device: u32) -> bool {
    matches!(device >> 8, 0x19 | 0x1a | 0x31 | 0x3e | 0x45 | 0x4e | 0x59 | 0x5a | 0x87 | 0x8a | 0x9b) || device == 0x0a84
}

/// Whether `c` is an Intel Gen 9 to Gen 11 integrated GPU listed through Intel's Windows Vulkan
/// driver ([`intel_gen9_to_gen11`]). Mesa's Vulkan driver for the same GPUs on Linux reports a
/// driver name with "Mesa" in it and is not included.
pub(crate) fn intel_vulkan_crashes(c: &Candidate) -> bool {
    c.backend == Backend::Vulkan
        && c.device_type == DeviceType::IntegratedGpu
        && c.vendor == INTEL
        && intel_gen9_to_gen11(c.device)
        && !c.driver.to_ascii_lowercase().contains("mesa")
}

/// For an Intel Gen 9 to Gen 11 integrated GPU listed through Intel's Windows Vulkan driver, the
/// index of the same GPU's usable DirectX 12 entry, if there is one.
fn dx12_instead(candidates: &[Candidate], preferred: &Candidate) -> Option<usize> {
    if !intel_vulkan_crashes(preferred) {
        return None;
    }
    candidates.iter().position(|c| c.usable && c.backend == Backend::Dx12 && c.vendor == INTEL && c.device == preferred.device)
}

/// Write the adapters and the choice to the log, with a warning when a correction applied.
pub(crate) fn log_choice(candidates: &[Candidate], index: usize, power: PowerPreference) {
    for candidate in candidates {
        log::info!("graphics adapter available: {candidate}");
    }
    let Some(chosen) = candidates.get(index) else {
        return;
    };
    let others = candidates.iter().enumerate().filter(|(i, c)| *i != index && c.usable && same_gpu_and_driver(chosen, c)).count();
    if others > 0 {
        log::warn!(
            "{:?} lists \"{}\" {} times: several versions of its driver are installed, and a window drawn through a stale one can crash. Using the one with the most features; installing one driver package for every GPU removes the others.",
            chosen.backend,
            chosen.name,
            others.saturating_add(1)
        );
    }
    if let Some(wgpu_own) = wgpu_choice(candidates, power).and_then(|i| candidates.get(i))
        && wgpu_own.backend == Backend::Vulkan
        && chosen.backend == Backend::Dx12
        && wgpu_own.vendor == INTEL
        && wgpu_own.device == chosen.device
    {
        log::warn!("Intel's Vulkan driver for \"{}\" crashes when the device is created: drawing through DirectX 12", wgpu_own.name);
    }
    log::info!("graphics adapter chosen: {chosen}");
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use wgpu::Backend::{Dx12, Gl, Vulkan};
    use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu, Other};
    use wgpu::PowerPreference::{HighPerformance, LowPower};

    pub(crate) const AMD: u32 = 0x1002;
    pub(crate) const RX_6800M: u32 = 0x73df;
    pub(crate) const RADEON_IGPU: u32 = 0x1638;
    const POWERS: [PowerPreference; 3] = [PowerPreference::None, LowPower, HighPerformance];

    #[expect(clippy::too_many_arguments, reason = "one line per adapter in the listings below")]
    pub(crate) fn gpu(
        name: &str,
        backend: Backend,
        device_type: DeviceType,
        vendor: u32,
        device: u32,
        bus: &str,
        driver: &str,
        driver_info: &str,
        features: u32,
    ) -> Candidate {
        Candidate {
            name: name.into(),
            backend,
            device_type,
            vendor,
            device,
            pci_bus_id: bus.into(),
            driver: driver.into(),
            driver_info: driver_info.into(),
            features,
            usable: true,
        }
    }

    /// A laptop with an integrated Radeon on a 2021 driver package and an RX 6800M on a 2026 one,
    /// as wgpu 30 lists it: the Vulkan loader has both versions of AMD's driver, and the 2021 one
    /// lists the RX 6800M first. A window drawn through that entry ended inside `amdvlk64.dll`
    /// (0xc0000005) on every launch.
    pub(crate) fn two_amd_drivers() -> Vec<Candidate> {
        let amd = "AMD proprietary driver";
        vec![
            gpu("AMD Radeon RX 6800M", Vulkan, DiscreteGpu, AMD, RX_6800M, "", amd, "26.9.2", 60),
            gpu("AMD Radeon RX 6800M", Vulkan, DiscreteGpu, AMD, RX_6800M, "", amd, "26.9.2 (AMD proprietary shader compiler)", 68),
            gpu("AMD Radeon(TM) Graphics", Vulkan, IntegratedGpu, AMD, RADEON_IGPU, "", amd, "21.30.02.13", 56),
            gpu("AMD Radeon RX 6800M", Dx12, DiscreteGpu, AMD, RX_6800M, "0000:03:00.0", "32.0.21045.11001", "", 36),
            gpu("AMD Radeon(TM) Graphics", Dx12, IntegratedGpu, AMD, RADEON_IGPU, "0000:08:00.0", "30.0.13002.13003", "", 36),
            gpu("Microsoft Basic Render Driver", Dx12, Cpu, 0x1414, 0x8c, "", "10.0.26100.9549", "", 36),
        ]
    }

    #[test]
    fn a_gpu_listed_through_two_versions_of_its_driver_draws_with_the_newer_one() {
        let adapters = two_amd_drivers();
        for power in [HighPerformance, PowerPreference::None] {
            assert_eq!(wgpu_choice(&adapters, power), Some(0), "wgpu takes the stale entry, which crashed");
            assert_eq!(choose(&adapters, power), Some(1));
        }
        // whichever version the loader lists first
        let mut newer_first = adapters.clone();
        newer_first.swap(0, 1);
        assert_eq!(choose(&newer_first, HighPerformance), Some(0));
        // two entries exposing as much: wgpu's, the first
        let mut equal = adapters.clone();
        if let Some(c) = equal.get_mut(1) {
            c.features = 60;
        }
        assert_eq!(choose(&equal, HighPerformance), Some(0));
        // three versions: the one exposing the most
        let mut three = adapters.clone();
        three.insert(1, gpu("AMD Radeon RX 6800M", Vulkan, DiscreteGpu, AMD, RX_6800M, "", "AMD proprietary driver", "24.1.1", 64));
        assert_eq!(choose(&three, HighPerformance), Some(2));
        // the stale entries are avoided, the newest is not
        assert!(avoided(&three, 0) && avoided(&three, 1) && !avoided(&three, 2));
    }

    #[test]
    fn a_newer_version_never_overtakes_another_gpu_or_backend() {
        assert_eq!(choose(&two_amd_drivers(), LowPower), Some(2));
        let mut dx12_first = two_amd_drivers();
        dx12_first.rotate_left(3);
        for power in [HighPerformance, PowerPreference::None] {
            assert_eq!(choose(&dx12_first, power), Some(0));
        }
        assert_eq!(choose(&dx12_first, LowPower), wgpu_choice(&dx12_first, LowPower));
    }

    #[test]
    fn without_two_versions_of_one_driver_the_choice_is_wgpus() {
        let nvidia = |backend, bus, driver, features| gpu("NVIDIA GeForce RTX 4070", backend, DiscreteGpu, 0x10de, 0x2786, bus, driver, "", features);
        let intel =
            |backend, bus, driver, features| gpu("Intel(R) UHD Graphics 770", backend, IntegratedGpu, 0x8086, 0x4680, bus, driver, "", features);
        let radeon = |bus, driver, features| gpu("AMD Radeon RX 7900 XT", Vulkan, DiscreteGpu, AMD, 0x744c, bus, driver, "", features);
        let gl = |name: &str, features| gpu(name, Gl, Other, 0, 0, "", "", "", features);
        let without = |skip: usize| two_amd_drivers().into_iter().enumerate().filter(|(i, _)| *i != skip).map(|(_, c)| c).collect::<Vec<_>>();
        let listings: Vec<Vec<Candidate>> = vec![
            vec![],
            vec![nvidia(Vulkan, "0000:01:00.0", "NVIDIA", 70)],
            // a laptop with two GPUs, through Vulkan and DirectX 12
            vec![
                intel(Vulkan, "0000:00:02.0", "Intel Corporation", 60),
                nvidia(Vulkan, "0000:01:00.0", "NVIDIA", 70),
                nvidia(Dx12, "0000:01:00.0", "32.0.15.6094", 36),
                intel(Dx12, "0000:00:02.0", "31.0.101.5186", 36),
                gpu("Microsoft Basic Render Driver", Dx12, Cpu, 0x1414, 0x8c, "", "10.0.26100.1", "", 36),
            ],
            // one GPU through two different drivers: Linux with AMDVLK and RADV
            vec![radeon("0000:03:00.0", "AMD open-source driver", 58), radeon("0000:03:00.0", "radv", 71)],
            // a vendor's driver and Dozen (Vulkan on DirectX 12)
            vec![nvidia(Vulkan, "", "Dozen", 75), nvidia(Vulkan, "0000:01:00.0", "NVIDIA", 70)],
            // two cards of one model, on their own buses
            vec![radeon("0000:03:00.0", "AMD proprietary driver", 60), radeon("0000:04:00.0", "AMD proprietary driver", 68)],
            // OpenGL reports no ids: two adapters cannot be told apart
            vec![gl("ANGLE", 20), gl("ANGLE", 30)],
            vec![gpu("llvmpipe (LLVM 19.1.0, 256 bits)", Vulkan, Cpu, 0x10005, 0, "", "llvmpipe", "", 50)],
            without(0),
            without(1),
        ];
        for adapters in &listings {
            for power in POWERS {
                assert_eq!(choose(adapters, power), wgpu_choice(adapters, power), "{power:?} over {adapters:#?}");
            }
        }
        assert_eq!(choose(&[], HighPerformance), None);
    }

    /// An Intel UHD Graphics 630 (Coffee Lake) on driver 25.20.100.6576 as wgpu 30 lists it. A
    /// Vulkan device on it ended inside `igvk64.dll` (0xc0000005) as it was created.
    pub(crate) fn uhd_630() -> Vec<Candidate> {
        vec![
            gpu("Intel(R) UHD Graphics 630", Vulkan, IntegratedGpu, INTEL, 0x3e91, "", "Intel Corporation", "Intel driver", 44),
            gpu("Intel(R) UHD Graphics 630", Dx12, IntegratedGpu, INTEL, 0x3e91, "0000:00:02.0", "25.20.100.6576", "", 36),
            gpu("Microsoft Basic Render Driver", Dx12, Cpu, 0x1414, 0x8c, "", "10.0.26100.9711", "", 36),
        ]
    }

    fn uhd_630_as(name: &str, device: u32) -> Vec<Candidate> {
        uhd_630().into_iter().map(|c| if c.vendor == INTEL { Candidate { name: name.into(), device, ..c } } else { c }).collect()
    }

    #[test]
    fn an_intel_gen9_to_gen11_gpu_draws_through_directx_12() {
        for power in POWERS {
            assert_eq!(wgpu_choice(&uhd_630(), power), Some(0), "wgpu takes Vulkan, which crashed");
            assert_eq!(choose(&uhd_630(), power), Some(1), "{power:?}");
        }
        for (name, device) in [("Intel(R) HD Graphics 520", 0x1916), ("Intel(R) Iris(R) Plus Graphics", 0x8a5a)] {
            assert_eq!(choose(&uhd_630_as(name, device), HighPerformance), Some(1), "{name}");
        }
        let mut dx12_first = uhd_630();
        dx12_first.swap(0, 1);
        assert_eq!(choose(&dx12_first, HighPerformance), Some(0));
        // without a usable DirectX 12 entry, the Vulkan entry is still never drawn with
        let vulkan_only: Vec<Candidate> = uhd_630().into_iter().take(1).collect();
        assert_eq!(choose(&vulkan_only, HighPerformance), None);
        assert!(avoided(&vulkan_only, 0));
        let mut dx12_unusable = uhd_630();
        if let Some(c) = dx12_unusable.get_mut(1) {
            c.usable = false;
        }
        assert_eq!(choose(&dx12_unusable, HighPerformance), Some(2), "the software adapter");
        // Mesa's Vulkan driver for the same GPU on Linux is drawn with
        let mesa = vec![gpu(
            "Intel(R) UHD Graphics 630 (CFL GT2)",
            Vulkan,
            IntegratedGpu,
            INTEL,
            0x3e91,
            "0000:00:02.0",
            "Intel open-source Mesa driver",
            "Mesa 25.2.4",
            44,
        )];
        assert_eq!(choose(&mesa, HighPerformance), Some(0));
        assert!(!avoided(&mesa, 0));
        // beside a discrete GPU: high performance keeps it; low power takes the Intel GPU's DirectX 12 entry
        let mut laptop = uhd_630();
        laptop.insert(1, gpu("NVIDIA GeForce MX250", Vulkan, DiscreteGpu, 0x10de, 0x1d13, "0000:01:00.0", "NVIDIA", "", 70));
        assert_eq!(choose(&laptop, HighPerformance), Some(1));
        assert_eq!(choose(&laptop, LowPower), Some(2));
        assert!(avoided(&laptop, 0));
    }

    #[test]
    fn later_intel_gpus_keep_wgpus_choice() {
        for (name, device) in [("Intel(R) Iris(R) Xe Graphics", 0x9a49), ("Intel(R) UHD Graphics 770", 0x4680), ("Intel(R) Arc(TM) Graphics", 0x7d55)]
        {
            let adapters = uhd_630_as(name, device);
            for power in POWERS {
                assert_eq!(choose(&adapters, power), Some(0), "{name} {power:?}");
            }
        }
        let discrete: Vec<Candidate> =
            uhd_630().into_iter().map(|c| if c.vendor == INTEL { Candidate { device_type: DiscreteGpu, ..c } } else { c }).collect();
        assert_eq!(choose(&discrete, HighPerformance), Some(0));
    }

    #[test]
    fn gen9_to_gen11_ids() {
        for id in [0x1916, 0x0a84, 0x5a85, 0x3184, 0x5917, 0x87c0, 0x3e91, 0x3ea0, 0x9b41, 0x5a52, 0x8a5a, 0x4571, 0x4e61] {
            assert!(intel_gen9_to_gen11(id), "{id:#06x}");
        }
        for id in [0x0a16, 0x0a1e, 0x1616, 0x9a49, 0x4c8a, 0x4680, 0x46a6, 0xa780, 0x4905, 0x56a0, 0x7d55, 0x64a0, 0xe20b, 0xb080] {
            assert!(!intel_gen9_to_gen11(id), "{id:#06x}");
        }
    }

    #[test]
    fn two_entries_are_one_gpu_only_with_the_same_ids_type_driver_and_bus() {
        let stale = two_amd_drivers().swap_remove(0);
        let newer = two_amd_drivers().swap_remove(1);
        assert!(same_gpu_and_driver(&stale, &newer) && same_gpu_and_driver(&stale, &stale));
        let on_bus = |bus: &str| Candidate { pci_bus_id: bus.into(), ..newer.clone() };
        assert!(same_gpu_and_driver(&stale, &on_bus("0000:03:00.0")));
        assert!(same_gpu_and_driver(&on_bus("0000:03:00.0"), &on_bus("0000:03:00.0")));
        assert!(!same_gpu_and_driver(&on_bus("0000:03:00.0"), &on_bus("0000:04:00.0")));
        for other in [
            Candidate { backend: Dx12, ..newer.clone() },
            Candidate { device_type: IntegratedGpu, ..newer.clone() },
            Candidate { vendor: 0x10de, ..newer.clone() },
            Candidate { device: RADEON_IGPU, ..newer.clone() },
            Candidate { driver: "radv".into(), ..newer.clone() },
        ] {
            assert!(!same_gpu_and_driver(&stale, &other), "{other}");
        }
        let unidentified = Candidate { vendor: 0, device: 0, ..newer };
        assert!(!same_gpu_and_driver(&unidentified, &unidentified));
    }

    #[test]
    fn adapters_that_cannot_draw_to_the_window_are_left_out() {
        let unusable = |which: &dyn Fn(usize, &Candidate) -> bool| {
            let mut adapters = two_amd_drivers();
            for (i, c) in adapters.iter_mut().enumerate() {
                c.usable = !which(i, c);
            }
            adapters
        };
        assert_eq!(choose(&unusable(&|i, _| i == 1), HighPerformance), Some(0));
        assert_eq!(choose(&unusable(&|i, _| i == 0), HighPerformance), Some(1));
        assert_eq!(choose(&unusable(&|_, c| c.backend == Vulkan), HighPerformance), Some(3));
        assert_eq!(choose(&unusable(&|_, _| true), HighPerformance), None);
    }

    #[test]
    fn candidates_describe_themselves_for_the_log() {
        let adapters = two_amd_drivers();
        let text = adapters.get(1).map(ToString::to_string).unwrap_or_default();
        assert_eq!(
            text,
            "\"AMD Radeon RX 6800M\" (Vulkan, discrete GPU, driver AMD proprietary driver 26.9.2 (AMD proprietary shader compiler), 68 features)"
        );
        let offscreen = adapters.get(4).map(|c| Candidate { usable: false, ..c.clone() }.to_string()).unwrap_or_default();
        assert_eq!(
            offscreen,
            "\"AMD Radeon(TM) Graphics\" (Dx12, integrated GPU, driver 30.0.13002.13003, 36 features, bus 0000:08:00.0, cannot draw to the window)"
        );
        assert!(adapters.get(5).is_some_and(Candidate::is_software));
        assert!(!adapters.first().is_some_and(Candidate::is_software));
    }

    /// The instance reads wgpu's flag variables and keeps its own backends whatever `WGPU_BACKEND`
    /// says. Runs in a child process, which has them set.
    #[test]
    fn native_descriptor_reads_wgpu_flags_but_keeps_its_backends() {
        const CHILD: &str = "CRAFT_GPU_DEVICE_TEST_NATIVE_DESCRIPTOR_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "adapter::tests::native_descriptor_reads_wgpu_flags_but_keeps_its_backends", "--nocapture"])
                .env(CHILD, "1")
                .env("WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER", "1")
                .env("WGPU_BACKEND", "gl")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success() && stdout.contains("1 passed"), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
            return;
        }
        let desc = native_descriptor();
        assert!(desc.flags.contains(wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER), "{:?}", desc.flags);
        let backends = if cfg!(windows) { wgpu::Backends::PRIMARY } else { wgpu::Backends::PRIMARY | wgpu::Backends::GL };
        assert_eq!(desc.backends, backends);
    }

    /// Against the adapters of the machine running the tests: [`wgpu_choice`] is what wgpu's
    /// `request_adapter` returns, and [`choose`] returns that adapter too, a version of it exposing
    /// more features, or for an Intel Gen 9 to Gen 11 GPU its DirectX 12 entry. Passes without
    /// adapters.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn on_this_host_the_choice_is_wgpus_unless_a_correction_applies() {
        let backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let adapters = pollster::block_on(instance.enumerate_adapters(backends));
        let candidates: Vec<Candidate> = adapters.iter().map(|a| Candidate::of(a, None)).collect();
        for power in POWERS {
            let options = wgpu::RequestAdapterOptions { power_preference: power, ..Default::default() };
            let wgpu_own = pollster::block_on(instance.request_adapter(&options)).ok().map(|a| Candidate::of(&a, None));
            assert_eq!(wgpu_choice(&candidates, power).and_then(|i| candidates.get(i)), wgpu_own.as_ref(), "{power:?} over {candidates:#?}");
            let picked = choose(&candidates, power).and_then(|i| candidates.get(i));
            match (&wgpu_own, picked) {
                (Some(wgpu_own), Some(picked)) if wgpu_own != picked => {
                    let newer_version = same_gpu_and_driver(wgpu_own, picked) && picked.features > wgpu_own.features;
                    let intel_dx12 = wgpu_own.vendor == INTEL
                        && intel_gen9_to_gen11(wgpu_own.device)
                        && picked.backend == Backend::Dx12
                        && picked.device == wgpu_own.device;
                    assert!(newer_version || intel_dx12, "{power:?}: {picked} instead of {wgpu_own}");
                }
                _ => assert_eq!(picked, wgpu_own.as_ref(), "{power:?}"),
            }
        }
    }
}
