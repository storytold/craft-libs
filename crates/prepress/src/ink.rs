//! The ink model: which inks a job prints with, and colors as tints of those inks.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use craft_cms::math;
use craft_pdf::interpret::color::CmykConversion;
use craft_pdf::write::backend::color::device_n::{self, Colorant, DeviceNSpace};
use craft_pdf::write::backend::color::separation::{self, SeparationColorant, SeparationSpace};
use craft_pdf::write::backend::color::{Color as PdfColor, RegularColor, cmyk};

use crate::cms::lab_to_srgb8;

/// A process ink, in plate order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Process {
    Cyan,
    Magenta,
    Yellow,
    Black,
}

impl Process {
    /// The four process inks, in plate order.
    pub const ALL: [Process; 4] = [Process::Cyan, Process::Magenta, Process::Yellow, Process::Black];

    /// The colorant name a `Separation` or `DeviceN` color space gives this ink.
    pub fn name(self) -> &'static str {
        match self {
            Self::Cyan => "Cyan",
            Self::Magenta => "Magenta",
            Self::Yellow => "Yellow",
            Self::Black => "Black",
        }
    }

    /// The process ink a colorant name denotes (ISO 32000-2 §8.6.6.4).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// The ink's position in a CMYK color.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// How a spot ink looks, for previews and for printing it as process color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Alternate {
    /// CMYK components, `0.0..=1.0`.
    Cmyk([f32; 4]),
    /// CIE L\*a\*b\* relative to D50.
    Lab([f32; 3]),
}

/// A spot ink.
#[derive(Clone, Debug, PartialEq)]
pub struct Spot {
    /// The colorant name, as a `Separation` color space would name it, without PDF name
    /// escapes (see [`normalize_name`]).
    pub name: String,
    /// The ink at 100%.
    pub alternate: Alternate,
    /// How opaque the ink is (`0.0..=1.0`, as InDesign's Ink Manager states it), when known.
    pub solidity: Option<f32>,
}

/// A process or spot ink.
#[derive(Clone, Debug, PartialEq)]
pub enum Ink {
    Process(Process),
    Spot(Spot),
}

impl Ink {
    /// The colorant name: `Cyan` … `Black`, or the spot ink's name.
    pub fn name(&self) -> &str {
        match self {
            Self::Process(p) => p.name(),
            Self::Spot(s) => &s.name,
        }
    }

    pub fn is_spot(&self) -> bool {
        matches!(self, Self::Spot(_))
    }
}

/// An ink's index in its [`InkSet`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InkId(u16);

impl InkId {
    /// The ink's position in [`InkSet::iter`] order, which is also its plane in
    /// [`Plates::planes`](crate::Plates::planes).
    pub fn index(self) -> usize {
        usize::from(self.0)
    }

    /// The id of process ink `p`, the same in every [`InkSet`].
    pub const fn process(p: Process) -> Self {
        Self(p as u16)
    }
}

/// The inks of one document or job: the four process inks, always first and in plate order,
/// then spot inks in the order added. Ids stay valid as inks are added.
///
/// The set is also the job's ink manager, as InDesign's Ink Manager states it: a spot ink can
/// print on another ink's plate ([`alias`](Self::alias)) or as process color
/// ([`set_to_process`](Self::set_to_process), [`set_all_to_process`](Self::set_all_to_process)).
/// [`resolve`](Self::resolve) applies both to a color.
#[derive(Clone, Debug, PartialEq)]
pub struct InkSet {
    inks: Vec<Ink>,
    aliases: BTreeMap<InkId, InkId>,
    to_process: BTreeSet<InkId>,
    all_to_process: bool,
}

impl Default for InkSet {
    fn default() -> Self {
        Self::new()
    }
}

impl InkSet {
    /// The four process inks.
    pub fn new() -> Self {
        Self {
            inks: Process::ALL.into_iter().map(Ink::Process).collect(),
            aliases: BTreeMap::new(),
            to_process: BTreeSet::new(),
            all_to_process: false,
        }
    }

