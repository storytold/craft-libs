//! The XInput2 reader: a second X connection, raw events on the root window, one thread.
//!
//! Adapted from PhotoCraft's `photocraft-tablet` (`crates/tablet/src/x11.rs`, storytold/photocraft
//! at `b609bc7`, MIT OR Apache-2.0), by way of TextureCraft (storytold/texturecraft#818). Unlike it,
//! events are keyed by `sourceid`, the physical device, as storytold/photocraft#6871 found Xwayland
//! needs.

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xinput::{self, ConnectionExt as _, DeviceClassData, DeviceType, XIEventMask};
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use crate::Error;
use crate::xi::{self, Device, State, Update, fp3232, valuators};

/// `XIAllDevices`: slaves included.
const ALL_DEVICES: u16 = 0;

pub(crate) fn spawn(
    thread_name: &str,
    mut on_update: impl FnMut(Option<f32>) + Send + 'static,
    on_closed: impl FnOnce() + Send + 'static,
) -> Result<(), Error> {
    let unavailable = Error::Unavailable;
    let (conn, screen) = RustConnection::connect(None).map_err(|e| unavailable(format!("cannot connect to the X server: {e}")))?;
    let root = conn.setup().roots.get(screen).map(|s| s.root).ok_or_else(|| unavailable(format!("X screen {screen} not found")))?;
    // Raw events for every client (not just the grabbing one) need XI 2.1
    let version =
        conn.xinput_xi_query_version(2, 2).map_err(|e| unavailable(e.to_string()))?.reply().map_err(|e| unavailable(format!("no XInput2: {e}")))?;
    if (version.major_version, version.minor_version) < (2, 1) {
        return Err(unavailable(format!("XInput {}.{} (raw events need 2.1)", version.major_version, version.minor_version)));
    }
    let mask = XIEventMask::RAW_MOTION
        | XIEventMask::RAW_BUTTON_PRESS
        | XIEventMask::RAW_BUTTON_RELEASE
        | XIEventMask::HIERARCHY
        | XIEventMask::DEVICE_CHANGED;
    conn.xinput_xi_select_events(root, &[xinput::EventMask { deviceid: ALL_DEVICES, mask: vec![mask] }])
        .map_err(|e| unavailable(e.to_string()))?
        .check()
        .map_err(|e| unavailable(format!("XISelectEvents: {e}")))?;
    let mut state = State::default();
    state.set_devices(query_devices(&conn));
    std::thread::Builder::new()
        .name(thread_name.into())
        .spawn(move || {
            run(&conn, &mut state, &mut on_update);
            on_closed();
        })
        .map(drop)
        .map_err(|e| unavailable(format!("cannot start the tablet thread: {e}")))
}

/// Ends when the X connection closes.
fn run(conn: &RustConnection, state: &mut State, on_update: &mut dyn FnMut(Option<f32>)) {
    while let Ok(event) = conn.wait_for_event() {
        let update = match event {
            Event::XinputRawMotion(e) | Event::XinputRawButtonPress(e) | Event::XinputRawButtonRelease(e) => {
                let values: Vec<f64> = e.axisvalues.iter().map(|v| fp3232(v.integral, v.frac)).collect();
                state.handle(e.sourceid, valuators(&e.valuator_mask, &values))
            }
            Event::XinputHierarchy(_) | Event::XinputDeviceChanged(_) => {
                state.set_devices(query_devices(conn));
                Update::Keep
            }
            _ => Update::Keep,
        };
        if let Update::Set(pressure) = update {
            on_update(pressure);
        }
    }
}

/// The slave pointer devices and their valuators (empty on any error).
fn query_devices(conn: &RustConnection) -> Vec<Device> {
    let Ok(reply) = conn.xinput_xi_query_device(ALL_DEVICES).map_err(drop).and_then(|c| c.reply().map_err(drop)) else { return Vec::new() };
    let mut out = Vec::new();
    for info in reply.infos.iter().take(xi::MAX_DEVICES) {
        if info.type_ != DeviceType::SLAVE_POINTER && info.type_ != DeviceType::FLOATING_SLAVE {
            continue;
        }
        let valuators = info.classes.iter().take(xi::MAX_AXES).filter_map(|class| match &class.data {
            DeviceClassData::Valuator(v) => {
                Some((v.number, atom_name(conn, v.label)?, fp3232(v.min.integral, v.min.frac), fp3232(v.max.integral, v.max.frac)))
            }
            _ => None,
        });
        out.push(Device::new(info.deviceid, valuators.collect::<Vec<_>>()));
    }
    out
}

fn atom_name(conn: &RustConnection, atom: u32) -> Option<String> {
    if atom == 0 {
        return None;
    }
    let reply = conn.get_atom_name(atom).ok()?.reply().ok()?;
    Some(String::from_utf8_lossy(&reply.name).into_owned())
}
