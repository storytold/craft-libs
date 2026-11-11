//! The value of a layer's style: effect instances, their parameters, the Blending Options.

use std::collections::BTreeMap;

use craft_theme::Srgba;

use super::contour::Contour;

/// The ten effects of Photoshop's Layer Style dialog, in its list order (Bevel & Emboss first,
/// Drop Shadow last).
// pc: crates/ui-egui/src/dialogs/layer_style.rs:53-64 `KINDS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum EffectKind {
    #[allow(missing_docs)]
    BevelEmboss,
    #[allow(missing_docs)]
    Stroke,
    #[allow(missing_docs)]
    InnerShadow,
    #[allow(missing_docs)]
    InnerGlow,
    #[allow(missing_docs)]
    Satin,
    #[allow(missing_docs)]
    ColorOverlay,
    #[allow(missing_docs)]
    GradientOverlay,
    #[allow(missing_docs)]
    PatternOverlay,
    #[allow(missing_docs)]
    OuterGlow,
    #[allow(missing_docs)]
    DropShadow,
}

impl EffectKind {
    /// Every kind in the list order.
    pub const ALL: [EffectKind; 10] = [
        EffectKind::BevelEmboss,
        EffectKind::Stroke,
        EffectKind::InnerShadow,
        EffectKind::InnerGlow,
        EffectKind::Satin,
        EffectKind::ColorOverlay,
        EffectKind::GradientOverlay,
        EffectKind::PatternOverlay,
        EffectKind::OuterGlow,
        EffectKind::DropShadow,
    ];

    /// The kind's stable key, PhotoCraft's command kind: `bevelEmboss`, `stroke`, `innerShadow`,
    /// `innerGlow`, `satin`, `colorOverlay`, `gradientOverlay`, `patternOverlay`, `outerGlow`,
    /// `dropShadow`. Apps convert their engine's effects through it.
    pub fn key(self) -> &'static str {
        // pc: crates/ui-egui/src/dialogs/layer_style.rs:53-64 `KINDS` (the first column).
        match self {
            EffectKind::BevelEmboss => "bevelEmboss",
            EffectKind::Stroke => "stroke",
            EffectKind::InnerShadow => "innerShadow",
            EffectKind::InnerGlow => "innerGlow",
            EffectKind::Satin => "satin",
            EffectKind::ColorOverlay => "colorOverlay",
            EffectKind::GradientOverlay => "gradientOverlay",
            EffectKind::PatternOverlay => "patternOverlay",
            EffectKind::OuterGlow => "outerGlow",
            EffectKind::DropShadow => "dropShadow",
        }
    }

    /// The kind with [`key`](Self::key) `key`; `None` for any other text.
    pub fn from_key(key: &str) -> Option<EffectKind> {
        // pc: crates/ui-egui/src/dialogs/layer_style.rs:53-64 `KINDS`.
        EffectKind::ALL.into_iter().find(|kind| kind.key() == key)
    }

    /// Whether a layer may hold several instances of the kind (the `+` and `−` buttons): Stroke,
    /// Drop Shadow, Inner Shadow, Color Overlay and Gradient Overlay, as Photoshop stores
    /// `…Multi` lists in a PSD.
    pub fn allows_several(self) -> bool {
        // pc: crates/ui-egui/src/dialogs/layer_style.rs:67 `MULTI`.
        matches!(self, EffectKind::Stroke | EffectKind::DropShadow | EffectKind::InnerShadow | EffectKind::ColorOverlay | EffectKind::GradientOverlay)
    }

    /// Whether a drag on the image moves this effect while its page is selected: Drop Shadow and
    /// Inner Shadow.
    pub fn drags_shadow(self) -> bool {
        // pc: crates/ui-egui/src/dialogs/layer_style.rs:498 `drags_shadow`.
        matches!(self, EffectKind::DropShadow | EffectKind::InnerShadow)
    }
}

/// An effect instance's id within one [`LayerStyle`]: `fxN` in PhotoCraft's dialog fields,
/// numbered from 1. New instances take one past the highest id in the style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EffectId(pub u32);

impl EffectId {
    /// The id's text, `fx{n}` (`fx3`).
    pub fn key(self) -> String {
        format!("fx{}", self.0)
    }

