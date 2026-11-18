//! Listing adapters and opening a device, trying the next adapter when one fails.

use std::fmt;

#[cfg(not(target_arch = "wasm32"))]
use wgpu::Instance;
use wgpu::{Adapter, Features, Limits, Surface};

use crate::adapter::{Candidate, avoided, log_choice};
use crate::caps::Capabilities;
use crate::health::DeviceHealth;
use crate::preference::{Notice, Preference, Selection, select};
#[cfg(not(target_arch = "wasm32"))]
use crate::preference::{asked_for, ranked};

/// What the device is opened with.
#[derive(Clone, Debug, Default)]
pub struct Request<'a> {
    pub label: Option<&'a str>,
    /// Features the device must have. An adapter without them is skipped.
    pub required_features: Features,
    /// Features asked for when the adapter has them.
    pub optional_features: Features,
    /// The limits to ask for; `None` asks for the adapter's own limits.
    pub limits: Option<Limits>,
}

/// An opened device.
#[derive(Debug)]
pub struct Opened {
    pub adapter: Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// The adapter as plain data.
    pub candidate: Candidate,
    /// What the device was opened with.
    pub caps: Capabilities,
    /// Watches the device for a loss and for uncaptured errors.
    pub health: DeviceHealth,
    /// What to tell the user about the choice.
    pub notices: Vec<Notice>,
}

/// Why no device could be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// No adapter is listed that can draw (to the surface, when there is one).
    NoAdapter,
    /// Every adapter tried failed to open a device; each failure is a [`Notice::DeviceFailed`].
    NoDevice(Vec<Notice>),
    /// The only adapters that can draw are ones [`choose`](crate::choose) never draws with,
    /// because their driver can end the process; these are their names.
    UnsafeDriver(Vec<String>),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::NoAdapter => f.write_str("no graphics adapter is available"),
            OpenError::NoDevice(notices) => {
                f.write_str("no graphics adapter could be opened")?;
                for n in notices {
                    if let Notice::DeviceFailed { adapter, error } = n {
                        write!(f, "; \"{adapter}\": {error}")?;
                    }
                }
                Ok(())
            }
            OpenError::UnsafeDriver(names) => {
                let names = names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ");
                write!(f, "the only graphics adapters that can draw ({names}) use a driver that can crash; update the graphics driver")
            }
        }
    }
}

impl std::error::Error for OpenError {}

/// Every adapter `instance` lists over its backends, as adapters and as plain data. `surface`
/// marks the adapters that cannot draw to it as unusable. Native only: it blocks on wgpu's
/// future, which on the web would wait on the thread that must resolve it.
#[cfg(not(target_arch = "wasm32"))]
pub fn list(instance: &Instance, surface: Option<&Surface<'_>>) -> (Vec<Adapter>, Vec<Candidate>) {
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    let candidates = adapters.iter().map(|a| Candidate::of(a, surface)).collect();
    (adapters, candidates)
}

/// The adapter to draw with for `preference`, from adapters already listed, with the choice in the
/// log. This fits egui-wgpu's `native_adapter_selector` for a window, which then opens the device
/// itself; that selector takes a `String` error, so convert with `map_err(|e| e.to_string())`.
/// With no usable adapter the error is [`OpenError::UnsafeDriver`] when only adapters that
/// [`choose`](crate::choose) never draws with can draw, and [`OpenError::NoAdapter`] otherwise,
/// with the listed adapters in the log.
pub fn pick(adapters: &[Adapter], surface: Option<&Surface<'_>>, preference: &Preference) -> Result<(Adapter, Selection), OpenError> {
    let candidates: Vec<Candidate> = adapters.iter().map(|a| Candidate::of(a, surface)).collect();
    let Some(selection) = select(&candidates, preference) else {
        return Err(unselected(&candidates));
    };
    log_choice(&candidates, selection.index, preference.power());
    for notice in &selection.notices {
        log::warn!("{notice}");
    }
    let adapter = adapters.get(selection.index).cloned().ok_or(OpenError::NoAdapter)?;
    Ok((adapter, selection))
}

/// The error for `candidates` among which `select` found no adapter, with the list in the log
/// when it is not [`OpenError::UnsafeDriver`].
fn unselected(candidates: &[Candidate]) -> OpenError {
    let left_out = left_out(candidates);
    if !left_out.is_empty() {
        return OpenError::UnsafeDriver(left_out);
    }
    let listed = candidates.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
    log::warn!("no usable graphics adapter among {}: {listed}", candidates.len());
    OpenError::NoAdapter
}

