//! XInput2 tablet valuators → pen pressure. Pure (no X calls), so it builds and tests on every
//! platform; the Linux reader queries the devices and feeds raw events here.
//!
//! Adapted from PhotoCraft's `photocraft-tablet` (`crates/tablet/src/xi.rs`, storytold/photocraft
//! at `b609bc7`, MIT OR Apache-2.0), reduced to pressure, by way of TextureCraft
//! (storytold/texturecraft#818).
//!
//! A tablet tool is a slave pointer device with an "Abs Pressure" valuator (xf86-input-wacom,
//! xf86-input-libinput and Xwayland all label their axes with the standard atom names), normalised
//! from its advertised `min..max`. Raw events only carry the valuators that changed, so the last
//! pressure is kept per device.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Valuator numbers beyond this are ignored (the protocol allows 65535; real devices have < 16).
pub const MAX_AXES: usize = 64;

/// Devices kept (a hostile server could report thousands).
pub const MAX_DEVICES: usize = 256;

const PRESSURE_LABEL: &str = "Abs Pressure";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    /// Index into a raw event's valuator mask.
    pub number: u16,
    pub min: f64,
    pub max: f64,
}

impl Axis {
    fn map(&self, v: f64) -> f32 {
        if !v.is_finite() {
            return 1.0;
        }
        let span = self.max - self.min;
        let unit = if span.is_finite() && span > 0.0 { (v - self.min) / span } else { v };
        unit.clamp(0.0, 1.0) as f32
    }
}

/// A slave pointer device; `pressure` is `None` for a mouse.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: u16,
    pub pressure: Option<Axis>,
}

impl Device {
    /// From what `XIQueryDevice` reports: each valuator's (number, label, min, max).
    pub fn new(id: u16, valuators: impl IntoIterator<Item = (u16, String, f64, f64)>) -> Self {
        let pressure = valuators
            .into_iter()
            .take(MAX_AXES)
            .find(|(number, label, ..)| usize::from(*number) < MAX_AXES && label == PRESSURE_LABEL)
            .map(|(number, _, min, max)| Axis { number, min, max });
        Self { id, pressure }
    }
}

/// What a raw event means for the pen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Update {
    /// An event from a device this module doesn't know.
    Keep,
    /// `Some(pressure)` for a pen, `None` for a mouse.
    Set(Option<f32>),
}

/// The values a raw event carries: valuator numbers set in `mask` (32 per word, LSB first)
/// paired, in order, with `values`. Extra mask bits beyond the values are ignored.
pub fn valuators<'a>(mask: &'a [u32], values: &'a [f64]) -> impl Iterator<Item = (u16, f64)> + 'a {
    let numbers = mask
        .iter()
        .enumerate()
        .flat_map(|(w, bits)| (0..32u32).filter(move |b| bits & (1 << b) != 0).map(move |b| w.saturating_mul(32).saturating_add(b as usize)));
    numbers.zip(values.iter().copied()).filter_map(|(n, v)| u16::try_from(n).ok().map(|n| (n, v)))
}

#[derive(Debug, Default)]
pub struct State {
    devices: HashMap<u16, Device>,
    last: HashMap<u16, f32>,
}

impl State {
    /// Replaces the device list (on start and on hierarchy or device changes).
    pub fn set_devices(&mut self, devices: impl IntoIterator<Item = Device>) {
        let devices: HashMap<_, _> = devices.into_iter().take(MAX_DEVICES).map(|d| (d.id, d)).collect();
        // A changed range makes the cached, already mapped pressure stale
        self.last.retain(|id, _| matches!((self.devices.get(id), devices.get(id)), (Some(old), Some(new)) if old == new));
        self.devices = devices;
    }

    /// A raw motion or button event from slave device `id` with its changed valuators.
    pub fn handle(&mut self, id: u16, valuators: impl IntoIterator<Item = (u16, f64)>) -> Update {
        let Some(device) = self.devices.get(&id) else { return Update::Keep };
        let Some(axis) = device.pressure else { return Update::Set(None) };
        let last = self.last.entry(id).or_insert(0.0);
        for (number, value) in valuators {
            if number == axis.number {
                *last = axis.map(value);
            }
        }
        Update::Set(Some(*last))
    }
}

/// Shortest wait between device queries while a refresh keeps failing.
pub const RETRY_EVERY: Duration = Duration::from_secs(1);

/// Device rediscovery that keeps the known devices when a query fails, and tries again later.
#[derive(Debug, Default)]
pub struct Rediscovery {
    pending: bool,
    last_try: Option<Instant>,
}

impl Rediscovery {
    /// The device hierarchy or a device changed: query again at the next chance, or, while a
    /// refresh is already failing, at its next retry.
    pub fn request(&mut self) {
        if !self.pending {
            self.last_try = None;
        }
        self.pending = true;
    }

    /// Whether to query the devices now.
    pub fn due(&self, now: Instant) -> bool {
        self.pending && self.last_try.is_none_or(|t| now.saturating_duration_since(t) >= RETRY_EVERY)
    }

