//! Editing a [`ThemeDef`]: [`Edit`] operations, [`diff`] between a saved theme and a draft, and
//! the revert of one [`Change`].
//!
//! A [`Key`] names one editable value of a theme. [`read`] and [`write`] are the only code that
//! knows where a value lives; edits, diffs and reverts all go through them. Every result is
//! checked with [`ThemeDef::validate`], so an edit is refused exactly where a theme file would be.

use crate::color::Rgba;
use crate::format::FormatErrors;
use crate::id::{AppId, IconPackId};
use crate::ramp::Mode;
use crate::theme::{AppOverrides, ContrastIntent, EdgeSeed, ModePins, Pins, RadiusOffset, SeedOverrides, Seeds, ShapeSeed, ThemeDef, Variants};
use crate::tokens::{ColorToken, KnobShape, RadiusSet, WellShape};

/// One of the six seed colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    /// The grey family and its tint.
    Neutral,
    /// The accent hue.
    Accent,
    /// The `good` status hue.
    Good,
    /// The `warn` status hue.
    Warn,
    /// The `bad` status hue.
    Bad,
    /// The `info` status hue.
    Info,
}

impl Seed {
    /// Every seed, in the order a theme file lists them.
    pub const ALL: [Seed; 6] = [Seed::Neutral, Seed::Accent, Seed::Good, Seed::Warn, Seed::Bad, Seed::Info];

    fn get(self, seeds: &Seeds) -> Rgba {
        match self {
            Seed::Neutral => seeds.neutral,
            Seed::Accent => seeds.accent,
            Seed::Good => seeds.status.good,
            Seed::Warn => seeds.status.warn,
            Seed::Bad => seeds.status.bad,
            Seed::Info => seeds.status.info,
        }
    }

    fn set(self, seeds: &mut Seeds, color: Rgba) {
        match self {
            Seed::Neutral => seeds.neutral = color,
            Seed::Accent => seeds.accent = color,
            Seed::Good => seeds.status.good = color,
            Seed::Warn => seeds.status.warn = color,
            Seed::Bad => seeds.status.bad = color,
            Seed::Info => seeds.status.info = color,
        }
    }

    fn override_slot(self, o: &mut SeedOverrides) -> &mut Option<Rgba> {
        match self {
            Seed::Neutral => &mut o.neutral,
            Seed::Accent => &mut o.accent,
            Seed::Good => &mut o.status.good,
            Seed::Warn => &mut o.status.warn,
            Seed::Bad => &mut o.status.bad,
            Seed::Info => &mut o.status.info,
        }
    }
}

/// Which pin map of a theme an edit writes: the theme's own or an app's, Standard contrast or
/// High.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinSlot {
    /// The app whose section is written; `None` for the theme's own pins.
    pub app: Option<AppId>,
    /// Whether the map applies with high contrast only.
    pub high_contrast: bool,
}

impl PinSlot {
    /// The theme's own Standard-contrast pins.
    pub fn theme() -> PinSlot {
        PinSlot { app: None, high_contrast: false }
    }

    /// The theme's own high-contrast pins.
    pub fn theme_high_contrast() -> PinSlot {
        PinSlot { app: None, high_contrast: true }
    }

    /// Every slot a theme can hold: its own two, then two per app.
    fn all(theme: &ThemeDef) -> Vec<PinSlot> {
        let mut slots = vec![PinSlot::theme(), PinSlot::theme_high_contrast()];
        for id in theme.apps.keys() {
            slots.push(PinSlot { app: Some(id.clone()), high_contrast: false });
            slots.push(PinSlot { app: Some(id.clone()), high_contrast: true });
        }
        slots
    }

