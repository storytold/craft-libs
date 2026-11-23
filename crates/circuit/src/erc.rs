//! Connection checks: pins that fight, inputs nobody drives, loose ends, designators.
//!
//! Each [`Finding`] names its [`Rule`] by a stable id, its [`Level`], and the items it is
//! about, so an app can zoom to it and show its own translated text. [`Rule::summary`] is the
//! English text to use as the translation key.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::connect::{Connectivity, Net};
use crate::model::{ItemKind, ItemRef, LabelScope, PinKind, Point, Project};
use crate::natural::natural_cmp;

/// What a check looks for. The serialised name is the stable rule id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// Two pins on one net whose kinds should not meet (two outputs).
    PinConflict,
    /// Inputs on a net that nothing drives.
    UndrivenInput,
    /// Supply inputs on a net with no supply output and no supply label.
    UnpoweredInput,
    /// A pin that is the only pin on its net.
    UnconnectedPin,
    /// A pin marked "not connected" that is connected to another pin.
    NotConnectedPinConnected,
    /// A wire end that touches nothing.
    DanglingWire,
    /// A label that sits on no wire, pin or junction.
    FloatingLabel,
    /// Two parts with the same designator.
    DuplicateDesignator,
    /// A part with no designator yet, or one ending in `?`.
    MissingDesignator,
    /// A net that carries several label names.
    SeveralNetNames,
    /// Label names that differ only in upper and lower case.
    LabelCaseMismatch,
    /// A global label whose name appears only once, so it joins nothing.
    GlobalLabelUsedOnce,
}

impl Rule {
    /// Every rule, in report order.
    pub const ALL: [Rule; 12] = [
        Rule::PinConflict,
        Rule::UndrivenInput,
        Rule::UnpoweredInput,
        Rule::UnconnectedPin,
        Rule::NotConnectedPinConnected,
        Rule::DanglingWire,
        Rule::FloatingLabel,
        Rule::DuplicateDesignator,
        Rule::MissingDesignator,
        Rule::SeveralNetNames,
        Rule::LabelCaseMismatch,
        Rule::GlobalLabelUsedOnce,
    ];

    /// The level a rule has unless [`Settings::levels`] changes it.
    pub fn default_level(self) -> Level {
        match self {
            Rule::PinConflict | Rule::UnpoweredInput | Rule::NotConnectedPinConnected | Rule::DuplicateDesignator | Rule::MissingDesignator => {
                Level::Error
            }
            Rule::UndrivenInput
            | Rule::UnconnectedPin
            | Rule::DanglingWire
            | Rule::FloatingLabel
            | Rule::SeveralNetNames
            | Rule::LabelCaseMismatch
            | Rule::GlobalLabelUsedOnce => Level::Warning,
        }
    }

    /// One line in English, for use as a translation key.
    pub fn summary(self) -> &'static str {
        match self {
            Rule::PinConflict => "Pins that should not be connected are on the same net",
            Rule::UndrivenInput => "Input pins on a net that nothing drives",
            Rule::UnpoweredInput => "Supply pins on a net with no supply",
            Rule::UnconnectedPin => "Pin is not connected",
            Rule::NotConnectedPinConnected => "Pin marked as not connected is connected",
            Rule::DanglingWire => "Wire end is not connected",
            Rule::FloatingLabel => "Label is not on a wire or pin",
            Rule::DuplicateDesignator => "Several parts have the same designator",
            Rule::MissingDesignator => "Part is not numbered",
            Rule::SeveralNetNames => "Net has several names",
            Rule::LabelCaseMismatch => "Label names differ only in upper and lower case",
            Rule::GlobalLabelUsedOnce => "Global label is used only once",
        }
    }
}

/// How serious a finding is. `Off` turns a rule or a pin pair off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Off,
    Warning,
    Error,
}

/// A change to how serious it is when two pin kinds meet on a net.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinPair {
    pub a: PinKind,
    pub b: PinKind,
    pub level: Level,
}

/// What a project changes about the checks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Rule levels that differ from [`Rule::default_level`].
    pub levels: BTreeMap<Rule, Level>,
    /// Pin pairs that differ from [`pin_pair_level`]. The order of `a` and `b` does not matter.
    pub pin_pairs: Vec<PinPair>,
}

impl Settings {
    pub fn level(&self, rule: Rule) -> Level {
        self.levels.get(&rule).copied().unwrap_or_else(|| rule.default_level())
    }

