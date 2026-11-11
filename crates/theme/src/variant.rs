//! Variants: emphasis by tone, custom variants for one component's style key, and the look both resolve to.
//! Egui-free; a component's style turns a [`VariantLook`] into style parts.

use crate::color::Rgba;
use crate::parts::States;
use crate::tokens::{ColorToken, Tokens};
use std::any::TypeId;
use std::hash::{Hash, Hasher};

/// How strongly a variant stands out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Emphasis {
    /// A solid tone fill.
    Solid,
    /// A tinted fill with a tone edge and text.
    Soft,
    /// An outline, filled only by state.
    Outline,
    /// No fill or edge until a state fills it.
    Ghost,
}

/// What a variant says about its subject. The one tone type of the component layer: messages, notices, badges and
/// status ink all name it, and `StyleCx::tone` and `StyleCx::tone_fill` give its colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Tone {
    /// No status.
    #[default]
    Neutral,
    /// The accent.
    Accent,
    /// Information.
    Info,
    /// Success.
    Good,
    /// Needs attention.
    Warn,
    /// Failure.
    Bad,
}

/// Which radius of the theme's shape a look uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RadiusStep {
    /// `radius.control`.
    #[default]
    Control,
    /// `radius.button`.
    Button,
    /// `radius.pill`.
    Pill,
}

/// The colours and shape of a variant. It holds no padding or width, so a variant cannot change layout.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct VariantLook {
    /// Background per state.
    pub fill: States<Rgba>,
    /// Label and icon per state.
    pub text: States<Rgba>,
    /// Edge per state; `None` draws no edge.
    pub edge: Option<States<Rgba>>,
    /// The corner radius step.
    pub radius: RadiusStep,
}

/// The colour tokens a tone's variants take their fills and inks from: the one place the variant table reads.
#[derive(Clone, Copy, Debug)]
struct ToneTokens {
    soft_fill: ColorToken,
    fg: ColorToken,
    edge: ColorToken,
    solid: ColorToken,
    solid_hover: ColorToken,
    solid_pressed: ColorToken,
    on_solid: ColorToken,
}

impl ToneTokens {
    fn of(tone: Tone) -> Self {
        use ColorToken as T;
        let status = |fg, bg, edge, solid, solid_hover, solid_pressed| ToneTokens {
            soft_fill: bg,
            fg,
            edge,
            solid,
            solid_hover,
            solid_pressed,
            on_solid: T::TextOnStatus,
        };
        match tone {
            Tone::Neutral => ToneTokens {
                soft_fill: T::SurfaceSunken,
                fg: T::TextNormal,
                edge: T::TextMuted,
                solid: T::TextMuted,
                solid_hover: T::TextMuted,
                solid_pressed: T::TextNormal,
                on_solid: T::SurfacePanel,
            },
            Tone::Accent => ToneTokens {
                soft_fill: T::AccentSoft,
                fg: T::AccentText,
                edge: T::AccentBorder,
                solid: T::AccentStrong,
                solid_hover: T::AccentStrongHover,
                solid_pressed: T::AccentStrongPressed,
                on_solid: T::TextOnStrong,
            },
            Tone::Info => {
                status(T::StatusInfoFg, T::StatusInfoBg, T::StatusInfoBorder, T::StatusInfoSolid, T::StatusInfoSolidHover, T::StatusInfoSolidPressed)
            }
            Tone::Good => {
                status(T::StatusGoodFg, T::StatusGoodBg, T::StatusGoodBorder, T::StatusGoodSolid, T::StatusGoodSolidHover, T::StatusGoodSolidPressed)
            }
            Tone::Warn => {
                status(T::StatusWarnFg, T::StatusWarnBg, T::StatusWarnBorder, T::StatusWarnSolid, T::StatusWarnSolidHover, T::StatusWarnSolidPressed)
            }
            Tone::Bad => {
                status(T::StatusBadFg, T::StatusBadBg, T::StatusBadBorder, T::StatusBadSolid, T::StatusBadSolidHover, T::StatusBadSolidPressed)
            }
        }
    }
}

/// The (ink, fill) token pairs of every soft and solid variant, in the variant table's own terms.
pub(crate) fn variant_pairs() -> Vec<(ColorToken, ColorToken)> {
    let mut pairs = Vec::new();
    for tone in [Tone::Neutral, Tone::Accent, Tone::Info, Tone::Good, Tone::Warn, Tone::Bad] {
        let t = ToneTokens::of(tone);
        pairs.extend([(t.fg, t.soft_fill), (t.on_solid, t.solid), (t.on_solid, t.solid_hover), (t.on_solid, t.solid_pressed)]);
    }
    pairs
}

impl VariantLook {
    /// A look with no edge and control corners.
    pub const fn new(fill: States<Rgba>, text: States<Rgba>) -> Self {
        VariantLook { fill, text, edge: None, radius: RadiusStep::Control }
    }