/// Open a device for `preference`: the selected adapter first, then the others in [`ranked`]
/// order, and last wgpu's fallback adapter (a software one) when none of those opened. An adapter
/// without `request.required_features`, or whose device fails to open, adds a
/// [`Notice::DeviceFailed`] and the next one is tried. Native only, like [`list`].
#[cfg(not(target_arch = "wasm32"))]
pub fn open(instance: &Instance, surface: Option<&Surface<'_>>, preference: &Preference, request: &Request<'_>) -> Result<Opened, OpenError> {
    let (adapters, candidates) = list(instance, surface);
    // Whether drawing is in software depends on the adapter that opens, which `finish` knows.
    let mut notices: Vec<Notice> =
        select(&candidates, preference).map(|s| s.notices).unwrap_or_default().into_iter().filter(|n| *n != Notice::SoftwareOnly).collect();
    let order = ranked(&candidates, preference);
    if let Some(first) = order.first() {
        log_choice(&candidates, *first, preference.power());
    }
    let mut tried_software = false;
    for index in order {
        let (Some(adapter), Some(candidate)) = (adapters.get(index), candidates.get(index)) else {
            continue;
        };
        tried_software |= candidate.is_software();
        match open_on(adapter, request) {
            Ok((device, queue)) => return Ok(finish(adapter.clone(), device, queue, candidate.clone(), preference, notices)),
            Err(error) => {
                log::warn!("graphics adapter {candidate} could not be opened: {error}");
                notices.push(Notice::DeviceFailed { adapter: candidate.name.clone(), error });
            }
        }
    }
    if !tried_software {
        let options = wgpu::RequestAdapterOptions { force_fallback_adapter: true, compatible_surface: surface, ..Default::default() };
        if let Ok(adapter) = pollster::block_on(instance.request_adapter(&options)) {
            let candidate = Candidate::of(&adapter, surface);
            match open_on(&adapter, request) {
                Ok((device, queue)) => {
                    log::info!("graphics adapter chosen: {candidate}");
                    return Ok(finish(adapter, device, queue, candidate, preference, notices));
                }
                Err(error) => notices.push(Notice::DeviceFailed { adapter: candidate.name.clone(), error }),
            }
        }
    }
    if notices.iter().any(|n| matches!(n, Notice::DeviceFailed { .. })) {
        return Err(OpenError::NoDevice(notices));
    }
    let left_out = left_out(&candidates);
    if left_out.is_empty() { Err(OpenError::NoAdapter) } else { Err(OpenError::UnsafeDriver(left_out)) }
}

