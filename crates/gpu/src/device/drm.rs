//! Which graphics cards drive displays, as the system reports them (Linux: `/sys/class/drm`).
//!
//! A window drawn on one card and shown on a screen attached to another is copied across on every
//! frame, so the app draws on the card that drives the screen (see the `linux` module).

use std::io::Read as _;
use std::path::Path;

/// A graphics card and the displays connected to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisplayCard {
    /// PCI vendor id: 0x1002 AMD, 0x10de NVIDIA, 0x8086 Intel.
    pub vendor: u32,
    /// PCI device id.
    pub device: u32,
    /// PCI address in the form `0000:05:00.0`, lowercase; empty when the system doesn't give one.
    pub pci_slot: String,
    /// Connected displays that are switched on, of any kind, built-in panel included.
    pub displays: u32,
    /// A connected, switched-on built-in panel (eDP, LVDS or DSI): this card drives a laptop's
    /// own screen.
    pub built_in_panel: bool,
}

/// Connector kinds that are a computer's own screen rather than a monitor plugged into it.
const BUILT_IN_PANELS: &[&str] = &["eDP", "LVDS", "DSI"];

/// At most this many entries are read from the DRM folder.
pub(crate) const MAX_ENTRIES: usize = 512;

/// At most this many bytes are read from one sysfs file.
pub(crate) const MAX_FILE: u64 = 4096;

/// The graphics cards on this computer and what's connected to them. Empty where the system
/// doesn't say (a sandbox without `/sys`).
#[cfg(target_os = "linux")]
pub(crate) fn display_cards() -> Vec<DisplayCard> {
    display_cards_in(Path::new("/sys/class/drm"))
}

/// The display cards read from a DRM folder laid out like `/sys/class/drm`: a `cardN` entry per
/// card, with the card's PCI ids in `cardN/device/uevent`, and a `cardN-<kind>-<n>` entry per
/// connector, with `connected` or `disconnected` in its `status` and, where the kernel gives it,
/// `enabled` or `disabled` in its `enabled`. Cards without PCI ids (a firmware framebuffer) are
/// left out.
pub(crate) fn display_cards_in(drm: &Path) -> Vec<DisplayCard> {
    let Ok(entries) = std::fs::read_dir(drm) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries.take(MAX_ENTRIES).filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
    names.sort();
    let cards = names.iter().filter(|n| is_card(n));
    cards
        .filter_map(|card| {
            let uevent = read_small(&drm.join(card).join("device").join("uevent"))?;
            let (vendor, device) = pci_ids(&uevent)?;
            let pci_slot = uevent_value(&uevent, "PCI_SLOT_NAME").unwrap_or_default().to_ascii_lowercase();
            let prefix = format!("{card}-");
            let mut out = DisplayCard { vendor, device, pci_slot, displays: 0, built_in_panel: false };
            for connector in names.iter().filter_map(|n| n.strip_prefix(&prefix)) {
                let dir = drm.join(format!("{prefix}{connector}"));
                let status = read_small(&dir.join("status")).unwrap_or_default();
                // A panel can be connected but switched off (a closed lid with a monitor in use):
                // `enabled` then reads `disabled`, and it drives nothing. No `enabled`: it counts.
                let enabled = read_small(&dir.join("enabled")).unwrap_or_default();
                if status.trim() != "connected" || enabled.trim() == "disabled" {
                    continue;
                }
                out.displays = out.displays.saturating_add(1);
                if BUILT_IN_PANELS.contains(&connector_kind(connector)) {
                    out.built_in_panel = true;
                }
            }
            Some(out)
        })
        .collect()
}

/// `card0`, `card12`: a card, not a connector (`card0-eDP-1`) or a render node.
pub(crate) fn is_card(name: &str) -> bool {
    name.strip_prefix("card").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `eDP-1` → `eDP`, `HDMI-A-1` → `HDMI-A`: the connector's name without its number.
pub(crate) fn connector_kind(connector: &str) -> &str {
    match connector.rsplit_once('-') {
        Some((kind, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => kind,
        _ => connector,
    }
}

/// `PCI_ID=1002:1638` → (0x1002, 0x1638).
pub(crate) fn pci_ids(uevent: &str) -> Option<(u32, u32)> {
    let (vendor, device) = uevent_value(uevent, "PCI_ID")?.split_once(':')?;
    Some((u32::from_str_radix(vendor.trim(), 16).ok()?, u32::from_str_radix(device.trim(), 16).ok()?))
}

/// The value of `key` in a `KEY=value` uevent file.
pub(crate) fn uevent_value<'a>(uevent: &'a str, key: &str) -> Option<&'a str> {
    uevent.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(str::trim)
}

/// The first [`MAX_FILE`] bytes of a sysfs file as text; `None` when it can't be read.
pub(crate) fn read_small(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(MAX_FILE).read_to_string(&mut text).ok()?;
    Some(text)
}