    /// The id written as `fx{n}`; `None` for any other text.
    pub fn from_key(key: &str) -> Option<EffectId> {
        key.strip_prefix("fx")?.parse().ok().map(EffectId)
    }
}

/// One value of an effect parameter or of a field the dialog does not model.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StyleValue {
    /// A slider or angle value in the field's unit (percent, pixels, degrees).
    Number(f64),
    /// A check box.
    Bool(bool),
    /// A choice's value key (`"softer"`), a blend mode's app key, a pattern id.
    Text(String),
    /// A color value.
    Color(Srgba),
    /// A contour value.
    Contour(Contour),
    /// Data the library never reads, in the app's own encoding (an imported multi-stop gradient,
    /// PSD descriptor data). It survives every edit unchanged.
    Other(String),
}

/// An effect's parameters by key (`"opacity"`, `"useGlobalLight"`), the field tables' keys plus
/// any key the app keeps that no field shows. Unknown keys survive every edit, so an apply never
/// drops what the dialog does not model (PhotoCraft kept an `fx` snapshot for this).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleParams(pub BTreeMap<String, StyleValue>);

impl StyleParams {
    /// The value at `key`.
    pub fn get(&self, key: &str) -> Option<&StyleValue> {
        self.0.get(key)
    }

    /// Sets `key` to `value`, keeping every other key.
    pub fn set(&mut self, key: impl Into<String>, value: StyleValue) {
        self.0.insert(key.into(), value);
    }

    /// The number at `key`; `None` when absent or not a number.
    pub fn number(&self, key: &str) -> Option<f64> {
        match self.0.get(key) {
            Some(StyleValue::Number(n)) => Some(*n),
            _ => None,
        }
    }

    /// The check at `key`; `None` when absent or not a check.
    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.0.get(key) {
            Some(StyleValue::Bool(b)) => Some(*b),
            _ => None,
        }
    }

    /// `self` over `defaults`: every key of `defaults` that `self` lacks is added (what a page
    /// shows: the factory defaults under the instance's values).
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:1010-1024 (defaults under the shown values).
    pub fn over(&self, defaults: &StyleParams) -> StyleParams {
        let mut merged = self.clone();
        for (key, value) in &defaults.0 {
            merged.0.entry(key.clone()).or_insert_with(|| value.clone());
        }
        merged
    }
}

/// One effect on the layer.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectInstance {
    #[allow(missing_docs)]
    pub id: EffectId,
    #[allow(missing_docs)]
    pub kind: EffectKind,
    /// The check box: whether the effect renders. Selecting a page does not change it.
    pub on: bool,
    /// The effect's parameter values.
    pub params: StyleParams,
}

/// The Knockout choice of Blending Options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Knockout {
    #[default]
    #[allow(missing_docs)]
    None,
    #[allow(missing_docs)]
    Shallow,
    #[allow(missing_docs)]
    Deep,
}

/// A Blend If slider's four handles in levels 0..=255: `[black low, black high, white low,
/// white high]`, always ordered black low ≤ black high ≤ white low ≤ white high. A handle pair
/// with equal halves shows as one handle; Alt-drag splits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlendIfRange(pub [u8; 4]);

impl Default for BlendIfRange {
    fn default() -> Self {
        BlendIfRange::FULL
    }
}

impl BlendIfRange {
    /// No blending range: `[0, 0, 255, 255]`.
    pub const FULL: BlendIfRange = BlendIfRange([0, 0, 255, 255]);

    /// `values` made ordered: each value raised to at least the one before it (PhotoCraft's
    /// `blend_quad` after reading).
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:1352-1365 `blend_quad`.
    pub fn ordered(values: [u8; 4]) -> BlendIfRange {
        let mut q = values;
        for i in 1..4 {
            q[i] = q[i].max(q[i - 1]);
        }
        BlendIfRange(q)
    }