    fn pins<'a>(&self, theme: &'a ThemeDef) -> Option<&'a ModePins> {
        match &self.app {
            None if self.high_contrast => Some(&theme.high_contrast),
            None => Some(&theme.pins),
            Some(id) => theme.apps.get(id).map(|a| if self.high_contrast { &a.high_contrast } else { &a.pins }),
        }
    }

    /// The pins to write; an app's section is created when `create` is set.
    fn pins_mut<'a>(&self, theme: &'a mut ThemeDef, create: bool) -> Option<&'a mut ModePins> {
        match &self.app {
            None if self.high_contrast => Some(&mut theme.high_contrast),
            None => Some(&mut theme.pins),
            Some(id) => {
                if create && !theme.apps.contains_key(id) {
                    theme.apps.insert(id.clone(), AppOverrides::default());
                }
                theme.apps.get_mut(id).map(|a| if self.high_contrast { &mut a.high_contrast } else { &mut a.pins })
            }
        }
    }
}

/// A shape field of a theme and its value; `None` takes the Size preset's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeValue {
    /// [`ShapeSeed::radius`].
    Radius(Option<RadiusSet>),
    /// [`ShapeSeed::radius_offset`].
    RadiusOffset(Option<RadiusOffset>),
    /// [`ShapeSeed::button_radius`].
    ButtonRadius(Option<f32>),
    /// [`ShapeSeed::edge`].
    Edge(Option<EdgeSeed>),
    /// [`ShapeSeed::knob`].
    Knob(Option<KnobShape>),
    /// [`ShapeSeed::wells`].
    Wells(Option<WellShape>),
}

/// Which field of the shape a [`ShapeValue`] holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeField {
    /// [`ShapeSeed::radius`].
    Radius,
    /// [`ShapeSeed::radius_offset`].
    RadiusOffset,
    /// [`ShapeSeed::button_radius`].
    ButtonRadius,
    /// [`ShapeSeed::edge`].
    Edge,
    /// [`ShapeSeed::knob`].
    Knob,
    /// [`ShapeSeed::wells`].
    Wells,
}

impl ShapeField {
    const ALL: [ShapeField; 6] =
        [ShapeField::Radius, ShapeField::RadiusOffset, ShapeField::ButtonRadius, ShapeField::Edge, ShapeField::Knob, ShapeField::Wells];

    fn get(self, s: &ShapeSeed) -> ShapeValue {
        match self {
            ShapeField::Radius => ShapeValue::Radius(s.radius),
            ShapeField::RadiusOffset => ShapeValue::RadiusOffset(s.radius_offset),
            ShapeField::ButtonRadius => ShapeValue::ButtonRadius(s.button_radius),
            ShapeField::Edge => ShapeValue::Edge(s.edge),
            ShapeField::Knob => ShapeValue::Knob(s.knob),
            ShapeField::Wells => ShapeValue::Wells(s.wells),
        }
    }
}

impl ShapeValue {
    fn field(self) -> ShapeField {
        match self {
            ShapeValue::Radius(_) => ShapeField::Radius,
            ShapeValue::RadiusOffset(_) => ShapeField::RadiusOffset,
            ShapeValue::ButtonRadius(_) => ShapeField::ButtonRadius,
            ShapeValue::Edge(_) => ShapeField::Edge,
            ShapeValue::Knob(_) => ShapeField::Knob,
            ShapeValue::Wells(_) => ShapeField::Wells,
        }
    }

    fn set(self, s: &mut ShapeSeed) {
        match self {
            ShapeValue::Radius(v) => s.radius = v,
            ShapeValue::RadiusOffset(v) => s.radius_offset = v,
            ShapeValue::ButtonRadius(v) => s.button_radius = v,
            ShapeValue::Edge(v) => s.edge = v,
            ShapeValue::Knob(v) => s.knob = v,
            ShapeValue::Wells(v) => s.wells = v,
        }
    }
}

const NO_SHAPE: ShapeSeed = ShapeSeed { radius: None, radius_offset: None, button_radius: None, edge: None, knob: None, wells: None };