    /// Adds `ink` and returns its id. An ink whose name is already in the set (compared after
    /// [`normalize_name`]) is not added again, and its existing id is returned; a spot named
    /// `Cyan` … `Black` is the process ink. A spot's name is stored normalized. `None` for the
    /// names `All` and `None`, which are not inks (§8.6.6.4), and when the set is full
    /// (65 536 inks).
    pub fn add(&mut self, ink: Ink) -> Option<InkId> {
        let spot = match ink {
            Ink::Process(p) => return Some(InkId::process(p)),
            Ink::Spot(spot) => spot,
        };
        let name = normalize_name(&spot.name).into_owned();
        self.add_decoded(Spot { name, ..spot })
    }

    /// [`add`](Self::add) for a spot whose name is already decoded, as a PDF reader hands it
    /// over: `#` in the name is a literal character.
    pub(crate) fn add_decoded(&mut self, spot: Spot) -> Option<InkId> {
        if spot.name == "All" || spot.name == "None" {
            return None;
        }
        if let Some(id) = self.lookup_decoded(&spot.name) {
            return Some(id);
        }
        let id = InkId(u16::try_from(self.inks.len()).ok()?);
        self.inks.push(Ink::Spot(spot));
        Some(id)
    }

    /// The ink named `name`, compared after [`normalize_name`]: `"/Gold#20Foil"` finds
    /// `Gold Foil`.
    pub fn lookup(&self, name: &str) -> Option<InkId> {
        self.lookup_decoded(&normalize_name(name))
    }

    fn lookup_decoded(&self, name: &str) -> Option<InkId> {
        if let Some(p) = Process::from_name(name) {
            return Some(InkId::process(p));
        }
        let at = self.inks.iter().skip(4).position(|ink| ink.name() == name)?;
        u16::try_from(at + 4).ok().map(InkId)
    }

    pub fn get(&self, id: InkId) -> Option<&Ink> {
        self.inks.get(id.index())
    }

    /// Every ink with its id: the process inks, then the spot inks.
    pub fn iter(&self) -> impl Iterator<Item = (InkId, &Ink)> {
        // At most 65 536 inks (`add`), so every index fits.
        self.inks.iter().enumerate().map(|(i, ink)| (InkId(i as u16), ink))
    }

    /// The spot inks with their ids.
    pub fn spots(&self) -> impl Iterator<Item = (InkId, &Spot)> {
        self.iter().filter_map(|(id, ink)| match ink {
            Ink::Spot(s) => Some((id, s)),
            Ink::Process(_) => None,
        })
    }

    /// The number of inks, process inks included.
    pub fn len(&self) -> usize {
        self.inks.len()
    }

    /// Always `false`: a set holds the process inks.
    pub fn is_empty(&self) -> bool {
        self.inks.is_empty()
    }

    fn spot(&self, id: InkId) -> Option<&Spot> {
        match self.get(id)? {
            Ink::Spot(s) => Some(s),
            Ink::Process(_) => None,
        }
    }

    /// Prints spot ink `from` on ink `to`'s plate. `to` may be a process ink or another spot,
    /// which may itself be aliased or printed as process color; `resolve` follows the chain.
    /// `to == from` removes the alias. Returns `false`, changing nothing, when `from` is not a
    /// spot ink of the set, `to` is not in the set, or the alias would make a loop.
    pub fn alias(&mut self, from: InkId, to: InkId) -> bool {
        if self.spot(from).is_none() || self.get(to).is_none() {
            return false;
        }
        if from == to {
            self.aliases.remove(&from);
            return true;
        }
        if self.plate_chain(to).any(|id| id == from) {
            return false;
        }
        self.aliases.insert(from, to);
        true
    }

    /// The ink spot ink `from` is aliased to, if any.
    pub fn alias_of(&self, from: InkId) -> Option<InkId> {
        self.aliases.get(&from).copied()
    }

    /// Prints spot ink `ink` as process color (`on`), converted from its alternate, or on its
    /// own plate. Returns `false`, changing nothing, when `ink` is not a spot ink of the set.
    pub fn set_to_process(&mut self, ink: InkId, on: bool) -> bool {
        if self.spot(ink).is_none() {
            return false;
        }
        if on {
            self.to_process.insert(ink);
        } else {
            self.to_process.remove(&ink);
        }
        true
    }

