//! [`DeviceView`]: the one snapshot a UI renders for a printer or scanner, built from
//! three-valued indicators ([`Tri`]) because many drivers report "not set" for "I do not know".
//!
//! Plain data, builders and small helpers. The domain crate maps its own status bits and printer
//! state reasons onto these (see the crate README).

use crate::id::DeviceId;
use crate::job::JobKind;

/// A device indicator that can be on, off, or **not reported**. A clear bit from some drivers
/// means "unknown", so a plain `bool` would invent information.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Tri {
    On,
    Off,
    #[default]
    #[cfg_attr(feature = "serde", serde(other))]
    NotReported,
}

impl Tri {
    /// `Some(true)` is on, `Some(false)` off, `None` not reported.
    pub const fn from_option(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::On,
            Some(false) => Self::Off,
            None => Self::NotReported,
        }
    }

    pub const fn from_bool(value: bool) -> Self {
        if value { Self::On } else { Self::Off }
    }

    pub const fn to_option(self) -> Option<bool> {
        match self {
            Self::On => Some(true),
            Self::Off => Some(false),
            Self::NotReported => None,
        }
    }

    pub const fn is_on(self) -> bool {
        matches!(self, Self::On)
    }

    pub const fn is_off(self) -> bool {
        matches!(self, Self::Off)
    }

    pub const fn is_reported(self) -> bool {
        !matches!(self, Self::NotReported)
    }
}

impl From<Option<bool>> for Tri {
    fn from(value: Option<bool>) -> Self {
        Self::from_option(value)
    }
}

/// How serious an indicator is, in increasing order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Severity {
    #[default]
    Ok,
    Info,
    Warning,
    Error,
}

/// Which indicator this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(into = "String"))]
pub enum IndicatorKey {
    /// Paper in the feeder (readiness: `On` is good).
    FeederPaper,
    /// The flatbed is ready (readiness).
    FlatbedReady,
    /// The duplexer is ready (readiness).
    DuplexReady,
    /// A cover is open.
    CoverOpen,
    /// A cover on the paper path is open.
    PathCoverOpen,
    Jam,
    /// Several sheets were fed together.
    MultiFeed,
    /// The scanner lamp reports an error. An error flag only: no on/off readback exists.
    LampError,
    /// The device asks for a person.
    Attention,
    Offline,
    PaperOut,
    SupplyLow,
    /// Anything else, named by the domain crate.
    Other(&'static str),
}

#[cfg(feature = "serde")]
impl IndicatorKey {
    /// The wire name: snake_case for the named keys, the text itself for `Other`.
    fn wire_name(self) -> &'static str {
        match self {
            Self::FeederPaper => "feeder_paper",
            Self::FlatbedReady => "flatbed_ready",
            Self::DuplexReady => "duplex_ready",
            Self::CoverOpen => "cover_open",
            Self::PathCoverOpen => "path_cover_open",
            Self::Jam => "jam",
            Self::MultiFeed => "multi_feed",
            Self::LampError => "lamp_error",
            Self::Attention => "attention",
            Self::Offline => "offline",
            Self::PaperOut => "paper_out",
            Self::SupplyLow => "supply_low",
            Self::Other(name) => name,
        }
    }
}

#[cfg(feature = "serde")]
impl From<IndicatorKey> for String {
    fn from(key: IndicatorKey) -> String {
        key.wire_name().to_string()
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for IndicatorKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::from)
    }
}

/// A name that is not one of the known keys becomes `Other` (interned and bounded: see the README).
#[cfg(feature = "serde")]
impl From<String> for IndicatorKey {
    fn from(text: String) -> Self {
        const NAMED: [IndicatorKey; 12] = [
            IndicatorKey::FeederPaper,
            IndicatorKey::FlatbedReady,
            IndicatorKey::DuplexReady,
            IndicatorKey::CoverOpen,
            IndicatorKey::PathCoverOpen,
            IndicatorKey::Jam,
            IndicatorKey::MultiFeed,
            IndicatorKey::LampError,
            IndicatorKey::Attention,
            IndicatorKey::Offline,
            IndicatorKey::PaperOut,
            IndicatorKey::SupplyLow,
        ];
        NAMED.into_iter().find(|key| key.wire_name() == text).unwrap_or_else(|| Self::Other(crate::wire::intern_or_other(&text)))
    }
}