/// The names of the adapters that can draw but that [`choose`](crate::choose) never draws with.
fn left_out(candidates: &[Candidate]) -> Vec<String> {
    candidates.iter().enumerate().filter(|(i, c)| c.usable && avoided(candidates, *i)).map(|(_, c)| c.name.clone()).collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn finish(
    adapter: Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    candidate: Candidate,
    preference: &Preference,
    mut notices: Vec<Notice>,
) -> Opened {
    let caps = Capabilities::of_device(&adapter, &device);
    if candidate.is_software() && !asked_for(preference, &candidate) {
        notices.push(Notice::SoftwareOnly);
    }
    for notice in &notices {
        log::warn!("{notice}");
    }
    log::info!("graphics device opened: {caps}");
    let health = DeviceHealth::watch(&device);
    Opened { adapter, device, queue, candidate, caps, health, notices }
}

/// Open a device on `adapter`, or say why it cannot be.
#[cfg(not(target_arch = "wasm32"))]
fn open_on(adapter: &Adapter, request: &Request<'_>) -> Result<(wgpu::Device, wgpu::Queue), String> {
    let has = adapter.features();
    if !has.contains(request.required_features) {
        return Err(format!("it lacks the features {:?}", request.required_features - has));
    }
    let required_limits = request.limits.clone().unwrap_or_else(|| adapter.limits());
    let desc = wgpu::DeviceDescriptor {
        label: request.label,
        required_features: request.required_features | (request.optional_features & has),
        required_limits,
        ..Default::default()
    };
    pollster::block_on(adapter.request_device(&desc)).map_err(|e| e.to_string())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::preference::AdapterKey;

    /// An instance whose only adapter is wgpu's noop one, which opens devices without a GPU. wgpu
    /// 30 lists it as a software adapter whatever `NoopBackendOptions::device_type` says.
    fn noop(features: Option<Features>) -> Instance {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::NOOP;
        desc.backend_options.noop = wgpu::NoopBackendOptions { enable: true, features, ..Default::default() };
        Instance::new(desc)
    }

    #[test]
    fn a_device_opens_with_the_optional_features_the_adapter_has() {
        let instance = noop(Some(Features::TEXTURE_FORMAT_NV12 | Features::TIMESTAMP_QUERY));
        let request = Request { optional_features: Features::TEXTURE_FORMAT_NV12 | Features::SHADER_F16, ..Default::default() };
        let opened = open(&instance, None, &Preference::Software, &request).unwrap();
        assert!(opened.caps.nv12, "{}", opened.caps);
        assert!(!opened.caps.shader_f16 && !opened.caps.timestamps, "only the features asked for: {}", opened.caps);
        assert!(opened.notices.is_empty(), "{:?}", opened.notices);
        assert!(opened.health.is_ok());
    }

    #[test]
    fn an_adapter_without_a_required_feature_is_reported() {
        let instance = noop(Some(Features::empty()));
        let request = Request { required_features: Features::SHADER_F16, ..Default::default() };
        let err = open(&instance, None, &Preference::Auto, &request).unwrap_err();
        let OpenError::NoDevice(notices) = &err else { panic!("{err}") };
        assert!(notices.iter().any(|n| matches!(n, Notice::DeviceFailed { error, .. } if error.contains("SHADER_F16"))), "{notices:?}");
        assert!(err.to_string().starts_with("no graphics adapter could be opened"), "{err}");
    }

    #[test]
    fn a_software_adapter_is_reported_unless_it_was_asked_for() {
        let cpu = noop(None);
        let opened = open(&cpu, None, &Preference::Auto, &Request::default()).unwrap();
        assert!(opened.caps.software);
        assert_eq!(opened.notices, vec![Notice::SoftwareOnly]);
        let asked = open(&cpu, None, &Preference::Software, &Request::default()).unwrap();
        assert!(asked.notices.is_empty(), "{:?}", asked.notices);
        // the software adapter picked by name is what was asked for
        let named = open(&cpu, None, &Preference::Adapter(AdapterKey::of(&asked.candidate)), &Request::default()).unwrap();
        assert!(named.notices.is_empty(), "{:?}", named.notices);
        // a named adapter that is gone: the automatic choice, with both notices
        let gone =
            AdapterKey { name: "gone".into(), backend: "vulkan".into(), vendor: 0x10de, device: 1, pci_bus_id: String::new(), driver: String::new() };
        let opened = open(&cpu, None, &Preference::Adapter(gone.clone()), &Request::default()).unwrap();
        assert_eq!(opened.notices, vec![Notice::AdapterMissing(gone), Notice::SoftwareOnly]);
    }

    #[test]
    fn no_adapter_is_an_error() {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::NOOP;
        let instance = Instance::new(desc);
        assert_eq!(open(&instance, None, &Preference::Auto, &Request::default()).unwrap_err(), OpenError::NoAdapter);
        assert_eq!(OpenError::NoAdapter.to_string(), "no graphics adapter is available");
        assert_eq!(
            OpenError::UnsafeDriver(vec!["Intel(R) UHD Graphics 630".into()]).to_string(),
            "the only graphics adapters that can draw (\"Intel(R) UHD Graphics 630\") use a driver that can crash; update the graphics driver"
        );
        assert_eq!(pick(&[], None, &Preference::Auto).unwrap_err(), OpenError::NoAdapter);
    }

    #[test]
    fn pick_says_when_only_a_driver_that_can_crash_can_draw() {
        let vulkan_only: Vec<Candidate> = crate::adapter::tests::uhd_630().into_iter().take(1).collect();
        assert_eq!(select(&vulkan_only, &Preference::Auto), None);
        assert_eq!(unselected(&vulkan_only), OpenError::UnsafeDriver(vec!["Intel(R) UHD Graphics 630".into()]));
        assert_eq!(unselected(&[]), OpenError::NoAdapter);
    }

    #[test]
    fn a_zero_timeout_leaves_a_healthy_device_healthy() {
        let opened = open(&noop(None), None, &Preference::Software, &Request::default()).unwrap();
        let everything = opened.queue.submit([]);
        assert!(!opened.health.wait_timeout(&opened.device, everything.clone(), std::time::Duration::ZERO));
        assert!(opened.health.is_ok(), "{:?}", opened.health.fault());
        assert!(opened.health.wait_timeout(&opened.device, everything, std::time::Duration::from_secs(5)));
        // an index from another device's queue is one this device never issued
        let other = open(&noop(None), None, &Preference::Software, &Request::default()).unwrap();
        other.queue.submit([]);
        let index = other.queue.submit([]);
        assert!(!opened.health.wait_timeout(&opened.device, index, std::time::Duration::from_secs(5)));
        assert!(opened.health.is_ok(), "{:?}", opened.health.fault());
    }

    #[test]
    fn a_lost_device_is_seen_by_its_health() {
        let opened = open(&noop(None), None, &Preference::Software, &Request::default()).unwrap();
        let health = opened.health.clone();
        opened.device.destroy();
        health.probe(&opened.device);
        assert!(health.fault().is_some_and(|f| f.is_lost()), "{health:?}");
        assert!(!health.is_ok());
        // a later wait does not touch the lost device
        assert!(!health.wait(&opened.device, None));
    }

    #[test]
    fn pick_returns_the_selected_adapter() {
        let instance = noop(None);
        let (adapters, candidates) = list(&instance, None);
        assert_eq!(candidates.len(), 1);
        let (adapter, selection) = pick(&adapters, None, &Preference::Auto).unwrap();
        assert_eq!(selection.index, 0);
        assert_eq!(Candidate::of(&adapter, None), candidates[0]);
    }
}