/// A theme's name, variants, contrast intent or icon pack.
#[derive(Clone, Debug, PartialEq)]
pub enum Meta {
    /// [`ThemeDef::name`].
    Name(String),
    /// [`ThemeDef::variants`].
    Variants(Variants),
    /// [`ThemeDef::contrast_intent`].
    ContrastIntent(ContrastIntent),
    /// [`ThemeDef::icon_pack`]; `None` takes the suite's pack.
    IconPack(Option<IconPackId>),
}

/// What a pin map holds for one token: a colour, a reference to another token, or both (a
/// reference wins when resolving).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinEntry {
    /// The pinned colour.
    pub color: Option<Rgba>,
    /// The token whose final value this one takes.
    pub reference: Option<ColorToken>,
}

/// One editable value of a theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// [`ThemeDef::name`].
    Name,
    /// [`ThemeDef::variants`].
    Variants,
    /// [`ThemeDef::contrast_intent`].
    ContrastIntent,
    /// [`ThemeDef::icon_pack`].
    IconPack,
    /// A seed of [`ThemeDef::seeds`].
    Seed(Seed),
    /// A seed one mode replaces.
    ModeSeed(Mode, Seed),
    /// The pin of a token in one slot and mode.
    Pin(PinSlot, Mode, ColorToken),
    /// A field of [`ThemeDef::shape`].
    Shape(ShapeField),
}

/// The value of a [`Key`].
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// The name.
    Name(String),
    /// The variants.
    Variants(Variants),
    /// The contrast intent.
    ContrastIntent(ContrastIntent),
    /// The icon pack.
    IconPack(IconPackId),
    /// A seed colour, for [`Key::Seed`] and [`Key::ModeSeed`].
    Color(Rgba),
    /// A pin.
    Pin(PinEntry),
    /// A shape field; a field left to the Size preset is `None` inside.
    Shape(ShapeValue),
}

/// One edit of a theme.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// Sets a seed for every mode.
    SetSeed {
        /// Which seed.
        seed: Seed,
        /// Its new colour.
        color: Rgba,
    },
    /// Sets a seed for one mode only, or with `None` removes that mode's override.
    SetModeSeed {
        /// The mode.
        mode: Mode,
        /// Which seed.
        seed: Seed,
        /// Its colour in that mode.
        color: Option<Rgba>,
    },
    /// Pins a token to a colour, replacing a reference on that token in the slot.
    SetPin {
        /// The pin map.
        slot: PinSlot,
        /// The mode.
        mode: Mode,
        /// The token.
        token: ColorToken,
        /// The colour.
        color: Rgba,
    },
    /// Makes a token take another token's value, replacing a colour pin on it in the slot.
    SetReference {
        /// The pin map.
        slot: PinSlot,
        /// The mode.
        mode: Mode,
        /// The token that follows.
        token: ColorToken,
        /// The token followed.
        target: ColorToken,
    },
    /// Removes a token's pin and reference from the slot, back to the derived value.
    ClearPin {
        /// The pin map.
        slot: PinSlot,
        /// The mode.
        mode: Mode,
        /// The token.
        token: ColorToken,
    },
    /// Sets one shape field.
    SetShape(ShapeValue),
    /// Sets the name, variants, contrast intent or icon pack.
    SetMeta(Meta),
}

impl Edit {
    fn target(&self) -> (Key, Option<Value>) {
        match self {
            Edit::SetSeed { seed, color } => (Key::Seed(*seed), Some(Value::Color(*color))),
            Edit::SetModeSeed { mode, seed, color } => (Key::ModeSeed(*mode, *seed), color.map(Value::Color)),
            Edit::SetPin { slot, mode, token, color } => {
                (Key::Pin(slot.clone(), *mode, *token), Some(Value::Pin(PinEntry { color: Some(*color), reference: None })))
            }
            Edit::SetReference { slot, mode, token, target } => {
                (Key::Pin(slot.clone(), *mode, *token), Some(Value::Pin(PinEntry { color: None, reference: Some(*target) })))
            }
            Edit::ClearPin { slot, mode, token } => (Key::Pin(slot.clone(), *mode, *token), None),
            Edit::SetShape(v) => (Key::Shape(v.field()), Some(Value::Shape(*v))),
            Edit::SetMeta(Meta::Name(n)) => (Key::Name, Some(Value::Name(n.clone()))),
            Edit::SetMeta(Meta::Variants(v)) => (Key::Variants, Some(Value::Variants(*v))),
            Edit::SetMeta(Meta::ContrastIntent(v)) => (Key::ContrastIntent, Some(Value::ContrastIntent(*v))),
            Edit::SetMeta(Meta::IconPack(v)) => (Key::IconPack, v.clone().map(Value::IconPack)),
        }
    }
}