    /// Prints every spot ink as process color ("All Spots to Process"), whatever
    /// [`set_to_process`](Self::set_to_process) says for each. Aliases still apply first: a
    /// spot aliased to a process ink prints on that ink's plate.
    pub fn set_all_to_process(&mut self, on: bool) {
        self.all_to_process = on;
    }

    pub fn all_to_process(&self) -> bool {
        self.all_to_process
    }

    /// Whether spot ink `ink` prints as process color when a color names it directly (an
    /// alias of it is followed first, see [`plate_of`](Self::plate_of)).
    pub fn is_to_process(&self, ink: InkId) -> bool {
        self.spot(ink).is_some() && (self.all_to_process || self.to_process.contains(&ink))
    }

    /// `id`, then each ink it is aliased to in turn. Stops after as many steps as there are
    /// inks, so a loop can't make it endless.
    fn plate_chain(&self, id: InkId) -> impl Iterator<Item = InkId> + '_ {
        std::iter::successors(Some(id), |id| self.alias_of(*id)).take(self.inks.len())
    }

    /// The end of `id`'s alias chain: `id` itself when it has no alias. It prints on its own
    /// plate unless [`is_to_process`](Self::is_to_process) says it converts. `None` when `id`
    /// is not in the set.
    pub fn alias_end(&self, id: InkId) -> Option<InkId> {
        self.get(id)?;
        self.plate_chain(id).last()
    }

    /// The ink whose plate a tint of `id` prints on after aliases: `id` itself, the end of its
    /// alias chain, or `None` when that ink prints as process color (or `id` is not in the
    /// set).
    pub fn plate_of(&self, id: InkId) -> Option<InkId> {
        let end = self.alias_end(id)?;
        (!self.is_to_process(end)).then_some(end)
    }

    /// Spot ink `id` at 100% as process color, `0.0..=1.0` per component: its CMYK alternate
    /// as it is, a Lab alternate through `cms`, or, when `cms` can't convert, through sRGB and
    /// the ISO 32000-2 §10.4.2.4 formulas (full black generation and undercolor removal).
    /// `None` for process inks, inks not in the set, and Lab colors craft-cms can't convert.
    pub fn spot_cmyk(&self, id: InkId, cms: &dyn CmykConversion) -> Option<[f32; 4]> {
        let lab = match self.spot(id)?.alternate {
            Alternate::Cmyk(cmyk) => return Some(cmyk.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 })),
            Alternate::Lab(lab) => lab,
        };
        let mut cmyk = [0.0_f32; 4];
        if cms.from_pcs_xyz(&math::lab_to_xyz_f32(lab), &mut cmyk).is_some() {
            return Some(cmyk);
        }
        let mut rgb = [0_u8; 3];
        lab_to_srgb8(&lab, &mut rgb)?;
        let [c, m, y] = rgb.map(|v| 1.0 - f32::from(v) / 255.0);
        let k = c.min(m).min(y);
        Some([c - k, m - k, y - k, k])
    }

    /// `color` as it prints with this set's aliases and to-process settings: each tint moves
    /// to the plate [`plate_of`](Self::plate_of) names, and a tint of an ink printed as process
    /// color becomes that tint of its [`spot_cmyk`](Self::spot_cmyk). Tints that land on one
    /// plate combine as overlapping halftones, `1 − (1 − a)(1 − b)`. When any tint became
    /// process color the result lists all four process inks first (a CMYK color if no spot
    /// is left); otherwise inks keep their order. Inks not in the set, and spots whose Lab
    /// alternate can't be converted, stay as they are. `All` and `None` are unchanged.
    pub fn resolve(&self, color: &InkColor, cms: &dyn CmykConversion) -> InkColor {
        let InkColor::Tints(tints) = color else { return color.clone() };
        let mut out: Vec<(InkId, f32)> = Vec::with_capacity(tints.len());
        let mut process: Option<[f32; 4]> = None;
        let combine = |a: f32, b: f32| 1.0 - (1.0 - a) * (1.0 - b);
        for &(id, tint) in tints {
            let end = match self.get(id) {
                Some(_) => self.plate_chain(id).last().unwrap_or(id),
                None => id,
            };
            if self.is_to_process(end)
                && let Some(cmyk) = self.spot_cmyk(end, cms)
            {
                let acc = process.get_or_insert([0.0; 4]);
                for (a, c) in acc.iter_mut().zip(cmyk) {
                    *a = combine(*a, c * tint);
                }
                continue;
            }
            match out.iter_mut().find(|(i, _)| *i == end) {
                Some((_, t)) => *t = combine(*t, tint),
                None => out.push((end, tint)),
            }
        }
        let Some(mut cmyk) = process else { return InkColor::Tints(out) };
        out.retain(|(id, tint)| match Process::ALL.get(id.index()) {
            Some(p) => {
                if let Some(c) = cmyk.get_mut(p.index()) {
                    *c = combine(*c, *tint);
                }
                false
            }
            None => true,
        });
        let mut all: Vec<(InkId, f32)> = Process::ALL.iter().zip(cmyk).map(|(p, c)| (InkId::process(*p), c)).collect();
        all.extend(out);
        InkColor::Tints(all)
    }
}