    /// Installs a query's devices. A failed query leaves `state` as it was and the refresh pending.
    pub fn apply<E>(&mut self, state: &mut State, now: Instant, result: Result<Vec<Device>, E>) {
        self.last_try = Some(now);
        if let Ok(devices) = result {
            state.set_devices(devices);
            self.pending = false;
        }
    }
}

/// A 32.32 fixed-point X value.
pub fn fp3232(integral: i32, frac: u32) -> f64 {
    f64::from(integral) + f64::from(frac) / 4_294_967_296.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wacom(id: u16, max: f64) -> Device {
        Device::new(
            id,
            [
                (0, "Abs X".to_string(), 0.0, 44704.0),
                (1, "Abs Y".to_string(), 0.0, 27940.0),
                (2, PRESSURE_LABEL.to_string(), 0.0, max),
                (3, "Abs Tilt X".to_string(), -64.0, 63.0),
            ],
        )
    }

    fn mouse(id: u16) -> Device {
        Device::new(id, [(0, "Rel X".to_string(), -1.0, -1.0), (1, "Rel Y".to_string(), -1.0, -1.0)])
    }

    fn pressure(update: Update) -> f32 {
        match update {
            Update::Set(Some(p)) => p,
            other => panic!("expected a pen, got {other:?}"),
        }
    }

    #[test]
    fn mask_pairs_numbers_with_values() {
        let v: Vec<_> = valuators(&[0b10_0101, 1], &[1.0, 2.0, 3.0, 4.0]).collect();
        assert_eq!(v, vec![(0, 1.0), (2, 2.0), (5, 3.0), (32, 4.0)]);
        assert_eq!(valuators(&[u32::MAX; 4], &[7.0]).count(), 1, "fewer values than mask bits stop at the values");
    }

    #[test]
    fn pens_report_pressure_and_other_pointers_mean_a_mouse() {
        let mut st = State::default();
        st.set_devices([wacom(9, 65535.0), mouse(12)]);
        let light = pressure(st.handle(9, [(0, 100.0), (2, 16000.0)]));
        let firm = pressure(st.handle(9, [(2, 60000.0)]));
        assert!(0.0 < light && light < firm && firm <= 1.0);
        assert_eq!(pressure(st.handle(9, [(0, 120.0)])), firm, "an event without the pressure axis keeps the last pressure");
        assert_eq!(st.handle(12, [(0, 3.0)]), Update::Set(None));
        assert_eq!(st.handle(2, [(2, 3.0)]), Update::Keep, "an unknown device changes nothing");
    }

    #[test]
    fn a_changed_pressure_range_drops_the_cached_pressure() {
        let mut st = State::default();
        st.set_devices([wacom(9, 1023.0), wacom(10, 1023.0)]);
        let _ = st.handle(9, [(2, 1023.0)]);
        let kept = pressure(st.handle(10, [(2, 1023.0)]));
        st.set_devices([wacom(9, 2047.0), wacom(10, 1023.0)]);
        assert!(pressure(st.handle(9, [])) < kept);
        assert_eq!(pressure(st.handle(10, [])), kept);
    }

    #[test]
    fn hostile_device_data_does_not_panic() {
        let mut st = State::default();
        st.set_devices([Device::new(1, [(2, PRESSURE_LABEL.into(), 5.0, 5.0)]), Device::new(2, [(u16::MAX, PRESSURE_LABEL.into(), 0.0, 1.0)])]);
        assert_eq!(st.handle(2, [(u16::MAX, 0.5)]), Update::Set(None), "an out-of-range valuator number is not a pressure axis");
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, f64::MIN, -0.0, 1e300] {
            assert!((0.0..=1.0).contains(&pressure(st.handle(1, [(2, v)]))), "{v}");
        }
        st.set_devices((0..2000u16).map(|i| Device::new(i, [])));
        assert!(st.devices.len() <= MAX_DEVICES);
    }

    #[test]
    fn fixed_point() {
        assert_eq!(fp3232(3, 1 << 31), 3.5);
        assert_eq!(fp3232(-2, 0), -2.0);
    }

    #[test]
    fn a_failed_refresh_keeps_the_known_pens_and_retries() {
        let mut state = State::default();
        state.set_devices([wacom(10, 2047.0)]);
        let (mut rediscovery, start) = (Rediscovery::default(), Instant::now());

        rediscovery.request();
        assert!(rediscovery.due(start));
        rediscovery.apply(&mut state, start, Err("BadAlloc"));
        assert!(pressure(state.handle(10, [(2, 1023.5)])) > 0.4, "the pen still reports pressure");
        assert!(!rediscovery.due(start + RETRY_EVERY / 2), "no query on every event while it fails");
        rediscovery.request();
        assert!(!rediscovery.due(start + RETRY_EVERY / 2), "nor on every further device change");
        assert!(rediscovery.due(start + RETRY_EVERY), "but another one later");

        // A successful empty reply does remove the pen
        rediscovery.apply(&mut state, start + RETRY_EVERY, Ok::<_, &str>(Vec::new()));
        assert!(!rediscovery.due(start + RETRY_EVERY * 2));
        assert!(matches!(state.handle(10, [(2, 1023.5)]), Update::Keep));
    }
}