    pub fn pair_level(&self, a: PinKind, b: PinKind) -> Level {
        self.pin_pairs.iter().rev().find(|p| (p.a == a && p.b == b) || (p.a == b && p.b == a)).map_or_else(|| pin_pair_level(a, b), |p| p.level)
    }
}

/// How serious it is, by default, when pins of kinds `a` and `b` share a net. Our own
/// policy: two sources that both force the line are an error; a three-state output against
/// a forcing source, or a pin whose kind is not known, is a warning. "Not connected" pins
/// have their own rule.
pub fn pin_pair_level(a: PinKind, b: PinKind) -> Level {
    use PinKind::{NotConnected, OpenCollector, Output, PowerOut, TriState, Unspecified};
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    match (a, b) {
        (_, NotConnected) | (NotConnected, _) => Level::Off,
        (Output, Output) | (Output, PowerOut) | (PowerOut, PowerOut) => Level::Error,
        (Output, OpenCollector) | (PowerOut, OpenCollector) => Level::Error,
        (Output, TriState) | (TriState, PowerOut) => Level::Warning,
        (Unspecified, _) | (_, Unspecified) => Level::Warning,
        _ => Level::Off,
    }
}

/// One problem found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub rule: Rule,
    pub level: Level,
    /// The net it is about, when it is about one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    /// The drawn items it is about, the first one being where to look.
    pub items: Vec<ItemRef>,
    /// The parts it is about, as indices into [`Project::parts`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<usize>,
    /// Sheet and position of the first item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<(usize, Point)>,
}

/// Runs every rule not turned off. Findings come errors first, then by rule, then by place.
pub fn check(project: &Project, conn: &Connectivity, settings: &Settings) -> Vec<Finding> {
    let mut out = Out { project, settings, findings: Vec::new() };
    for net in &conn.nets {
        net_rules(&mut out, net);
    }
    for end in &conn.dangling_ends {
        out.push(Rule::DanglingWire, None, vec![end.wire], Vec::new());
    }
    for &label in &conn.floating_labels {
        out.push(Rule::FloatingLabel, None, vec![label], Vec::new());
    }
    designator_rules(&mut out);
    label_rules(&mut out);
    let mut findings = out.findings;
    findings.sort_by(|a, b| b.level.cmp(&a.level).then_with(|| a.rule.cmp(&b.rule)).then_with(|| a.items.cmp(&b.items)));
    findings
}

struct Out<'a> {
    project: &'a Project,
    settings: &'a Settings,
    findings: Vec<Finding>,
}

impl Out<'_> {
    fn push(&mut self, rule: Rule, net: Option<&Net>, items: Vec<ItemRef>, parts: Vec<usize>) {
        let level = self.settings.level(rule);
        self.push_at(rule, level, net, items, parts);
    }

    fn push_at(&mut self, rule: Rule, level: Level, net: Option<&Net>, items: Vec<ItemRef>, parts: Vec<usize>) {
        if level == Level::Off {
            return;
        }
        let at = items.first().and_then(|&i| self.project.item_point(i).map(|p| (i.sheet, p)));
        self.findings.push(Finding { rule, level, net: net.map(|n| n.name.clone()), items, parts, at });
    }
}

/// A net's pins, one entry per distinct part and pin number.
fn distinct_pins(project: &Project, net: &Net) -> Vec<(ItemRef, PinKind)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for &item in &net.pins {
        if let Some(pin) = project.pin(item)
            && seen.insert((pin.part, pin.number.as_str()))
        {
            out.push((item, pin.kind));
        }
    }
    out
}