/// A color as ink tints.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum InkColor {
    /// Tints `0.0..=1.0` of the listed inks; other inks are unspecified (they keep what is
    /// under the paint when it overprints, and are knocked out otherwise). One entry is a
    /// `Separation` color, several a `DeviceN` one.
    Tints(Vec<(InkId, f32)>),
    /// The same tint on every plate: the registration color (the `All` colorant).
    All(f32),
    /// No ink on any plate (the `None` colorant).
    None,
}

impl InkColor {
    /// The color as DeviceCMYK components when every tint is of a process ink, as
    /// [`InkSet::resolve`] leaves a spot printed as process color; process inks not listed
    /// are 0. `None` when a spot ink is listed, and for `All` and `None`.
    pub fn as_cmyk(&self) -> Option<[f32; 4]> {
        let Self::Tints(tints) = self else { return None };
        let mut cmyk = [0.0; 4];
        for (id, tint) in tints {
            *cmyk.get_mut(id.index())? = *tint;
        }
        Some(cmyk)
    }
}

/// `color` as a krilla fill or stroke color, for the PDF writer. Pass the color as it prints,
/// after [`InkSet::resolve`].
///
/// - Tints of process inks only: DeviceCMYK, unlisted inks 0 (so with `OPM 1` an overprinting
///   paint leaves their plates as they are).
/// - One spot ink alone: a `Separation` space with the spot's alternate as DeviceCMYK fallback.
/// - Any other mix: a `DeviceN` space, process inks first in plate order, then spot inks in
///   set order. Each spot keeps the alternate its `Separation` space has.
/// - `All`: the registration color, a `Separation /All` space; `None`: a `Separation /None`
///   space, which marks no plate.
///
/// Spot alternates are [`InkSet::spot_cmyk`]: a CMYK alternate as given, a Lab one through
/// `cms` into the printing condition. A tint listed twice combines as overlapping halftones,
/// as in `resolve`. `None` when `Tints` lists no ink (there is nothing to paint; use
/// [`InkColor::None`] to mark no plate), a tint's ink is not in `inks`, a spot's Lab alternate
/// can't be converted, a spot name is empty, or a `DeviceN` space would have more than 32
/// colorants.
pub fn to_pdf_color(color: &InkColor, inks: &InkSet, cms: &dyn CmykConversion) -> Option<PdfColor> {
    let tints = match color {
        InkColor::All(tint) => return Some(separation(SeparationColorant::AllColorants, *tint, cmyk::Color::new(255, 255, 255, 255))),
        InkColor::None => return Some(separation(SeparationColorant::NoColorant, 0.0, cmyk::Color::new(0, 0, 0, 0))),
        InkColor::Tints(tints) if tints.is_empty() => return None,
        InkColor::Tints(tints) => tints,
    };
    // Sorted by id: process inks first, in plate order, then spots in set order.
    let mut merged: BTreeMap<InkId, f32> = BTreeMap::new();
    for &(id, tint) in tints {
        inks.get(id)?;
        let t = merged.entry(id).or_insert(0.0);
        *t = 1.0 - (1.0 - *t) * (1.0 - unit(tint));
    }
    if merged.keys().all(|id| id.index() < Process::ALL.len()) {
        let mut cmyk = [0_u8; 4];
        for (id, tint) in &merged {
            if let Some(c) = cmyk.get_mut(id.index()) {
                *c = byte(*tint);
            }
        }
        let [c, m, y, k] = cmyk;
        return Some(RegularColor::Cmyk(cmyk::Color::new(c, m, y, k)).into());
    }
    let spot = |id: InkId| -> Option<(String, cmyk::Color)> {
        let name = inks.get(id)?.name();
        let [c, m, y, k] = inks.spot_cmyk(id, cms)?.map(byte);
        (!name.is_empty()).then(|| (name.to_owned(), cmyk::Color::new(c, m, y, k)))
    };
    if let [(&id, &tint)] = merged.iter().collect::<Vec<_>>()[..] {
        let (name, alternate) = spot(id)?;
        return Some(separation(SeparationColorant::Custom(name), tint, alternate));
    }
    let mut colorants = Vec::with_capacity(merged.len());
    for &id in merged.keys() {
        colorants.push(match Process::ALL.get(id.index()) {
            Some(p) => Colorant::Process(match p {
                Process::Cyan => device_n::Process::Cyan,
                Process::Magenta => device_n::Process::Magenta,
                Process::Yellow => device_n::Process::Yellow,
                Process::Black => device_n::Process::Black,
            }),
            None => {
                let (name, alternate) = spot(id)?;
                Colorant::Spot { name, alternate }
            }
        });
    }
    let space = DeviceNSpace::new(colorants)?;
    Some(device_n::Color::new(merged.values().map(|t| byte(*t)).collect(), space)?.into())
}

fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

fn byte(v: f32) -> u8 {
    (unit(v) * 255.0).round() as u8
}

fn separation(colorant: SeparationColorant, tint: f32, fallback: cmyk::Color) -> PdfColor {
    separation::Color::new(byte(tint), SeparationSpace::new(colorant, RegularColor::Cmyk(fallback))).into()
}

/// Overprint parameters of a paint (ISO 32000-2 §8.6.7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Overprint {
    /// Fills and other non-stroking paints overprint (`op`).
    pub fill: bool,
    /// Strokes overprint (`OP`).
    pub stroke: bool,
    /// Nonzero overprint mode (`OPM 1`): a zero DeviceCMYK component leaves its plate as it is.
    pub nonzero: bool,
}

/// A colorant name as a PDF name object carries it: without a leading `/` and with `#xx`
/// escapes decoded (ISO 32000-2 §7.3.5), read as UTF-8 (invalid sequences replaced).
pub fn normalize_name(name: &str) -> Cow<'_, str> {
    let name = name.strip_prefix('/').unwrap_or(name);
    if !name.contains('#') {
        return Cow::Borrowed(name);
    }
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b, hex) {
            (b'#', Some(decoded)) => {
                out.push(decoded);
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    Cow::Owned(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(name: &str) -> Ink {
        Ink::Spot(Spot { name: name.into(), alternate: Alternate::Lab([80.0, 5.0, 60.0]), solidity: None })
    }

    #[test]
    fn spots_follow_the_process_inks_and_names_are_compared_as_pdf_names() {
        let mut set = InkSet::new();
        let gold = set.add(spot("/Gold#20Foil")).unwrap();
        assert_eq!(gold.index(), 4);
        assert_eq!(set.get(gold).unwrap().name(), "Gold Foil");
        assert_eq!(set.add(spot("Gold Foil")), Some(gold), "added once");
        assert_eq!(set.lookup("Gold#20Foil"), Some(gold));
        assert_eq!(set.add(spot("Black")), Some(InkId::process(Process::Black)));
        assert_eq!(set.lookup("/Cyan"), Some(InkId::process(Process::Cyan)));
        assert_eq!(set.add(spot("All")), None);
        assert_eq!(set.add(spot("None")), None);
        assert_eq!(set.lookup("gold foil"), None, "names are case-sensitive");
        let names: Vec<&str> = set.iter().map(|(_, ink)| ink.name()).collect();
        assert_eq!(names, ["Cyan", "Magenta", "Yellow", "Black", "Gold Foil"]);
        assert_eq!(set.spots().count(), 1);
    }

    fn close(got: Option<[f32; 4]>, expected: [f32; 4]) -> bool {
        got.is_some_and(|got| got.iter().zip(expected).all(|(g, e)| (g - e).abs() < 1e-4))
    }

    #[test]
    fn aliases_and_spots_to_process_resolve_colors() {
        let mut set = InkSet::new();
        let gold = set.add(Ink::Spot(Spot { name: "Gold".into(), alternate: Alternate::Cmyk([0.0, 0.3, 1.0, 0.0]), solidity: None })).unwrap();
        let yellow = set.add(spot("Lab Yellow")).unwrap();
        let varnish = set.add(spot("Varnish")).unwrap();
        let k = InkId::process(Process::Black);
        let cms = crate::CmsConversion::fogra39();
        let tints = |t: &[(InkId, f32)]| InkColor::Tints(t.to_vec());

        let plain = tints(&[(gold, 0.5), (k, 1.0)]);
        assert_eq!(set.resolve(&plain, &cms), plain, "no settings: unchanged");
        assert_eq!(set.resolve(&InkColor::All(1.0), &cms), InkColor::All(1.0));

        assert!(set.alias(varnish, gold));
        assert_eq!((set.alias_of(varnish), set.plate_of(varnish)), (Some(gold), Some(gold)));
        assert_eq!(set.resolve(&tints(&[(varnish, 0.5)]), &cms), tints(&[(gold, 0.5)]));
        assert_eq!(set.resolve(&tints(&[(gold, 0.5), (varnish, 0.5)]), &cms), tints(&[(gold, 0.75)]), "one plate: tints overlap");
        assert!(!set.alias(gold, varnish), "a loop is refused");
        assert!(!set.alias(k, gold), "process inks aren't aliased");
        assert!(!set.alias(gold, InkId(99)));

        assert!(set.set_to_process(gold, true));
        assert!(!set.set_to_process(k, true));
        assert_eq!(set.plate_of(varnish), None, "aliased to a spot printed as process");
        assert_eq!(set.alias_end(varnish), Some(gold), "the converted ink is still named");
        assert!(set.is_to_process(gold));
        assert_eq!(set.alias_end(InkId(99)), None);
        assert!(close(set.resolve(&tints(&[(varnish, 0.5)]), &cms).as_cmyk(), [0.0, 0.15, 0.5, 0.0]));
        let rich = set.resolve(&tints(&[(k, 0.5), (gold, 1.0)]), &cms);
        assert!(close(rich.as_cmyk(), [0.0, 0.3, 1.0, 0.5]), "{rich:?}");
        let InkColor::Tints(mixed) = set.resolve(&tints(&[(yellow, 0.4), (gold, 1.0)]), &cms) else { panic!() };
        let order: Vec<InkId> = mixed.iter().map(|(id, _)| *id).collect();
        assert_eq!(order[..4], Process::ALL.map(InkId::process), "process inks first");
        assert_eq!(mixed[4], (yellow, 0.4), "the spot left on its plate");

        set.set_all_to_process(true);
        assert!(set.is_to_process(yellow) && !set.is_to_process(k));
        let lab = set.resolve(&tints(&[(yellow, 1.0)]), &cms).as_cmyk().unwrap();
        assert!(lab[2] > 0.4 && lab[2] > lab[0] && lab[3] < 0.2, "a light yellow through FOGRA39: {lab:?}");
        assert!(set.alias(yellow, k));
        assert_eq!(set.resolve(&tints(&[(yellow, 0.5)]), &cms), tints(&[(k, 0.5)]), "aliases apply first");

        assert!(set.alias(varnish, varnish));
        assert_eq!(set.alias_of(varnish), None);
    }

    #[test]
    fn malformed_escapes_stay_as_written() {
        assert_eq!(normalize_name("A#2"), "A#2");
        assert_eq!(normalize_name("A#zz#41"), "A#zzA");
        assert_eq!(normalize_name("#E2#82#AC"), "€");
    }
}
