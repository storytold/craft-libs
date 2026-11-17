//! The adapter the user asked for, and what was drawn with instead when it is not there.

use std::fmt;

use wgpu::{Backend, PowerPreference};

use crate::adapter::{Candidate, avoided, choose, corrected, intel_vulkan_crashes, type_order};

/// Which adapter to draw with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase", tag = "kind"))]
pub enum Preference {
    /// The fastest adapter: a discrete GPU before an integrated one, hardware before software.
    #[default]
    Auto,
    /// An integrated GPU before a discrete one, for battery life.
    LowPower,
    /// One adapter the user picked from [`crate::list`].
    Adapter(AdapterKey),
    /// A software adapter that draws on the CPU.
    Software,
}

impl Preference {
    /// The power preference that orders the adapters for this preference: low power for
    /// [`Preference::LowPower`], high performance otherwise.
    pub fn power(&self) -> PowerPreference {
        match self {
            Preference::LowPower => PowerPreference::LowPower,
            _ => PowerPreference::HighPerformance,
        }
    }
}

/// What identifies an adapter across runs: the backend, the PCI ids and bus, the driver name, and
/// the name. The order in which the system lists adapters can change between runs, so an index
/// cannot be kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase", default))]
pub struct AdapterKey {
    pub name: String,
    /// wgpu's lowercase backend name (`vulkan`, `dx12`, `metal`, `gl`).
    pub backend: String,
    pub vendor: u32,
    pub device: u32,
    /// Empty when the driver reports none.
    pub pci_bus_id: String,
    /// The Vulkan driver's name, which tells apart two Vulkan drivers for one GPU (such as two on
    /// Linux) and stays the same across versions of a driver. Empty on the other backends, and
    /// when the driver reports none.
    pub driver: String,
}

impl AdapterKey {
    pub fn of(candidate: &Candidate) -> Self {
        AdapterKey {
            name: candidate.name.clone(),
            backend: backend_name(candidate.backend).to_string(),
            vendor: candidate.vendor,
            device: candidate.device,
            pci_bus_id: candidate.pci_bus_id.clone(),
            // DirectX 12 reports the driver's version here, which changes with every update.
            driver: if candidate.backend == Backend::Vulkan { candidate.driver.clone() } else { String::new() },
        }
    }

    /// Whether `candidate` is this adapter: the same backend and PCI ids, the same bus and the
    /// same driver when both report one, and the same name when the backend reports no PCI ids.
    pub fn matches(&self, candidate: &Candidate) -> bool {
        let identified = self.vendor != 0 || self.device != 0;
        let same = |a: &str, b: &str| a.is_empty() || b.is_empty() || a == b;
        self.backend == backend_name(candidate.backend)
            && self.vendor == candidate.vendor
            && self.device == candidate.device
            && same(&self.pci_bus_id, &candidate.pci_bus_id)
            && same(&self.driver, &candidate.driver)
            && (identified || self.name == candidate.name)
    }
}

impl fmt::Display for AdapterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\" ({})", self.name, self.backend)
    }
}

/// wgpu's lowercase name of `backend`.
pub fn backend_name(backend: Backend) -> &'static str {
    backend.to_str()
}

/// Something the app tells the user about the adapter it draws with. The [`fmt::Display`] text is
/// English, for the log; an app shows its own translated text for each variant.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Notice {
    /// The adapter the user picked is not listed (unplugged, disabled, or its driver removed).
    /// The automatic choice draws instead.
    AdapterMissing(AdapterKey),
    /// Software was asked for and no software adapter is listed. The automatic choice draws
    /// instead.
    NoSoftwareAdapter,
    /// The only adapter that can draw is a software one: drawing is slower and uses the CPU.
    SoftwareOnly,
    /// An adapter could not open a device, and the next one was tried.
    DeviceFailed { adapter: String, error: String },
    /// The device was lost this many times in a short time, so drawing moved to software.
    SoftwareAfterLosses(u32),
}

impl fmt::Display for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Notice::AdapterMissing(key) => write!(f, "the graphics adapter {key} is not available; using the automatic choice"),
            Notice::NoSoftwareAdapter => f.write_str("no software graphics adapter is available; using the automatic choice"),
            Notice::SoftwareOnly => f.write_str("no graphics hardware can be used; drawing in software, which is slower"),
            Notice::DeviceFailed { adapter, error } => {
                write!(f, "the graphics adapter \"{adapter}\" could not be opened ({error}); trying the next one")
            }
            Notice::SoftwareAfterLosses(n) => write!(f, "the graphics device stopped working {n} times; drawing in software"),
        }
    }
}