impl IndicatorKey {
    /// Readiness keys are good when `On`; every other key is a fault, bad when `On`.
    pub const fn is_readiness(self) -> bool {
        matches!(self, Self::FeederPaper | Self::FlatbedReady | Self::DuplexReady)
    }

    /// A default label (English); the app localises its own.
    pub const fn default_label(self) -> &'static str {
        match self {
            Self::FeederPaper => "Paper in feeder",
            Self::FlatbedReady => "Flatbed ready",
            Self::DuplexReady => "Duplex ready",
            Self::CoverOpen => "Cover open",
            Self::PathCoverOpen => "Paper path cover open",
            Self::Jam => "Paper jam",
            Self::MultiFeed => "Multiple sheets fed",
            Self::LampError => "Lamp error",
            Self::Attention => "Needs attention",
            Self::Offline => "Offline",
            Self::PaperOut => "Out of paper",
            Self::SupplyLow => "Supply low",
            Self::Other(name) => name,
        }
    }

    /// The severity of this indicator in `state`. A state that is not reported is never a problem.
    pub const fn default_severity(self, state: Tri) -> Severity {
        match (self.is_readiness(), state) {
            (_, Tri::NotReported) => Severity::Ok,
            (true, Tri::On) => Severity::Ok,
            (true, Tri::Off) => Severity::Info,
            (false, Tri::Off) => Severity::Ok,
            (false, Tri::On) => match self {
                Self::Jam | Self::MultiFeed | Self::LampError | Self::Offline => Severity::Error,
                Self::CoverOpen | Self::PathCoverOpen | Self::Attention | Self::PaperOut | Self::SupplyLow => Severity::Warning,
                _ => Severity::Info,
            },
        }
    }
}

/// One thing a device says about itself.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Indicator {
    pub key: IndicatorKey,
    pub label: String,
    pub state: Tri,
    pub severity: Severity,
    #[cfg_attr(feature = "serde", serde(default))]
    pub detail: Option<String>,
}

impl Indicator {
    /// With the key's default label and the default severity for `state`.
    pub fn new(key: IndicatorKey, state: Tri) -> Self {
        Self { key, label: key.default_label().to_string(), state, severity: key.default_severity(state), detail: None }
    }

    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    #[must_use]
    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// A warning or an error.
    pub fn is_problem(&self) -> bool {
        self.severity >= Severity::Warning
    }
}

/// The overall state of a device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum DeviceState {
    Online,
    Busy,
    Offline,
    Attention,
    #[default]
    #[cfg_attr(feature = "serde", serde(other))]
    Unknown,
}

/// How a device is attached. Named apart from craft-scan's `Connection`; the mapping is in the README.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum DeviceConnection {
    Usb,
    Network,
    /// A local port or a virtual device.
    Local,
    Other,
    #[default]
    #[cfg_attr(feature = "serde", serde(other))]
    Unknown,
}

/// What the device can do, where it says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct CapabilitiesSummary {
    pub has_feeder: Tri,
    pub has_flatbed: Tri,
    pub duplex: Tri,
    /// Option names the device applies (the rest are advisory).
    pub honoured: Vec<String>,
}

impl CapabilitiesSummary {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_feeder(mut self, v: Tri) -> Self {
        self.has_feeder = v;
        self
    }

    #[must_use]
    pub fn with_flatbed(mut self, v: Tri) -> Self {
        self.has_flatbed = v;
        self
    }

    #[must_use]
    pub fn with_duplex(mut self, v: Tri) -> Self {
        self.duplex = v;
        self
    }

    #[must_use]
    pub fn with_honoured(mut self, option: impl Into<String>) -> Self {
        self.honoured.push(option.into());
        self
    }
}

/// Where paper comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum PaperSource {
    Feeder,
    Glass,
    /// A named tray.
    Tray(String),
}

/// The state of the paper path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct PaperPathView {
    pub feeder_loaded: Tri,
    pub feeder_capacity: Option<u32>,
    pub glass_available: Tri,
    pub source_in_use: Option<PaperSource>,
}

impl PaperPathView {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_feeder_loaded(mut self, v: Tri) -> Self {
        self.feeder_loaded = v;
        self
    }

    #[must_use]
    pub fn with_feeder_capacity(mut self, sheets: u32) -> Self {
        self.feeder_capacity = Some(sheets);
        self
    }

    #[must_use]
    pub fn with_glass_available(mut self, v: Tri) -> Self {
        self.glass_available = v;
        self
    }