/// `theme` with `edit` applied, or the errors [`ThemeDef::validate`] reports for the result: a
/// mode the theme's variants leave out, a translucent surface pin, a reference cycle, a bad
/// length. `theme` is not changed.
pub fn apply(theme: &ThemeDef, edit: &Edit) -> Result<ThemeDef, FormatErrors> {
    let (key, value) = edit.target();
    let mut out = theme.clone();
    write(&mut out, &key, value);
    checked(out)
}

fn checked(theme: ThemeDef) -> Result<ThemeDef, FormatErrors> {
    theme.validate()?;
    Ok(theme)
}

fn pin_map(pins: &Pins, token: ColorToken) -> Option<PinEntry> {
    let entry = PinEntry { color: pins.colors.get(&token).copied(), reference: pins.refs.get(&token).copied() };
    (entry.color.is_some() || entry.reference.is_some()).then_some(entry)
}

fn mode_pins(pins: &ModePins, mode: Mode) -> &Pins {
    pins.get(mode)
}

fn mode_pins_mut(pins: &mut ModePins, mode: Mode) -> &mut Pins {
    match mode {
        Mode::Light => &mut pins.light,
        Mode::Dark => &mut pins.dark,
    }
}

/// The value of `key` in `theme`; `None` for an unset mode seed or pin.
pub fn read(theme: &ThemeDef, key: &Key) -> Option<Value> {
    match key {
        Key::Name => Some(Value::Name(theme.name.clone())),
        Key::Variants => Some(Value::Variants(theme.variants)),
        Key::ContrastIntent => Some(Value::ContrastIntent(theme.contrast_intent)),
        Key::IconPack => theme.icon_pack.clone().map(Value::IconPack),
        Key::Seed(seed) => Some(Value::Color(seed.get(&theme.seeds))),
        Key::ModeSeed(mode, seed) => {
            let mut o = *theme.mode_seeds.get(*mode);
            (*seed.override_slot(&mut o)).map(Value::Color)
        }
        Key::Pin(slot, mode, token) => {
            let pins = slot.pins(theme)?;
            pin_map(mode_pins(pins, *mode), *token).map(Value::Pin)
        }
        Key::Shape(field) => Some(Value::Shape(field.get(&theme.shape.unwrap_or(NO_SHAPE)))),
    }
}