    /// Handle `handle` (0..4) moved to `to`. `split` (Alt held): only that half moves. Otherwise
    /// the pair (0 and 1, or 2 and 3) moves by the same amount, keeping its spread, each half
    /// clamped to 0..=255. Then the order is restored by pushing: handles before `handle` are
    /// lowered to at most its value, handles after it raised to at least it, and each value
    /// raised to at least the one before.
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:1399-1430 (the drag in `blend_if_bar`).
    pub fn moved(self, handle: usize, to: u8, split: bool) -> BlendIfRange {
        let Some(&current) = self.0.get(handle) else {
            return self;
        };
        let mut n = self.0;
        if split {
            n[handle] = to;
        } else {
            let delta = i16::from(to) - i16::from(current);
            let pair = if handle < 2 { 0 } else { 2 };
            for v in &mut n[pair..pair + 2] {
                *v = u8::try_from((i16::from(*v) + delta).clamp(0, 255)).unwrap_or(*v);
            }
        }
        for i in 0..4 {
            if i < handle {
                n[i] = n[i].min(n[handle]);
            } else if i > handle {
                n[i] = n[i].max(n[handle]);
            }
        }
        BlendIfRange::ordered(n)
    }
}

/// The Blend If ranges of one channel entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlendIf {
    /// The host's channel entry: 0 is Gray in a colour document, then the colour channels in
    /// [`Blending::channels`] order; a one-channel document has only its own entry.
    pub channel: usize,
    /// The range applied to the layer's own pixels.
    pub this_layer: BlendIfRange,
    /// The range applied to the pixels beneath the layer.
    pub underlying: BlendIfRange,
}

/// The Blending Options page's values (Photoshop's General Blending, Advanced Blending and Blend
/// If).
#[derive(Clone, Debug, PartialEq)]
pub struct Blending {
    /// The layer's blend mode, the app's key (one of the host's `BlendChoice` keys; `"Normal"`
    /// by default, PhotoCraft's key).
    pub blend: String,
    /// Opacity in percent, 0..=100, whole.
    pub opacity: f32,
    /// Fill Opacity in percent, 0..=100, whole.
    pub fill_opacity: f32,
    /// One entry per colour channel of the document, in the host's channel-name order: true
    /// blends it (Photoshop's Channels check boxes).
    pub channels: Vec<bool>,
    #[allow(missing_docs)]
    pub knockout: Knockout,
    /// The five Advanced Blending checks, in Photoshop's order. Defaults: off, on, on, off, off.
    pub blend_interior_effects_as_group: bool,
    /// Advanced Blending: Blend Clipped Layers as Group.
    pub blend_clipped_layers_as_group: bool,
    /// Advanced Blending: Transparency Shapes Layer.
    pub transparency_shapes_layer: bool,
    /// Advanced Blending: Layer Mask Hides Effects.
    pub layer_mask_hides_effects: bool,
    /// Advanced Blending: Vector Mask Hides Effects.
    pub vector_mask_hides_effects: bool,
    /// One entry per Blend If channel (Gray first in a colour document).
    pub blend_if: Vec<BlendIf>,
}

impl Default for Blending {
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:1301 (the Advanced checks' defaults),
    // :1210-1241 `set_blending_fields` (an RGB document: three channels, Gray plus three Blend If entries).
    fn default() -> Self {
        Blending {
            blend: "Normal".into(),
            opacity: 100.0,
            fill_opacity: 100.0,
            channels: vec![true; 3],
            knockout: Knockout::None,
            blend_interior_effects_as_group: false,
            blend_clipped_layers_as_group: true,
            transparency_shapes_layer: true,
            layer_mask_hides_effects: false,
            vector_mask_hides_effects: false,
            blend_if: (0..4).map(|channel| BlendIf { channel, ..BlendIf::default() }).collect(),
        }
    }
}

/// A layer's style: its effect instances in the layer's order, the Blending Options, and the
/// document's shared light angle.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerStyle {
    /// The instances in render order. Several instances of one kind sit next to each other.
    pub effects: Vec<EffectInstance>,
    #[allow(missing_docs)]
    pub blending: Blending,
    /// The document's global light angle in degrees, which every effect with Use Global Light on
    /// shows and edits. The app compares it with the document's on OK.
    pub global_light: f32,
}

impl Default for LayerStyle {
    fn default() -> Self {
        // pc: crates/ui-egui/src/dialogs/layer_style.rs:464 (120° when the document has none).
        LayerStyle { effects: Vec::new(), blending: Blending::default(), global_light: 120.0 }
    }
}

