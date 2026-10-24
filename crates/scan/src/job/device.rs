//! A scanner's paper-path status as a [`craft_job::DeviceView`].
//!
//! The ready bits `flat_ready` and `duplex_ready` are read by connection (`feeder_ready` is taken as the backend gave it).
//! A USB scanner's driver publishes them, so a clear bit is `Off`. A network scanner (`Wsd`, `Other`) or one of unknown
//! connection often publishes no usable tray or ready state (the inbox WSD scan driver reports every bit
//! clear), so there a clear ready bit is `NotReported`; a set bit is a positive reading, `On`. The fault bits
//! are not ready bits: they are read as given once the status says anything at all.

use craft_job::{
    CapabilitiesSummary, DeviceConnection, DeviceId, DeviceState, DeviceView, Indicator, IndicatorKey, JobKind, PaperPathView, PaperSource, Tri,
};

use crate::{Connection, HandlingStatus, Presence, Scanner, Source};

/// Present is `On`, Absent is `Off` (the driver answered without it) and Unknown stays `NotReported`.
fn tri(presence: Presence) -> Tri {
    match presence {
        Presence::Present => Tri::On,
        Presence::Absent => Tri::Off,
        Presence::Unknown => Tri::NotReported,
    }
}

/// `Usb` is `Usb`, `Wsd` is `Network`, and `Other` and `Unknown` stay themselves. (`Other` is how an eSCL
/// scanner is listed: it is on the network, but the registry only says "not USB, not WSD".)
impl From<Connection> for DeviceConnection {
    fn from(connection: Connection) -> Self {
        match connection {
            Connection::Usb => Self::Usb,
            Connection::Wsd => Self::Network,
            Connection::Other => Self::Other,
            Connection::Unknown => Self::Unknown,
        }
    }
}

/// One scanner as a [`DeviceView`]: capabilities, the paper path and an indicator for each status bit.
///
/// `status` is what [`crate::status`] answered (`None`: the backend cannot preflight), or
/// `Some(scanner.handling_status)` for the reading taken with the details. Every indicator is a [`Tri`]:
///
/// * `feeder_ready` is an `Option<bool>` and is taken as the backend gave it: `None` is `NotReported`.
/// * The ready bits `flat_ready` and `duplex_ready` are read by the scanner's connection. On USB a clear bit is `Off`. On a
///   network (`Wsd`, `Other`) or `Unknown` connection a clear ready bit is `NotReported` (those drivers publish no usable
///   ready state); a set bit is `On` on any connection. `feeder_ready` does not depend on the connection: an explicit
///   `Some(false)` is a reading and stays `Off`.
/// * The other bits are plain `bool`s that mean "set only when reported". A status that is entirely clear and
///   leaves the feeder unreported ([`HandlingStatus::default`]) is what a driver that reports nothing looks
///   like (the inbox WSD scan driver reports every bit clear), so **every** indicator is `NotReported`, never
///   `Off`. Once any bit says something, the others are read as given: a clear fault bit is `Off`.
/// * The capabilities (`feeder`, `flatbed`, `duplex`) are three-state through [`Scanner::feeder_presence`] and its
///   siblings: `Off` only when the backend read the handling bits and the unit was not in them
///   ([`crate::Reported::units`]); a plain `false` from a backend that did not say is `NotReported`.
/// * An indicator for hardware the scanner does not have (no feeder, no flatbed, no duplexer) is left out
///   unless the status asserts it.
///
/// `source_in_use` fills [`PaperPathView::source_in_use`] (`Auto` says nothing).
pub fn device_view(scanner: &Scanner, status: Option<HandlingStatus>, source_in_use: Option<Source>) -> DeviceView {
    let id = scanner.id.parse::<DeviceId>().unwrap_or_else(|_| DeviceId::new("unknown", scanner.id.clone()));
    let reported = status.filter(|s| *s != HandlingStatus::default());
    let flag = |bit: fn(&HandlingStatus) -> bool| reported.map_or(Tri::NotReported, |s| Tri::from_bool(bit(&s)));
    let feeder = reported.map_or(Tri::NotReported, |s| Tri::from_option(s.feeder_ready));
    // A clear ready bit is only "off" where the driver is known to publish readiness: USB.
    let ready = |bit: fn(&HandlingStatus) -> bool| match reported {
        Some(s) if bit(&s) => Tri::On,
        Some(_) if scanner.connection == Connection::Usb => Tri::Off,
        _ => Tri::NotReported,
    };
    let flat = ready(|s| s.flat_ready);
    let duplex = ready(|s| s.duplex_ready);

    let capabilities = CapabilitiesSummary::new()
        .with_feeder(tri(scanner.feeder_presence()))
        .with_flatbed(tri(scanner.flatbed_presence()))
        .with_duplex(tri(scanner.duplex_presence()));

    let mut paper =
        PaperPathView::new().with_feeder_loaded(feeder).with_glass_available(if scanner.flatbed { flat } else { tri(scanner.flatbed_presence()) });
    if let Some(capacity) = scanner.feeder_capacity {
        paper = paper.with_feeder_capacity(capacity);
    }
    match source_in_use {
        Some(Source::Feeder) => paper = paper.with_source_in_use(PaperSource::Feeder),
        Some(Source::Glass) => paper = paper.with_source_in_use(PaperSource::Glass),
        Some(Source::Auto) | None => {}
    }

    let mut view = DeviceView::new(id, scanner.name.clone(), JobKind::Scan)
        .with_connection(scanner.connection.into())
        .with_capabilities(capabilities)
        .with_paper(paper);

    // Readiness indicators only for hardware that exists (or that the status says is there).
    if scanner.feeder || feeder.is_on() {
        view = view.with_indicator(Indicator::new(IndicatorKey::FeederPaper, feeder));
    }
    if scanner.flatbed || flat.is_on() {
        view = view.with_indicator(Indicator::new(IndicatorKey::FlatbedReady, flat));
    }
    if scanner.duplex || duplex.is_on() {
        view = view.with_indicator(Indicator::new(IndicatorKey::DuplexReady, duplex));
    }
    for (key, state) in [
        (IndicatorKey::CoverOpen, flag(|s| s.cover_open)),
        (IndicatorKey::PathCoverOpen, flag(|s| s.path_cover_open)),
        (IndicatorKey::Jam, flag(|s| s.jam)),
        (IndicatorKey::MultiFeed, flag(|s| s.multiple_feed)),
        (IndicatorKey::Attention, flag(|s| s.attention)),
        // The lamp bit is an error flag: there is no readback, so a clear bit is still only "no error reported".
        (IndicatorKey::LampError, flag(|s| s.lamp_error)),
    ] {
        view = view.with_indicator(Indicator::new(key, state));
    }

    let state = if view.problems().any(|i| i.state.is_on()) {
        DeviceState::Attention
    } else if reported.is_some() {
        DeviceState::Online
    } else {
        DeviceState::Unknown
    };
    view.with_state(state)
}
