//! Linux: draw on the GPU that drives the screen.
//!
//! egui-wgpu picks the discrete GPU by default (`PowerPreference::HighPerformance`). On a laptop
//! whose integrated GPU drives the panel, every frame drawn on the discrete one is copied across
//! to it, and the discrete GPU leaves its idle state whenever the window redraws: on an Intel HD
//! 530 + NVIDIA Quadro M4000M laptop, from 7 W at P8 to over 20 W at P0/P1. Forcing the integrated
//! GPU instead breaks desktops whose screen is on the discrete card (storytold/pdfcraft#96), and a
//! desktop with a monitor on each card needs the discrete one (storytold/pdfcraft#445). So the
//! choice follows the connected displays, as in craft-anything (storytold/craft-anything#571):
//!
//! 1. `WGPU_ADAPTER_NAME` (part of the adapter's name) or `WGPU_POWER_PREF` (`high`, `low`,
//!    `none`), when set, decide;
//! 2. a connected built-in panel: the GPU behind it;
//! 3. displays on only one GPU: that GPU;
//! 4. otherwise (displays on several GPUs, or nothing readable): the discrete GPU, as egui-wgpu does.

use super::drm::DisplayCard;
use egui_wgpu::wgpu::{DeviceType, PowerPreference};

/// What we know of one adapter wgpu offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    /// `0000:05:00.0`, lowercase; empty when the driver doesn't say (always, under GL).
    pub pci_slot: String,
    pub device_type: DeviceType,
    pub gl: bool,
}

/// What the person asked for through wgpu's environment variables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Asked {
    pub adapter_name: Option<String>,
    pub power: Option<PowerPreference>,
}

/// Why an adapter was chosen, for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Why {
    AskedByName,
    AskedPower,
    BuiltInPanel,
    OnlyGpuWithDisplays,
    Fallback,
}

/// The index of the adapter to draw with, and why; `None` when there is none.
pub(crate) fn choose(candidates: &[Candidate], cards: &[DisplayCard], asked: &Asked) -> Option<(usize, Why)> {
    if let Some(want) = asked.adapter_name.as_deref().map(str::to_lowercase).filter(|w| !w.is_empty()) {
        // As wgpu's own `WGPU_ADAPTER_NAME`: part of the name, any case. No match: the usual choice.
        if let Some(i) = candidates.iter().position(|c| c.name.to_lowercase().contains(&want)) {
            return Some((i, Why::AskedByName));
        }
        log::warn!("WGPU_ADAPTER_NAME={want}: no graphics adapter has that in its name");
    }
    if let Some(power) = asked.power {
        return by_kind(candidates, power).map(|i| (i, Why::AskedPower));
    }
    let panels: Vec<&DisplayCard> = cards.iter().filter(|c| c.built_in_panel).collect();
    let with_displays: Vec<&DisplayCard> = cards.iter().filter(|c| c.displays > 0).collect();
    let target = if !panels.is_empty() {
        Some((panels, Why::BuiltInPanel))
    } else if with_displays.len() == 1 {
        Some((with_displays, Why::OnlyGpuWithDisplays))
    } else {
        None
    };
    if let Some((wanted, why)) = target {
        // Vulkan before GL for the same card; otherwise wgpu's order.
        let found = candidates.iter().enumerate().filter(|(_, c)| wanted.iter().any(|card| drives(card, c, cards))).min_by_key(|(_, c)| c.gl);
        if let Some((i, _)) = found {
            return Some((i, why));
        }
    }
    by_kind(candidates, PowerPreference::HighPerformance).map(|i| (i, Why::Fallback))
}

/// Whether `card` is the GPU behind `adapter`: by PCI address when the driver gives one, else by
/// PCI ids, which count only when no other card has the same ids (two cards of one model); GL
/// gives only a vendor, which counts only when no other card has that vendor.
fn drives(card: &DisplayCard, adapter: &Candidate, all: &[DisplayCard]) -> bool {
    if !adapter.pci_slot.is_empty() && !card.pci_slot.is_empty() {
        return adapter.pci_slot == card.pci_slot;
    }
    if adapter.device != 0 {
        return adapter.vendor == card.vendor
            && adapter.device == card.device
            && all.iter().filter(|c| c.vendor == card.vendor && c.device == card.device).count() == 1;
    }
    adapter.vendor != 0 && adapter.vendor == card.vendor && all.iter().filter(|c| c.vendor == card.vendor).count() == 1
}

/// The first adapter in wgpu's own order for that power preference.
fn by_kind(candidates: &[Candidate], power: PowerPreference) -> Option<usize> {
    let rank = |t: DeviceType| match (power, t) {
        (PowerPreference::None, _) => 0,
        (PowerPreference::LowPower, DeviceType::IntegratedGpu) | (PowerPreference::HighPerformance, DeviceType::DiscreteGpu) => 1,
        (_, DeviceType::IntegratedGpu | DeviceType::DiscreteGpu) => 2,
        (_, DeviceType::Other) => 3,
        (_, DeviceType::VirtualGpu) => 4,
        (_, DeviceType::Cpu) => 5,
    };
    candidates.iter().enumerate().min_by_key(|(_, c)| rank(c.device_type)).map(|(i, _)| i)
}

#[cfg(target_os = "linux")]
impl Candidate {
    fn of(info: &egui_wgpu::wgpu::AdapterInfo) -> Self {
        Candidate {
            name: info.name.clone(),
            vendor: info.vendor,
            device: info.device,
            pci_slot: info.device_pci_bus_id.to_ascii_lowercase(),
            device_type: info.device_type,
            gl: info.backend == egui_wgpu::wgpu::Backend::Gl,
        }
    }
}

#[cfg(target_os = "linux")]
impl Asked {
    fn from_env() -> Self {
        Asked { adapter_name: std::env::var("WGPU_ADAPTER_NAME").ok(), power: PowerPreference::from_env() }
    }
}

/// The adapter that draws the window, among those that can draw to it: egui-wgpu's
/// `native_adapter_selector`.
#[cfg(target_os = "linux")]
pub(crate) fn pick_adapter(
    adapters: &[egui_wgpu::wgpu::Adapter],
    surface: Option<&egui_wgpu::wgpu::Surface<'_>>,
) -> Result<egui_wgpu::wgpu::Adapter, String> {
    let usable: Vec<&egui_wgpu::wgpu::Adapter> = adapters.iter().filter(|a| surface.is_none_or(|s| a.is_surface_supported(s))).collect();
    let candidates: Vec<Candidate> = usable.iter().map(|a| Candidate::of(&a.get_info())).collect();
    let cards = super::drm::display_cards();
    let picked = choose(&candidates, &cards, &Asked::from_env()).and_then(|(i, why)| Some((*usable.get(i)?, why)));
    match picked {
        Some((a, why)) => {
            log::info!("drawing with {:?} ({why:?}); displays: {cards:?}", a.get_info());
            Ok(a.clone())
        }
        None => Err(format!("None of the {} graphics adapters found can draw this window.", adapters.len())),
    }
}
