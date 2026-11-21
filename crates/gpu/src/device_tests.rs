//! Trials of the GPU start-up choices. Every platform's choice is pure, so all of them run on every host.

use crate::device::drm::{self, DisplayCard, display_cards_in};
use crate::device::linux::{Asked, Candidate, Why, choose};
use crate::device::windows::{Dx12Adapter, choose_backends, dx12_compiler};
use egui_wgpu::wgpu::{Backends, DeviceType, Dx12Compiler, PowerPreference};
use std::fs;
use std::path::Path;

const fn adapter(device_type: DeviceType, meets_limits: bool) -> Dx12Adapter {
    Dx12Adapter { device_type, meets_limits }
}

/// #316: the backends Windows starts on. A wrong row loses the window on Intel 4th-generation and AMD machines.
#[test]
fn windows_backend_choice() {
    let warp_only = [adapter(DeviceType::Cpu, true)];
    let gpu = [adapter(DeviceType::IntegratedGpu, true)];
    // `WGPU_BACKEND` wins over the 32-bit rule and over a usable GPU.
    assert_eq!(choose_backends(Some(Backends::VULKAN), &gpu, false), Backends::VULKAN);
    let weak_gpu = [adapter(DeviceType::IntegratedGpu, false), adapter(DeviceType::Cpu, true)];
    let mixed = [adapter(DeviceType::Cpu, true), adapter(DeviceType::IntegratedGpu, false), adapter(DeviceType::DiscreteGpu, true)];
    let rows: [(&str, &[Dx12Adapter], bool, Backends); 6] = [
        ("no DirectX 12 adapter: OpenGL", &[], true, Backends::GL),
        ("only WARP: OpenGL, WARP kept as a last resort", &warp_only, true, Backends::DX12 | Backends::GL),
        ("a GPU that can't create the device beside WARP: OpenGL alone", &weak_gpu, true, Backends::GL),
        ("a usable GPU beside WARP and a weaker GPU: DirectX 12 only", &mixed, true, Backends::DX12),
        ("a usable integrated GPU: DirectX 12 only, no OpenGL", &gpu, true, Backends::DX12),
        ("32-bit with a usable GPU: OpenGL", &gpu, false, Backends::GL),
    ];
    for (case, adapters, dx12_graphics, want) in rows {
        assert_eq!(choose_backends(None, adapters, dx12_graphics), want, "{case}");
    }
}

/// #603: DirectX 12 shaders compile with FXC, never a `dxcompiler.dll` found on the search
/// path, unless `WGPU_DX12_COMPILER` asks for it.
#[test]
fn dx12_compiles_with_fxc_unless_asked() {
    assert!(matches!(dx12_compiler(None), Dx12Compiler::Fxc));
    assert!(matches!(dx12_compiler(Some("bogus")), Dx12Compiler::Fxc));
    assert!(matches!(dx12_compiler(Some("dxc")), Dx12Compiler::DynamicDxc { .. }));
}

const AMD: (u32, u32, &str) = (0x1002, 0x1638, "0000:05:00.0");
const NVIDIA: (u32, u32, &str) = (0x10de, 0x25a2, "0000:01:00.0");
const INTEL: (u32, u32, &str) = (0x8086, 0xa780, "0000:00:02.0");

fn vulkan(name: &str, (vendor, device, slot): (u32, u32, &str), device_type: DeviceType) -> Candidate {
    Candidate { name: name.into(), vendor, device, pci_slot: slot.into(), device_type, gl: false }
}

fn gl(name: &str, vendor: u32, device_type: DeviceType) -> Candidate {
    Candidate { name: name.into(), vendor, device: 0, pci_slot: String::new(), device_type, gl: true }
}

fn card((vendor, device, slot): (u32, u32, &str), displays: u32, built_in_panel: bool) -> DisplayCard {
    DisplayCard { vendor, device, pci_slot: slot.into(), displays, built_in_panel }
}

/// What wgpu offers on a laptop whose AMD iGPU drives the panel, in its order (Vulkan, then GL).
fn laptop_adapters() -> Vec<Candidate> {
    vec![
        vulkan("AMD Radeon Graphics (RADV RENOIR)", AMD, DeviceType::IntegratedGpu),
        vulkan("NVIDIA GeForce RTX 3050 Laptop GPU", NVIDIA, DeviceType::DiscreteGpu),
        gl("AMD Radeon Graphics (radeonsi, renoir)", 0x1002, DeviceType::IntegratedGpu),
    ]
}