    /// Sets the edge colours.
    pub const fn edge(mut self, edge: States<Rgba>) -> Self {
        self.edge = Some(edge);
        self
    }

    /// Sets the corner radius step.
    pub const fn radius(mut self, radius: RadiusStep) -> Self {
        self.radius = radius;
        self
    }

    /// The look of `emphasis` in `tone`, from the resolved tokens.
    pub fn of(emphasis: Emphasis, tone: Tone, tokens: &Tokens) -> VariantLook {
        let c = &tokens.color;
        let clear = Rgba::TRANSPARENT;
        let flat = |color: Rgba| States::new(clear, clear, clear, color);
        let text = |color: Rgba| States::new(color, color, color, c.text.disabled);
        let state_fill = States::new(clear, c.state.hover, c.state.pressed, clear);
        let t = ToneTokens::of(tone);
        let (soft_fill, fg, edge, solid, solid_hover, solid_pressed, on_solid) =
            (c.get(t.soft_fill), c.get(t.fg), c.get(t.edge), c.get(t.solid), c.get(t.solid_hover), c.get(t.solid_pressed), c.get(t.on_solid));
        let button = RadiusStep::Button;
        match emphasis {
            Emphasis::Solid => VariantLook::new(States::new(solid, solid_hover, solid_pressed, c.state.disabled_fill), text(on_solid)).radius(button),
            Emphasis::Soft => VariantLook::new(States::new(soft_fill, soft_fill, soft_fill, c.state.disabled_fill), text(fg))
                .edge(States::new(edge, edge, edge, c.line.divider))
                .radius(button),
            // A neutral outline darkens to the strongest ink under the pointer and the press; a toned one keeps its edge.
            Emphasis::Outline => {
                let lit = if tone == Tone::Neutral { c.text.normal } else { edge };
                VariantLook::new(state_fill, text(fg)).edge(States::new(edge, lit, lit, c.line.divider)).radius(button)
            }
            Emphasis::Ghost => VariantLook::new(state_fill, text(fg)).radius(button).edge(flat(clear)),
        }
    }
}

/// A look for one component's style key, built in a `const`. It describes itself; nothing registers it.
#[derive(Clone, Copy)]
pub struct CustomVariant {
    id: &'static str,
    look: fn(&Tokens) -> VariantLook,
    for_key: fn() -> TypeId,
}

fn type_id_of<K: 'static>() -> TypeId {
    TypeId::of::<K>()
}

impl CustomVariant {
    /// A variant with id `"crate.name"` (a const panic without a crate prefix) whose look is `look`. `K` is the
    /// component's style key type.
    pub const fn new<K: 'static>(id: &'static str, look: fn(&Tokens) -> VariantLook) -> Self {
        let bytes = id.as_bytes();
        let mut i = 1;
        let mut prefixed = false;
        while i + 1 < bytes.len() {
            if bytes[i] == b'.' {
                prefixed = true;
            }
            i += 1;
        }
        assert!(prefixed, "a custom variant id is `crate.name`");
        CustomVariant { id, look, for_key: type_id_of::<K> }
    }

    /// The id, `"crate.name"`.
    pub const fn id(&self) -> &'static str {
        self.id
    }

    /// The look in `tokens`.
    pub fn look(&self, tokens: &Tokens) -> VariantLook {
        (self.look)(tokens)
    }

    /// The `TypeId` of the style key the variant is for.
    pub fn key_type(&self) -> TypeId {
        (self.for_key)()
    }
}

impl PartialEq for CustomVariant {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for CustomVariant {}

impl Hash for CustomVariant {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl std::fmt::Debug for CustomVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CustomVariant({})", self.id)
    }
}

/// An emphasis in a tone, or a custom variant. Equal and hashed by id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Variant {
    /// A built-in look.
    #[non_exhaustive]
    Look {
        /// How strongly it stands out.
        emphasis: Emphasis,
        /// What it says.
        tone: Tone,
    },
    /// A look written for one component.
    Custom(CustomVariant),
}

impl Variant {
    /// Solid accent.
    pub const PRIMARY: Variant = Variant::look(Emphasis::Solid, Tone::Accent);
    /// Outline neutral.
    pub const SECONDARY: Variant = Variant::look(Emphasis::Outline, Tone::Neutral);
    /// Ghost neutral.
    pub const GHOST: Variant = Variant::look(Emphasis::Ghost, Tone::Neutral);
    /// Solid bad.
    pub const DANGER: Variant = Variant::look(Emphasis::Solid, Tone::Bad);

    /// A built-in variant; the constructor outside this crate, where `Look` cannot be built directly.
    pub const fn look(emphasis: Emphasis, tone: Tone) -> Variant {
        Variant::Look { emphasis, tone }
    }

    /// The look in `tokens`.
    pub fn resolve(&self, tokens: &Tokens) -> VariantLook {
        match self {
            Variant::Look { emphasis, tone } => VariantLook::of(*emphasis, *tone, tokens),
            Variant::Custom(custom) => custom.look(tokens),
        }
    }
}