fn net_rules(out: &mut Out, net: &Net) {
    let pins = distinct_pins(out.project, net);
    pin_conflicts(out, net, &pins);
    let has = |k: PinKind| pins.iter().any(|p| p.1 == k);
    let of = |k: PinKind| pins.iter().filter(|p| p.1 == k).map(|p| p.0).collect::<Vec<_>>();
    let drives = |k: PinKind| !matches!(k, PinKind::Input | PinKind::PowerIn | PinKind::NotConnected);
    if has(PinKind::Input) && !net.has_supply && !pins.iter().any(|p| drives(p.1)) {
        out.push(Rule::UndrivenInput, Some(net), of(PinKind::Input), Vec::new());
    }
    if has(PinKind::PowerIn) && !net.has_supply && !has(PinKind::PowerOut) {
        out.push(Rule::UnpoweredInput, Some(net), of(PinKind::PowerIn), Vec::new());
    }
    if pins.len() == 1 && pins[0].1 != PinKind::NotConnected {
        out.push(Rule::UnconnectedPin, Some(net), vec![pins[0].0], Vec::new());
    }
    if pins.len() > 1 && has(PinKind::NotConnected) {
        out.push(Rule::NotConnectedPinConnected, Some(net), of(PinKind::NotConnected), Vec::new());
    }
    if net.label_names.len() > 1 {
        let labels = net.members.iter().copied().filter(|m| m.kind == ItemKind::Label).collect();
        out.push(Rule::SeveralNetNames, Some(net), labels, Vec::new());
    }
}

/// One finding per pair of pin kinds on a net whose meeting is not allowed.
fn pin_conflicts(out: &mut Out, net: &Net, pins: &[(ItemRef, PinKind)]) {
    let mut by_kind: BTreeMap<PinKind, Vec<ItemRef>> = BTreeMap::new();
    for &(item, kind) in pins {
        by_kind.entry(kind).or_default().push(item);
    }
    let kinds: Vec<PinKind> = by_kind.keys().copied().collect();
    for (i, &a) in kinds.iter().enumerate() {
        for &b in &kinds[i..] {
            if a == b && by_kind[&a].len() < 2 {
                continue;
            }
            let level = out.settings.pair_level(a, b).min(out.settings.level(Rule::PinConflict));
            let mut items = by_kind[&a].clone();
            if a != b {
                items.extend(&by_kind[&b]);
            }
            out.push_at(Rule::PinConflict, level, Some(net), items, Vec::new());
        }
    }
}

fn designator_rules(out: &mut Out) {
    let project = out.project;
    let first_pin = first_pins(project);
    let items_of = |part: usize| first_pin.get(&part).map(|&i| vec![i]).unwrap_or_default();
    let mut by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, part) in project.parts.iter().enumerate() {
        if part.is_numbered() {
            by_name.entry(part.designator.as_str()).or_default().push(i);
        } else {
            out.push(Rule::MissingDesignator, None, items_of(i), vec![i]);
        }
    }
    let mut dupes: Vec<(&str, Vec<usize>)> = by_name.into_iter().filter(|(_, v)| v.len() > 1).collect();
    dupes.sort_by(|a, b| natural_cmp(a.0, b.0));
    for (_, parts) in dupes {
        let items = parts.iter().flat_map(|&p| items_of(p)).collect();
        out.push(Rule::DuplicateDesignator, None, items, parts);
    }
}

/// The first placed pin of each part, to point a part finding at.
fn first_pins(project: &Project) -> HashMap<usize, ItemRef> {
    let mut out = HashMap::new();
    for (s, sheet) in project.sheets.iter().enumerate() {
        for (i, pin) in sheet.pins.iter().enumerate() {
            out.entry(pin.part).or_insert(ItemRef { sheet: s, kind: ItemKind::Pin, index: i });
        }
    }
    out
}

fn label_rules(out: &mut Out) {
    let mut by_lower: BTreeMap<String, BTreeMap<&str, Vec<ItemRef>>> = BTreeMap::new();
    let mut global: BTreeMap<&str, Vec<ItemRef>> = BTreeMap::new();
    for (s, sheet) in out.project.sheets.iter().enumerate() {
        for (i, label) in sheet.labels.iter().enumerate() {
            let item = ItemRef { sheet: s, kind: ItemKind::Label, index: i };
            by_lower.entry(label.name.to_lowercase()).or_default().entry(label.name.as_str()).or_default().push(item);
            if label.scope != LabelScope::Sheet {
                global.entry(label.name.as_str()).or_default().push(item);
            }
        }
    }
    for spellings in by_lower.values().filter(|v| v.len() > 1) {
        let items = spellings.values().map(|v| v[0]).collect();
        out.push(Rule::LabelCaseMismatch, None, items, Vec::new());
    }
    let project = out.project;
    for items in global.values().filter(|v| v.len() == 1) {
        let label = &project.sheets[items[0].sheet].labels[items[0].index];
        if label.scope == LabelScope::Global {
            out.push(Rule::GlobalLabelUsedOnce, None, items.clone(), Vec::new());
        }
    }
}