/// Sets `key` to `value`, `None` removing a mode seed or pin. A value of the wrong kind for the
/// key is ignored. The result is not validated.
fn write(theme: &mut ThemeDef, key: &Key, value: Option<Value>) {
    match (key, value) {
        (Key::Name, Some(Value::Name(n))) => theme.name = n,
        (Key::Variants, Some(Value::Variants(v))) => theme.variants = v,
        (Key::ContrastIntent, Some(Value::ContrastIntent(v))) => theme.contrast_intent = v,
        (Key::IconPack, Some(Value::IconPack(v))) => theme.icon_pack = Some(v),
        (Key::IconPack, None) => theme.icon_pack = None,
        (Key::Seed(seed), Some(Value::Color(c))) => seed.set(&mut theme.seeds, c),
        (Key::ModeSeed(mode, seed), value) => {
            let o = match mode {
                Mode::Light => &mut theme.mode_seeds.light,
                Mode::Dark => &mut theme.mode_seeds.dark,
            };
            match value {
                Some(Value::Color(c)) => *seed.override_slot(o) = Some(c),
                None => *seed.override_slot(o) = None,
                Some(_) => {}
            }
        }
        (Key::Pin(slot, mode, token), value) => {
            let entry = match value {
                Some(Value::Pin(e)) => Some(e),
                None => None,
                Some(_) => return,
            };
            let Some(maps) = slot.pins_mut(theme, entry.is_some()) else { return };
            let pins = mode_pins_mut(maps, *mode);
            pins.colors.remove(token);
            pins.refs.remove(token);
            if let Some(e) = entry {
                if let Some(c) = e.color {
                    pins.colors.insert(*token, c);
                }
                if let Some(r) = e.reference {
                    pins.refs.insert(*token, r);
                }
            }
            if let Some(id) = &slot.app
                && theme.apps.get(id).is_some_and(|a| a.pins == ModePins::default() && a.high_contrast == ModePins::default())
            {
                theme.apps.remove(id);
            }
        }
        (Key::Shape(_), Some(Value::Shape(v))) => {
            let mut shape = theme.shape.unwrap_or(NO_SHAPE);
            v.set(&mut shape);
            theme.shape = (shape != NO_SHAPE).then_some(shape);
        }
        _ => {}
    }
}

/// How a value differs between the saved theme and the draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// Set in the draft only.
    Added,
    /// Set in the saved theme only.
    Removed,
    /// Set in both, with different values.
    Changed,
}

/// One difference between a saved theme and a draft.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// What differs.
    pub key: Key,
    /// How it differs.
    pub kind: ChangeKind,
    /// The saved theme's value.
    pub saved: Option<Value>,
    /// The draft's value.
    pub draft: Option<Value>,
}

impl Change {
    /// `draft` with this one value set back to the saved theme's, the other changes kept. Refused
    /// when the result is not valid, for instance when reverting the variants would leave pins for
    /// a mode out.
    pub fn revert(&self, draft: &ThemeDef) -> Result<ThemeDef, FormatErrors> {
        let mut out = draft.clone();
        write(&mut out, &self.key, self.saved.clone());
        checked(out)
    }
}

/// Every key a theme sets or may set: the fixed ones, then each pin and reference present.
fn keys(theme: &ThemeDef, into: &mut Vec<Key>) {
    let mut push = |k: Key| {
        if !into.contains(&k) {
            into.push(k);
        }
    };
    push(Key::Name);
    push(Key::Variants);
    push(Key::ContrastIntent);
    push(Key::IconPack);
    for seed in Seed::ALL {
        push(Key::Seed(seed));
    }
    for mode in [Mode::Light, Mode::Dark] {
        for seed in Seed::ALL {
            push(Key::ModeSeed(mode, seed));
        }
    }
    for field in ShapeField::ALL {
        push(Key::Shape(field));
    }
    for slot in PinSlot::all(theme) {
        let Some(maps) = slot.pins(theme) else { continue };
        for mode in [Mode::Light, Mode::Dark] {
            let pins = mode_pins(maps, mode);
            for &token in pins.colors.keys().chain(pins.refs.keys()) {
                push(Key::Pin(slot.clone(), mode, token));
            }
        }
    }
}

/// Every difference between `saved` and `draft`: name, variants and intent, seeds, mode seeds,
/// shape, then pins of the theme, its high-contrast pins and each app's.
pub fn diff(saved: &ThemeDef, draft: &ThemeDef) -> Vec<Change> {
    let mut all = Vec::new();
    keys(saved, &mut all);
    keys(draft, &mut all);
    all.into_iter()
        .filter_map(|key| {
            let (s, d) = (read(saved, &key), read(draft, &key));
            if s == d {
                return None;
            }
            let kind = match (&s, &d) {
                (None, _) => ChangeKind::Added,
                (_, None) => ChangeKind::Removed,
                _ => ChangeKind::Changed,
            };
            Some(Change { key, kind, saved: s, draft: d })
        })
        .collect()
}