/// The adapter chosen for a preference, and what the user should be told about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Index into the candidates.
    pub index: usize,
    pub notices: Vec<Notice>,
}

/// The adapter to draw with for `preference`, or `None` when no adapter is usable.
///
/// - [`Preference::Auto`] and [`Preference::LowPower`] use [`choose`].
/// - [`Preference::Adapter`] uses the usable entry that matches the key (the one exposing the most
///   features when a driver lists it twice), with [`choose`]'s corrections; without one, or when
///   it is an Intel Vulkan entry [`choose`] never draws with, the automatic choice and
///   [`Notice::AdapterMissing`].
/// - [`Preference::Software`] uses the first usable software adapter, with [`choose`]'s
///   corrections; without one, the automatic choice and [`Notice::NoSoftwareAdapter`].
///
/// A software adapter that the automatic choice took adds [`Notice::SoftwareOnly`].
pub fn select(candidates: &[Candidate], preference: &Preference) -> Option<Selection> {
    let mut notices = Vec::new();
    let wanted = match preference {
        Preference::Auto | Preference::LowPower => None,
        Preference::Adapter(key) => {
            // An entry that reports no bus id matches any saved bus. When it is the same model on
            // the same driver it cannot be told apart from an older version of that driver, and
            // the correction draws with the entry exposing the most features.
            let found =
                candidates.iter().enumerate().filter(|(_, c)| c.usable && key.matches(c)).max_by_key(|(i, c)| (c.features, std::cmp::Reverse(*i)));
            let found = found.and_then(|(index, _)| drawn_with(candidates, index));
            if found.is_none() {
                notices.push(Notice::AdapterMissing(key.clone()));
            }
            found
        }
        Preference::Software => {
            let found = candidates.iter().position(|c| c.usable && c.is_software()).and_then(|index| drawn_with(candidates, index));
            if found.is_none() {
                notices.push(Notice::NoSoftwareAdapter);
            }
            found
        }
    };
    let index = match wanted {
        Some(index) => index,
        None => {
            let index = choose(candidates, preference.power())?;
            if candidates.get(index).is_some_and(Candidate::is_software) {
                notices.push(Notice::SoftwareOnly);
            }
            index
        }
    };
    Some(Selection { index, notices })
}

/// The entry to draw with when the entry at `index` is asked for, with [`choose`]'s corrections:
/// the newest version of a driver that lists the GPU twice, or the DirectX 12 entry of an Intel
/// GPU whose Vulkan entry [`choose`] never draws with. `None` when that Intel Vulkan entry has no
/// usable DirectX 12 entry.
fn drawn_with(candidates: &[Candidate], index: usize) -> Option<usize> {
    let index = corrected(candidates, index, candidates.get(index)?);
    (!candidates.get(index).is_some_and(intel_vulkan_crashes)).then_some(index)
}

/// Whether `preference` asked for `candidate` itself: software for a software adapter, or the key
/// of this adapter.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn asked_for(preference: &Preference, candidate: &Candidate) -> bool {
    match preference {
        Preference::Software => candidate.is_software(),
        Preference::Adapter(key) => key.matches(candidate),
        Preference::Auto | Preference::LowPower => false,
    }
}