/// The adapter behind the displays draws: a laptop stays off its discrete GPU (7 W against over 20 W), a desktop
/// keeps its surface.
#[test]
fn the_adapter_behind_the_displays_is_chosen() {
    let none = Asked::default();
    let laptop = laptop_adapters();

    // The iGPU drives the panel; egui-wgpu's default drew on the discrete GPU.
    let panel_on_amd = [card(NVIDIA, 0, false), card(AMD, 1, true)];
    assert_eq!(choose(&laptop, &panel_on_amd, &none), Some((0, Why::BuiltInPanel)), "laptop panel");
    let mut swapped = laptop_adapters();
    swapped.swap(0, 1);
    assert_eq!(choose(&swapped, &panel_on_amd, &none), Some((1, Why::BuiltInPanel)), "laptop panel, discrete listed first");
    let monitor_on_nvidia = [card(NVIDIA, 1, false), card(AMD, 1, true)];
    assert_eq!(choose(&laptop, &monitor_on_nvidia, &none), Some((0, Why::BuiltInPanel)), "the panel wins over a monitor on the other card");

    // The measured laptop: Intel HD 530 behind the panel, a Quadro M4000M offered first with nothing connected.
    const HD530: (u32, u32, &str) = (0x8086, 0x191b, "0000:00:02.0");
    const M4000M: (u32, u32, &str) = (0x10de, 0x13f9, "0000:01:00.0");
    let measured =
        [vulkan("Quadro M4000M", M4000M, DeviceType::DiscreteGpu), vulkan("Intel(R) HD Graphics 530 (SKL GT2)", HD530, DeviceType::IntegratedGpu)];
    assert_eq!(choose(&measured, &[card(M4000M, 0, false), card(HD530, 1, true)], &none), Some((1, Why::BuiltInPanel)), "Intel and NVIDIA laptop");

    // A desktop with its only monitor on one card (a low-power default can't start the window there).
    let desktop =
        [vulkan("Intel(R) Graphics (RPL-S)", INTEL, DeviceType::IntegratedGpu), vulkan("NVIDIA GeForce RTX", NVIDIA, DeviceType::DiscreteGpu)];
    assert_eq!(
        choose(&desktop, &[card(INTEL, 0, false), card(NVIDIA, 1, false)], &none),
        Some((1, Why::OnlyGpuWithDisplays)),
        "monitor on the discrete card"
    );
    assert_eq!(choose(&desktop, &[card(INTEL, 2, false), card(NVIDIA, 0, false)], &none), Some((0, Why::OnlyGpuWithDisplays)), "monitor on the iGPU");
    // A monitor on each card: the discrete GPU (an iGPU choice there lost its surface).
    assert_eq!(choose(&desktop, &[card(INTEL, 1, false), card(NVIDIA, 1, false)], &none), Some((1, Why::Fallback)), "monitors on two cards");

    // Nothing readable: the discrete GPU.
    assert_eq!(choose(&laptop, &[], &none), Some((1, Why::Fallback)), "no display facts");
    assert_eq!(choose(&laptop, &[card((0x1234, 0x1, "0000:09:00.0"), 1, true)], &none), Some((1, Why::Fallback)), "a card no adapter matches");
    assert_eq!(choose(&[], &[card(AMD, 1, true)], &none), None, "no adapter at all");

    // The environment variables decide when set: prime-run style requests reach the NVIDIA card.
    let asked = |name: Option<&str>, power: Option<PowerPreference>| Asked { adapter_name: name.map(Into::into), power };
    assert_eq!(choose(&laptop, &panel_on_amd, &asked(None, Some(PowerPreference::HighPerformance))), Some((1, Why::AskedPower)));
    assert_eq!(choose(&laptop, &panel_on_amd, &asked(None, Some(PowerPreference::LowPower))), Some((0, Why::AskedPower)));
    assert_eq!(choose(&laptop, &panel_on_amd, &asked(None, Some(PowerPreference::None))), Some((0, Why::AskedPower)));
    assert_eq!(choose(&laptop, &panel_on_amd, &asked(Some("nvidia"), None)), Some((1, Why::AskedByName)));
    assert_eq!(
        choose(&laptop, &panel_on_amd, &asked(Some("RadeonSI"), Some(PowerPreference::HighPerformance))),
        Some((2, Why::AskedByName)),
        "the name beats the power"
    );
    // A name nothing has, or an empty one: the usual choice, not a failure.
    assert_eq!(choose(&laptop, &panel_on_amd, &asked(Some("matrox"), None)), Some((0, Why::BuiltInPanel)));
    assert_eq!(choose(&laptop, &panel_on_amd, &asked(Some(""), None)), Some((0, Why::BuiltInPanel)));

    // Under GL there's no PCI address or device id: the vendor finds the card only when no other card shares it.
    let gl_adapters =
        [gl("NVIDIA GeForce RTX 3050", 0x10de, DeviceType::Other), gl("AMD Radeon Graphics (radeonsi)", 0x1002, DeviceType::IntegratedGpu)];
    assert_eq!(choose(&gl_adapters, &panel_on_amd, &none), Some((1, Why::BuiltInPanel)), "GL, one AMD card");
    let two_amd = [card((0x1002, 0x73df, "0000:03:00.0"), 0, false), card(AMD, 1, true)];
    let gl_two_amd = [gl("AMD Radeon RX 6700", 0x1002, DeviceType::Other), gl("AMD Radeon Graphics", 0x1002, DeviceType::IntegratedGpu)];
    assert_eq!(choose(&gl_two_amd, &two_amd, &none), Some((1, Why::Fallback)), "GL, two AMD cards");

    // Two adapters for the panel's card: Vulkan, whatever the order.
    let gl_first = [
        gl("AMD Radeon Graphics (radeonsi)", 0x1002, DeviceType::IntegratedGpu),
        vulkan("AMD Radeon Graphics (RADV RENOIR)", AMD, DeviceType::IntegratedGpu),
    ];
    assert_eq!(choose(&gl_first, &[card(AMD, 1, true)], &none), Some((1, Why::BuiltInPanel)), "Vulkan before GL");

    // A closed laptop with a monitor on the NVIDIA card: the switched-off panel isn't one (`drm` skips it).
    assert_eq!(choose(&laptop, &[card(NVIDIA, 1, false), card(AMD, 0, false)], &none), Some((1, Why::OnlyGpuWithDisplays)), "closed laptop");

    // Two cards of one model: only the PCI address tells them apart; without one the old choice stands.
    const SECOND: (u32, u32, &str) = (0x10de, 0x25a2, "0000:02:00.0");
    let twins = [card(NVIDIA, 0, false), card(SECOND, 1, false)];
    let with_slots = [vulkan("NVIDIA A", NVIDIA, DeviceType::DiscreteGpu), vulkan("NVIDIA B", SECOND, DeviceType::DiscreteGpu)];
    assert_eq!(choose(&with_slots, &twins, &none), Some((1, Why::OnlyGpuWithDisplays)), "twins with PCI addresses");
    let without: Vec<Candidate> = with_slots.iter().cloned().map(|c| Candidate { pci_slot: String::new(), ..c }).collect();
    assert_eq!(choose(&without, &twins, &none), Some((0, Why::Fallback)), "twins without PCI addresses");
}