    #[must_use]
    pub fn with_source_in_use(mut self, source: PaperSource) -> Self {
        self.source_in_use = Some(source);
        self
    }
}

/// A consumable (toner, ink), print only in practice.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Supply {
    pub label: String,
    /// 0 to 100, when the device reports a level.
    #[cfg_attr(feature = "serde", serde(default))]
    pub level_percent: Option<u8>,
    /// `On`: available; `Off`: low or empty; `NotReported`: unknown.
    pub state: Tri,
}

impl Supply {
    pub fn new(label: impl Into<String>, state: Tri) -> Self {
        Self { label: label.into(), level_percent: None, state }
    }

    /// Sets the level, clamped to 100.
    #[must_use]
    pub fn with_level_percent(mut self, percent: u8) -> Self {
        self.level_percent = Some(percent.min(100));
        self
    }
}

/// A snapshot of one device for display.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DeviceView {
    pub id: DeviceId,
    pub name: String,
    pub kind: JobKind,
    #[cfg_attr(feature = "serde", serde(default))]
    pub connection: DeviceConnection,
    #[cfg_attr(feature = "serde", serde(default))]
    pub state: DeviceState,
    #[cfg_attr(feature = "serde", serde(default))]
    pub capabilities: CapabilitiesSummary,
    #[cfg_attr(feature = "serde", serde(default))]
    pub indicators: Vec<Indicator>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub paper: Option<PaperPathView>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub supplies: Vec<Supply>,
}

impl DeviceView {
    pub fn new(id: DeviceId, name: impl Into<String>, kind: JobKind) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
            connection: DeviceConnection::Unknown,
            state: DeviceState::Unknown,
            capabilities: CapabilitiesSummary::default(),
            indicators: Vec::new(),
            paper: None,
            supplies: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_connection(mut self, connection: DeviceConnection) -> Self {
        self.connection = connection;
        self
    }

    #[must_use]
    pub fn with_state(mut self, state: DeviceState) -> Self {
        self.state = state;
        self
    }

    #[must_use]
    pub fn with_capabilities(mut self, capabilities: CapabilitiesSummary) -> Self {
        self.capabilities = capabilities;
        self
    }

    #[must_use]
    pub fn with_indicator(mut self, indicator: Indicator) -> Self {
        self.indicators.push(indicator);
        self
    }

    #[must_use]
    pub fn with_paper(mut self, paper: PaperPathView) -> Self {
        self.paper = Some(paper);
        self
    }

    #[must_use]
    pub fn with_supply(mut self, supply: Supply) -> Self {
        self.supplies.push(supply);
        self
    }

    /// The indicators that are warnings or errors.
    pub fn problems(&self) -> impl Iterator<Item = &Indicator> {
        self.indicators.iter().filter(|i| i.is_problem())
    }