/// The order in which to try opening a device for `preference`: [`select`]'s choice first, then
/// every other usable adapter by device type for the preference's power (software after
/// hardware), leaving out the entries [`choose`] never draws with: an older version of a driver
/// listed twice, and the Vulkan entry of an Intel Gen 9 to Gen 11 GPU on Intel's Windows driver.
/// Empty when no adapter is usable.
pub fn ranked(candidates: &[Candidate], preference: &Preference) -> Vec<usize> {
    let Some(first) = select(candidates, preference).map(|s| s.index) else {
        return Vec::new();
    };
    let power = preference.power();
    let mut rest: Vec<usize> =
        (0..candidates.len()).filter(|&i| i != first && candidates.get(i).is_some_and(|c| c.usable) && !avoided(candidates, i)).collect();
    rest.sort_by_key(|&i| (candidates.get(i).map_or(u8::MAX, |c| type_order(c.device_type, power)), i));
    let mut order = vec![first];
    order.extend(rest);
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::tests::{gpu, two_amd_drivers, uhd_630};
    use wgpu::Backend::{Dx12, Vulkan};
    use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu};

    fn key_of(candidates: &[Candidate], index: usize) -> AdapterKey {
        candidates.get(index).map(AdapterKey::of).unwrap_or_default()
    }

    #[test]
    fn auto_and_low_power_are_the_corrected_wgpu_choice() {
        let adapters = two_amd_drivers();
        assert_eq!(select(&adapters, &Preference::Auto), Some(Selection { index: 1, notices: vec![] }));
        assert_eq!(select(&adapters, &Preference::LowPower).map(|s| s.index), Some(2));
        assert_eq!(select(&[], &Preference::Auto), None);
        assert_eq!(Preference::default(), Preference::Auto);
    }

    #[test]
    fn a_named_adapter_is_found_wherever_it_is_listed() {
        let adapters = two_amd_drivers();
        // the integrated Radeon through DirectX 12, picked by the user
        let key = key_of(&adapters, 4);
        assert_eq!(select(&adapters, &Preference::Adapter(key.clone())).map(|s| s.index), Some(4));
        // listed in another order on the next run
        let mut reordered = adapters.clone();
        reordered.reverse();
        assert_eq!(select(&reordered, &Preference::Adapter(key)).map(|s| s.index), Some(1));
        // the stale Vulkan entry of the RX 6800M stored as the choice: the newer version draws
        let stale = key_of(&adapters, 0);
        assert_eq!(select(&adapters, &Preference::Adapter(stale)).map(|s| s.index), Some(1));
    }

    #[test]
    fn a_named_intel_gen9_vulkan_entry_draws_through_directx_12() {
        let adapters = uhd_630();
        let vulkan = key_of(&adapters, 0);
        assert_eq!(select(&adapters, &Preference::Adapter(vulkan.clone())).map(|s| s.index), Some(1));
        // without the DirectX 12 entry, the named Vulkan entry is not available
        let mut dx12_unusable = adapters.clone();
        dx12_unusable[1].usable = false;
        let s = select(&dx12_unusable, &Preference::Adapter(vulkan.clone()));
        assert_eq!(s.map(|s| (s.index, s.notices)), Some((2, vec![Notice::AdapterMissing(vulkan), Notice::SoftwareOnly])));
    }

    #[test]
    fn a_missing_named_adapter_falls_back_to_auto_with_a_notice() {
        let adapters = two_amd_drivers();
        let gone = AdapterKey {
            name: "NVIDIA GeForce RTX 4090".into(),
            backend: "vulkan".into(),
            vendor: 0x10de,
            device: 0x2684,
            pci_bus_id: String::new(),
            driver: "NVIDIA".into(),
        };
        let s = select(&adapters, &Preference::Adapter(gone.clone()));
        assert_eq!(s, Some(Selection { index: 1, notices: vec![Notice::AdapterMissing(gone.clone())] }));
        // listed but unable to draw to the window counts as missing
        let mut unusable = adapters.clone();
        for c in &mut unusable {
            c.usable = c.backend != Vulkan;
        }
        let vulkan_key = key_of(&adapters, 1);
        let s = select(&unusable, &Preference::Adapter(vulkan_key.clone()));
        assert_eq!(s.map(|s| (s.index, s.notices)), Some((3, vec![Notice::AdapterMissing(vulkan_key)])));
        // nothing usable at all
        assert_eq!(select(&[], &Preference::Adapter(gone)), None);
    }

    #[test]
    fn keys_tell_apart_backends_buses_and_unidentified_names() {
        let adapters = two_amd_drivers();
        let dx12 = key_of(&adapters, 3);
        assert!(dx12.matches(&adapters[3]));
        assert!(!dx12.matches(&adapters[1]), "same GPU, other backend");
        let other_bus = Candidate { pci_bus_id: "0000:04:00.0".into(), ..adapters[3].clone() };
        assert!(!dx12.matches(&other_bus));
        // a key stored without a bus id still matches an entry that reports one
        let no_bus = AdapterKey { pci_bus_id: String::new(), ..dx12.clone() };
        assert!(no_bus.matches(&adapters[3]));
        // two Vulkan drivers for one GPU are told apart by the driver name; a key saved without
        // one matches either
        let radv = gpu("AMD Radeon RX 6800M (RADV NAVI22)", Vulkan, DiscreteGpu, 0x1002, 0x73df, "0000:03:00.0", "radv", "Mesa 25.2.4", 70);
        let amdvlk = gpu("AMD Radeon RX 6800M", Vulkan, DiscreteGpu, 0x1002, 0x73df, "0000:03:00.0", "AMD open-source driver", "2025.Q2.1", 72);
        let two = vec![radv.clone(), amdvlk.clone()];
        assert_eq!(select(&two, &Preference::Adapter(AdapterKey::of(&radv))).map(|s| s.index), Some(0));
        assert_eq!(select(&two, &Preference::Adapter(AdapterKey::of(&amdvlk))).map(|s| s.index), Some(1));
        let old = AdapterKey { driver: String::new(), ..AdapterKey::of(&radv) };
        assert!(old.matches(&radv) && old.matches(&amdvlk));
        // a DirectX 12 key keeps matching after a driver update changes the reported version
        assert!(dx12.driver.is_empty());
        let updated = Candidate { driver: "32.0.21045.12000".into(), ..adapters[3].clone() };
        assert!(dx12.matches(&updated));
        // OpenGL reports no ids: the name decides
        let angle = gpu("ANGLE (D3D11)", wgpu::Backend::Gl, wgpu::DeviceType::Other, 0, 0, "", "", "", 20);
        let swiftshader = gpu("SwiftShader", wgpu::Backend::Gl, wgpu::DeviceType::Other, 0, 0, "", "", "", 20);
        let key = AdapterKey::of(&angle);
        assert!(key.matches(&angle) && !key.matches(&swiftshader));
        assert_eq!(dx12.to_string(), "\"AMD Radeon RX 6800M\" (dx12)");
    }

    #[test]
    fn software_is_used_when_asked_and_reported_when_it_is_all_there_is() {
        let adapters = two_amd_drivers();
        assert_eq!(select(&adapters, &Preference::Software), Some(Selection { index: 5, notices: vec![] }));
        let hardware_only: Vec<Candidate> = adapters.iter().filter(|c| !c.is_software()).cloned().collect();
        assert_eq!(select(&hardware_only, &Preference::Software), Some(Selection { index: 1, notices: vec![Notice::NoSoftwareAdapter] }));
        let llvmpipe = vec![gpu("llvmpipe (LLVM 19.1.0, 256 bits)", Vulkan, Cpu, 0x10005, 0, "", "llvmpipe", "", 50)];
        assert_eq!(select(&llvmpipe, &Preference::Auto), Some(Selection { index: 0, notices: vec![Notice::SoftwareOnly] }));
        assert_eq!(select(&llvmpipe, &Preference::Software), Some(Selection { index: 0, notices: vec![] }));
        // picked by name, it is what was asked for
        let named = Preference::Adapter(AdapterKey::of(&llvmpipe[0]));
        assert_eq!(select(&llvmpipe, &named), Some(Selection { index: 0, notices: vec![] }));
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert!(asked_for(&named, &llvmpipe[0]) && asked_for(&Preference::Software, &llvmpipe[0]));
            assert!(!asked_for(&Preference::Auto, &llvmpipe[0]) && !asked_for(&Preference::Software, &adapters[1]));
        }
    }

    #[test]
    fn software_draws_with_the_newer_version_of_a_driver_listed_twice() {
        let stale = gpu("llvmpipe (LLVM 15.0.7, 256 bits)", Vulkan, Cpu, 0x10005, 0, "", "llvmpipe", "Mesa 23.0.4", 40);
        let newer = gpu("llvmpipe (LLVM 19.1.0, 256 bits)", Vulkan, Cpu, 0x10005, 0, "", "llvmpipe", "Mesa 25.2.4", 50);
        let hardware = gpu("AMD Radeon RX 6800M", Dx12, DiscreteGpu, 0x1002, 0x73df, "0000:03:00.0", "32.0.21045.11001", "", 36);
        let adapters = vec![stale, newer, hardware];
        assert_eq!(select(&adapters, &Preference::Software), Some(Selection { index: 1, notices: vec![] }));
        // the stale entry is never tried, and the newer one is not left out
        assert_eq!(ranked(&adapters, &Preference::Software), vec![1, 2]);
    }

    #[test]
    fn the_fallback_order_never_tries_entries_that_can_crash() {
        let adapters = two_amd_drivers();
        // the newer RX 6800M, then the other hardware by type for high performance, then software;
        // the stale RX 6800M entry is never tried
        assert_eq!(ranked(&adapters, &Preference::Auto), vec![1, 3, 2, 4, 5]);
        assert_eq!(ranked(&adapters, &Preference::LowPower), vec![2, 4, 1, 3, 5]);
        assert_eq!(ranked(&adapters, &Preference::Software), vec![5, 1, 3, 2, 4]);
        // two cards of one model on one driver, one without a Vulkan bus id: the second card's
        // Vulkan entry looks like an older version, and the card is tried through DirectX 12
        let card = |bus: &str| gpu("AMD Radeon RX 7900 XT", Vulkan, DiscreteGpu, 0x1002, 0x744c, bus, "AMD proprietary driver", "", 68);
        let dx12 = |bus: &str| gpu("AMD Radeon RX 7900 XT", Dx12, DiscreteGpu, 0x1002, 0x744c, bus, "32.0.21045.11001", "", 36);
        let twins = vec![card("0000:03:00.0"), card(""), dx12("0000:03:00.0"), dx12("0000:04:00.0")];
        assert_eq!(ranked(&twins, &Preference::Auto), vec![0, 2, 3]);
        // the same bus id on both entries: one GPU, and the older version is never tried
        let mut versions = vec![card("0000:03:00.0"), card("0000:03:00.0")];
        versions[1].features = 70;
        assert_eq!(ranked(&versions, &Preference::Auto), vec![1]);
        // a third entry without a bus id and with more features: both older entries are left out
        versions.push(card(""));
        versions[2].features = 80;
        assert_eq!(ranked(&versions, &Preference::Auto), vec![2]);
        // two GPUs on different buses are both tried
        let cards = vec![card("0000:03:00.0"), card("0000:04:00.0")];
        assert_eq!(ranked(&cards, &Preference::Auto), vec![0, 1]);
        // the Intel Gen 9 Vulkan entry is never tried, with or without a usable DirectX 12 entry
        assert_eq!(ranked(&uhd_630(), &Preference::Auto), vec![1, 2]);
        let mut dx12_unusable = uhd_630();
        dx12_unusable[1].usable = false;
        assert_eq!(ranked(&dx12_unusable, &Preference::Auto), vec![2]);
        assert!(ranked(&uhd_630()[..1], &Preference::Auto).is_empty());
        assert!(ranked(&[], &Preference::Auto).is_empty());
        let unusable: Vec<Candidate> = adapters.iter().map(|c| Candidate { usable: false, ..c.clone() }).collect();
        assert!(ranked(&unusable, &Preference::Auto).is_empty());
    }

    #[test]
    fn a_laptop_with_two_vendors_keeps_the_named_gpu_across_reorders() {
        let intel = gpu("Intel(R) UHD Graphics 770", Dx12, IntegratedGpu, 0x8086, 0x4680, "0000:00:02.0", "31.0.101.5186", "", 36);
        let nvidia = gpu("NVIDIA GeForce RTX 4070 Laptop GPU", Dx12, DiscreteGpu, 0x10de, 0x2820, "0000:01:00.0", "32.0.15.6094", "", 36);
        let a = vec![intel.clone(), nvidia.clone()];
        let b = vec![nvidia, intel];
        let picked = Preference::Adapter(key_of(&a, 0));
        assert_eq!(select(&a, &picked).map(|s| s.index), Some(0));
        assert_eq!(select(&b, &picked).map(|s| s.index), Some(1));
        assert_eq!(ranked(&b, &picked), vec![1, 0]);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn preferences_read_back_what_they_write() {
        let key = AdapterKey {
            name: "AMD Radeon RX 6800M".into(),
            backend: "dx12".into(),
            vendor: 0x1002,
            device: 0x73df,
            pci_bus_id: "0000:03:00.0".into(),
            driver: String::new(),
        };
        for p in [Preference::Auto, Preference::LowPower, Preference::Software, Preference::Adapter(key)] {
            let text = serde_json::to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<Preference>(&text).unwrap(), p, "{text}");
        }
        assert_eq!(serde_json::to_string(&Preference::Auto).unwrap(), r#"{"kind":"auto"}"#);
        // a key written by an older version without some fields
        let p: Preference = serde_json::from_str(r#"{"kind":"adapter","name":"x","backend":"vulkan"}"#).unwrap();
        assert_eq!(p, Preference::Adapter(AdapterKey { name: "x".into(), backend: "vulkan".into(), ..Default::default() }));
        assert!(serde_json::from_str::<Preference>(r#"{"kind":"fastest"}"#).is_err());
    }
}