fn add_card(dir: &Path, card: &str, id: &str, slot: &str) {
    let device = dir.join(card).join("device");
    fs::create_dir_all(&device).unwrap();
    fs::write(device.join("uevent"), format!("DRIVER=x\nPCI_CLASS=30000\nPCI_ID={id}\nPCI_SLOT_NAME={slot}\n")).unwrap();
}

/// A fake `/sys/class/drm`: cards as (name, PCI_ID, PCI_SLOT_NAME), connectors as (name, status), and the render
/// node and version file a real one has.
fn drm(cards: &[(&str, &str, &str)], connectors: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (card, id, slot) in cards {
        add_card(dir.path(), card, id, slot);
    }
    for (connector, status) in connectors {
        let c = dir.path().join(connector);
        fs::create_dir_all(&c).unwrap();
        fs::write(c.join("status"), format!("{status}\n")).unwrap();
    }
    fs::create_dir_all(dir.path().join("renderD128")).unwrap();
    fs::write(dir.path().join("version"), "drm 1.1.0\n").unwrap();
    dir
}

/// Parsing of the kernel's DRM folder: hostile or odd content is left out, never a panic.
#[test]
fn sysfs_display_cards() {
    // A laptop whose AMD iGPU drives the panel, with an NVIDIA card that has no outputs.
    let d = drm(
        &[("card0", "10DE:25A2", "0000:01:00.0"), ("card1", "1002:1638", "0000:05:00.0")],
        &[("card1-DP-1", "disconnected"), ("card1-eDP-1", "connected"), ("card1-HDMI-A-1", "disconnected")],
    );
    assert_eq!(
        display_cards_in(d.path()),
        vec![
            DisplayCard { vendor: 0x10de, device: 0x25a2, pci_slot: "0000:01:00.0".into(), displays: 0, built_in_panel: false },
            DisplayCard { vendor: 0x1002, device: 0x1638, pci_slot: "0000:05:00.0".into(), displays: 1, built_in_panel: true },
        ],
        "laptop panel"
    );

    // A desktop with monitors on each card: counted on each, no built-in panel.
    let d = drm(
        &[("card0", "8086:A780", "0000:00:02.0"), ("card1", "10de:2206", "0000:01:00.0")],
        &[("card0-HDMI-A-1", "connected"), ("card1-DP-1", "connected"), ("card1-DP-2", "connected"), ("card1-DP-3", "disconnected")],
    );
    let counts: Vec<(u32, bool)> = display_cards_in(d.path()).iter().map(|c| (c.displays, c.built_in_panel)).collect();
    assert_eq!(counts, vec![(1, false), (2, false)], "monitors on two cards");

    // `card10` is a different card from `card1`, and its connectors aren't `card1`'s.
    let d = drm(&[("card1", "1002:1638", "0000:05:00.0"), ("card10", "10de:25a2", "0000:01:00.0")], &[("card10-DP-1", "connected")]);
    let counts: Vec<(u32, u32)> = display_cards_in(d.path()).iter().map(|c| (c.vendor, c.displays)).collect();
    assert_eq!(counts, vec![(0x1002, 0), (0x10de, 1)], "card10 is not card1");

    // LVDS and DSI panels are built in too; HDMI-A is a monitor, whatever its number.
    assert_eq!(drm::connector_kind("eDP-1"), "eDP");
    assert_eq!(drm::connector_kind("HDMI-A-1"), "HDMI-A");
    assert_eq!(drm::connector_kind("DSI-12"), "DSI");
    assert_eq!(drm::connector_kind("Virtual"), "Virtual");
    let d = drm(&[("card0", "8086:9a49", "0000:00:02.0")], &[("card0-LVDS-1", "connected")]);
    let cards = display_cards_in(d.path());
    assert!(!cards.is_empty() && cards.iter().all(|c| c.built_in_panel), "LVDS is a panel: {cards:?}");

    // A status of `unknown` (some virtual outputs) isn't a connected display.
    let d = drm(&[("card0", "1af4:1050", "0000:00:01.0")], &[("card0-Virtual-1", "unknown")]);
    assert_eq!(display_cards_in(d.path()).first().map(|c| c.displays), Some(0), "only connected counts");

    // A closed lid with a monitor on the discrete card: the panel reads `connected` but is switched off.
    let d = drm(
        &[("card0", "10de:25a2", "0000:01:00.0"), ("card1", "1002:1638", "0000:05:00.0")],
        &[("card0-HDMI-A-1", "connected"), ("card1-eDP-1", "connected")],
    );
    fs::write(d.path().join("card0-HDMI-A-1").join("enabled"), "enabled\n").unwrap();
    fs::write(d.path().join("card1-eDP-1").join("enabled"), "disabled\n").unwrap();
    let seen: Vec<(u32, u32, bool)> = display_cards_in(d.path()).iter().map(|c| (c.vendor, c.displays, c.built_in_panel)).collect();
    assert_eq!(seen, vec![(0x10de, 1, false), (0x1002, 0, false)], "a switched-off panel does not count");

    // A firmware framebuffer has no PCI ids; unreadable or odd entries are skipped.
    let d = drm(&[("card0", "nonsense", ""), ("card1", "1002:", "")], &[("card0-Virtual-1", "connected")]);
    fs::create_dir_all(d.path().join("card2")).unwrap();
    fs::create_dir_all(d.path().join("card")).unwrap();
    assert!(display_cards_in(d.path()).is_empty(), "unreadable cards are left out");
    assert!(display_cards_in(&d.path().join("missing")).is_empty(), "a missing folder gives no cards");
    assert_eq!(drm::pci_ids("PCI_ID=10de:25a2"), Some((0x10de, 0x25a2)));
    assert_eq!(drm::pci_ids("PCI_IDX=10de:25a2"), None);

    // The bounds: a uevent whose ids lie past `MAX_FILE` bytes isn't read that far, and no more than `MAX_ENTRIES`
    // entries of the folder are looked at.
    let d = tempfile::tempdir().unwrap();
    let device = d.path().join("card0").join("device");
    fs::create_dir_all(&device).unwrap();
    let padding = "x".repeat(usize::try_from(drm::MAX_FILE).unwrap() * 2);
    fs::write(device.join("uevent"), format!("PAD={padding}\nPCI_ID=1002:1638\n")).unwrap();
    assert!(display_cards_in(d.path()).is_empty(), "ids past the read bound are not seen");
    let d = tempfile::tempdir().unwrap();
    for n in 0..drm::MAX_ENTRIES + 40 {
        add_card(d.path(), &format!("card{n}"), "1002:1638", "");
    }
    assert_eq!(display_cards_in(d.path()).len(), drm::MAX_ENTRIES, "the folder is read up to its entry bound");
}