impl LayerStyle {
    /// The instance with `id`.
    pub fn effect(&self, id: EffectId) -> Option<&EffectInstance> {
        self.effects.iter().find(|e| e.id == id)
    }

    /// The instance with `id`, to change.
    pub fn effect_mut(&mut self, id: EffectId) -> Option<&mut EffectInstance> {
        self.effects.iter_mut().find(|e| e.id == id)
    }

    /// The ids of `kind`'s instances, in order.
    pub fn instances(&self, kind: EffectKind) -> impl Iterator<Item = EffectId> + '_ {
        self.effects.iter().filter(move |e| e.kind == kind).map(|e| e.id)
    }

    /// A hash of everything that changes the render: the instances (order, on, parameters), the
    /// Blending Options and the light angle. Equal styles hash equally; floats hash by their bits
    /// with −0 as 0. The host keys its live preview on it.
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:730-735 `preview_hash` (FNV-1a over the fields
    // that change the rendered style).
    pub fn render_hash(&self) -> u64 {
        let mut h = Fnv::default();
        h.count(self.effects.len());
        for e in &self.effects {
            h.u64(u64::from(e.id.0));
            h.text(e.kind.key());
            h.byte(u8::from(e.on));
            h.count(e.params.0.len());
            for (key, value) in &e.params.0 {
                h.text(key);
                h.value(value);
            }
        }
        let b = &self.blending;
        h.text(&b.blend);
        h.f32(b.opacity);
        h.f32(b.fill_opacity);
        h.count(b.channels.len());
        for &on in &b.channels {
            h.byte(u8::from(on));
        }
        h.byte(match b.knockout {
            Knockout::None => 0,
            Knockout::Shallow => 1,
            Knockout::Deep => 2,
        });
        for on in [
            b.blend_interior_effects_as_group,
            b.blend_clipped_layers_as_group,
            b.transparency_shapes_layer,
            b.layer_mask_hides_effects,
            b.vector_mask_hides_effects,
        ] {
            h.byte(u8::from(on));
        }
        h.count(b.blend_if.len());
        for entry in &b.blend_if {
            h.count(entry.channel);
            h.bytes(&entry.this_layer.0);
            h.bytes(&entry.underlying.0);
        }
        h.f32(self.global_light);
        h.0
    }
}

/// The page the dialog shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayerStylePage {
    /// The style presets (PhotoCraft's `styles` page).
    Styles,
    /// Blending Options (PhotoCraft's `blendingOptions`).
    #[default]
    Blending,
    /// An effect instance's page.
    Effect(EffectId),
}

/// FNV-1a over the fields that change a render.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn byte(&mut self, b: u8) {
        self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
    }

    fn u64(&mut self, n: u64) {
        self.bytes(&n.to_le_bytes());
    }

    /// A length or index; lengths prefix every list so neighbouring lists cannot run together.
    fn count(&mut self, n: usize) {
        self.u64(n as u64);
    }

    fn text(&mut self, s: &str) {
        self.count(s.len());
        self.bytes(s.as_bytes());
    }

    fn f32(&mut self, x: f32) {
        self.bytes(&(if x == 0.0 { 0.0 } else { x }).to_bits().to_le_bytes());
    }

    fn f64(&mut self, x: f64) {
        self.u64((if x == 0.0 { 0.0 } else { x }).to_bits());
    }

    fn value(&mut self, v: &StyleValue) {
        match v {
            StyleValue::Number(n) => {
                self.byte(0);
                self.f64(*n);
            }
            StyleValue::Bool(b) => {
                self.byte(1);
                self.byte(u8::from(*b));
            }
            StyleValue::Text(s) => {
                self.byte(2);
                self.text(s);
            }
            StyleValue::Color(c) => {
                self.byte(3);
                for x in [c.r, c.g, c.b, c.a] {
                    self.f32(x);
                }
            }
            StyleValue::Contour(c) => {
                self.byte(4);
                self.text(&c.name);
                self.count(c.points.len());
                for p in &c.points {
                    self.f32(p.input);
                    self.f32(p.output);
                    self.byte(u8::from(p.corner));
                }
            }
            StyleValue::Other(s) => {
                self.byte(5);
                self.text(s);
            }
        }
    }
}