    /// The most serious severity among the indicators ([`Severity::Ok`] when there are none).
    pub fn worst_severity(&self) -> Severity {
        self.indicators.iter().map(|i| i.severity).max().unwrap_or(Severity::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use IndicatorKey::*;

    #[test]
    fn tri_conversions() {
        assert_eq!(Tri::from_option(Some(true)), Tri::On);
        assert_eq!(Tri::from_option(Some(false)), Tri::Off);
        assert_eq!(Tri::from_option(None), Tri::NotReported);
        assert_eq!(Tri::from(Some(true)), Tri::On);
        assert_eq!(Tri::from_bool(false), Tri::Off);
        assert_eq!(Tri::default(), Tri::NotReported);
        for t in [Tri::On, Tri::Off, Tri::NotReported] {
            assert_eq!(Tri::from_option(t.to_option()), t);
        }
        assert!(Tri::On.is_on() && Tri::Off.is_off() && !Tri::NotReported.is_on() && !Tri::NotReported.is_off());
        assert!(Tri::Off.is_reported() && !Tri::NotReported.is_reported());
    }

    #[test]
    fn default_severities() {
        let sev = |k: IndicatorKey, s| Indicator::new(k, s).severity;
        assert_eq!(sev(Jam, Tri::On), Severity::Error);
        assert_eq!(sev(MultiFeed, Tri::On), Severity::Error);
        assert_eq!(sev(LampError, Tri::On), Severity::Error);
        assert_eq!(sev(Offline, Tri::On), Severity::Error);
        assert_eq!(sev(CoverOpen, Tri::On), Severity::Warning);
        assert_eq!(sev(PathCoverOpen, Tri::On), Severity::Warning);
        assert_eq!(sev(Attention, Tri::On), Severity::Warning);
        assert_eq!(sev(PaperOut, Tri::On), Severity::Warning);
        assert_eq!(sev(SupplyLow, Tri::On), Severity::Warning);
        assert_eq!(sev(FeederPaper, Tri::Off), Severity::Info);
        assert_eq!(sev(FlatbedReady, Tri::Off), Severity::Info);
        assert_eq!(sev(DuplexReady, Tri::Off), Severity::Info);
        assert_eq!(sev(FeederPaper, Tri::On), Severity::Ok);
        assert_eq!(sev(Other("x"), Tri::On), Severity::Info);
        for k in [Jam, CoverOpen, FeederPaper, FlatbedReady, Offline, Other("x")] {
            assert_eq!(sev(k, Tri::NotReported), Severity::Ok, "{k:?}: unknown is not a problem");
            if !k.is_readiness() {
                assert_eq!(sev(k, Tri::Off), Severity::Ok, "{k:?}: a clear fault is fine");
            }
        }
    }

    #[test]
    fn indicator_defaults_and_overrides() {
        let i = Indicator::new(Jam, Tri::On);
        assert_eq!((i.label.as_str(), i.detail.as_deref()), ("Paper jam", None));
        assert!(i.is_problem());
        assert_eq!(Indicator::new(Other("Door"), Tri::On).label, "Door");
        let i = Indicator::new(CoverOpen, Tri::On).with_label("Lid").with_severity(Severity::Error).with_detail("ADF lid");
        assert_eq!((i.label.as_str(), i.severity, i.detail.as_deref()), ("Lid", Severity::Error, Some("ADF lid")));
        assert!(!Indicator::new(FeederPaper, Tri::Off).is_problem(), "an empty feeder is information");
        assert!(!Indicator::new(Jam, Tri::NotReported).is_problem());
        assert!(!Indicator::new(Jam, Tri::Off).is_problem());
        assert!(Severity::Error > Severity::Warning && Severity::Warning > Severity::Info && Severity::Info > Severity::Ok);
    }

    #[test]
    fn a_device_view_collects_problems() {
        let view = DeviceView::new(DeviceId::new("wia", "dev0"), "Desk scanner", JobKind::Scan)
            .with_connection(DeviceConnection::Usb)
            .with_state(DeviceState::Attention)
            .with_capabilities(
                CapabilitiesSummary::new().with_feeder(Tri::On).with_flatbed(Tri::from_option(None)).with_duplex(Tri::Off).with_honoured("dpi"),
            )
            .with_indicator(Indicator::new(FeederPaper, Tri::Off))
            .with_indicator(Indicator::new(CoverOpen, Tri::On))
            .with_indicator(Indicator::new(Jam, Tri::NotReported))
            .with_indicator(Indicator::new(LampError, Tri::On))
            .with_paper(PaperPathView::new().with_feeder_loaded(Tri::Off).with_feeder_capacity(50).with_source_in_use(PaperSource::Glass))
            .with_supply(Supply::new("Toner", Tri::On).with_level_percent(250));
        let keys: Vec<_> = view.problems().map(|i| i.key).collect();
        assert_eq!(keys, [CoverOpen, LampError]);
        assert_eq!(view.worst_severity(), Severity::Error);
        assert_eq!(view.capabilities.has_flatbed, Tri::NotReported);
        assert_eq!(view.capabilities.honoured, ["dpi"]);
        assert_eq!(view.paper.as_ref().and_then(|p| p.feeder_capacity), Some(50));
        assert_eq!(view.supplies.first().and_then(|s| s.level_percent), Some(100), "clamped");
        assert_eq!(view.connection, DeviceConnection::Usb);
        assert_eq!(view.clone(), view);
    }

    #[test]
    fn an_empty_view_has_no_problems() {
        let v = DeviceView::new(DeviceId::new("a", "b"), "x", JobKind::Print);
        assert_eq!(v.problems().count(), 0);
        assert_eq!(v.worst_severity(), Severity::Ok);
        assert_eq!((v.state, v.connection), (DeviceState::Unknown, DeviceConnection::Unknown));
        assert!(v.paper.is_none() && v.supplies.is_empty());
        assert_eq!(PaperSource::Tray("A4".into()), PaperSource::Tray("A4".into()));
        assert_eq!(Supply::new("Ink", Tri::Off).level_percent, None);
    }
}
