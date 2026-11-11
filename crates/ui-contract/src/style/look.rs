//! Style parts: surface, text, icon and focus looks, the two colour types `Draw` accepts, and `ComponentStyle`.

use craft_behavior::Metrics;
use craft_i18n::Direction;
use craft_theme::{Mix, Stateful, TypeRole, Variant};
use egui::{Color32, CornerRadius, Stroke};

/// A colour a style builder picked from tokens. `Draw` accepts only this or `DataColor`. It has no
/// `From<Color32>`: the one constructor is `StyleCx::color`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyleColor(#[doc(hidden)] pub Color32);

impl StyleColor {
    /// The colour as egui paints it: a domain painter wraps it in `DataColor::new` for a document run, so it never holds
    /// a raw channel array.
    pub fn egui(self) -> Color32 {
        self.0
    }
}

impl Mix for StyleColor {
    fn mix(self, to: StyleColor, t: f32) -> StyleColor {
        StyleColor(self.0.mix(to.0, t))
    }
}

/// A colour from data (a swatch, a layer label dot).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DataColor(#[doc(hidden)] pub Color32);

impl DataColor {
    /// A data colour.
    pub const fn new(color: Color32) -> Self {
        DataColor(color)
    }
}

/// The surface a container or part sits on, which picks fill, edge, shadow and ink.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SurfaceKind {
    /// The window behind everything.
    Background,
    /// A canvas pasteboard.
    Canvas,
    /// Bars, title bars and rails.
    Chrome,
    /// Docked panels.
    Panel,
    /// Raised controls and cards.
    Raised,
    /// A raised surface while pressed.
    Pressed,
    /// Sunken areas.
    Sunken,
    /// Text and value fields.
    Well,
    /// A knob that rides on a track.
    Knob,
    /// Popups and menus.
    Popup,
    /// Dialogs.
    Dialog,
    /// Floating windows and panels.
    Floating,
    /// The scrim behind a modal layer.
    Scrim,
    /// An outlined box without fill.
    Frame,
}

impl SurfaceKind {
    /// The ink text and icons read on this surface.
    pub const fn ink(self) -> super::Ink {
        match self {
            SurfaceKind::Scrim => super::Ink::OnDark,
            _ => super::Ink::OnPanel,
        }
    }

    /// The surface content inside a box of kind `own` sits on: `own`, unless it paints no fill of its own (none, a
    /// frame, a scrim), where the content sits on `outer`.
    pub(crate) const fn under(own: Option<SurfaceKind>, outer: SurfaceKind) -> SurfaceKind {
        match own {
            None | Some(SurfaceKind::Frame | SurfaceKind::Scrim) => outer,
            Some(kind) => kind,
        }
    }
}

/// Which corners of a surface are rounded. `Start` and `End` mirror under right-to-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Corners {
    /// All four.
    #[default]
    All,
    /// The top two.
    Top,
    /// The two on the reading start side.
    Start,
    /// The two on the reading end side.
    End,
    /// None.
    None,
}

impl Corners {
    /// `radius` with the corners this value leaves square zeroed.
    pub fn apply(self, radius: CornerRadius, direction: Direction) -> CornerRadius {
        let left = (direction == Direction::Ltr) == (self == Corners::Start);
        match self {
            Corners::All => radius,
            Corners::None => CornerRadius::ZERO,
            Corners::Top => CornerRadius { sw: 0, se: 0, ..radius },
            Corners::Start | Corners::End if left => CornerRadius { ne: 0, se: 0, ..radius },
            Corners::Start | Corners::End => CornerRadius { nw: 0, sw: 0, ..radius },
        }
    }
}

/// A component's or part's surface.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SurfaceLook {
    /// The surface kind; `None` draws no surface (ghost buttons, list rows).
    pub kind: Option<SurfaceKind>,
    /// The kind while pressed; `None` keeps `kind`.
    pub kind_down: Option<SurfaceKind>,
    /// The state fill, which replaces the kind's fill.
    pub fill: Stateful<StyleColor>,
    /// The state border.
    pub border: Option<Stateful<StyleColor>>,
    /// The border width.
    pub border_width: f32,
    /// The corner radius.
    pub radius: CornerRadius,
    /// Which corners are rounded.
    pub corners: Corners,
    /// The border replaces the kind's flat edge instead of drawing over it, so a state whose border is transparent
    /// shows no frame. A bevel stays.
    pub border_replaces_edge: bool,
}

impl SurfaceLook {
    /// A look with `fill` and no kind, border or radius.
    pub fn new(fill: Stateful<StyleColor>) -> Self {
        SurfaceLook {
            kind: None,
            kind_down: None,
            fill,
            border: None,
            border_width: 0.0,
            radius: CornerRadius::ZERO,
            corners: Corners::All,
            border_replaces_edge: false,
        }
    }

    /// The surface kind in `state`: `kind_down` while pressed, else `kind`.
    pub fn kind_in(&self, state: &super::StateSet) -> Option<SurfaceKind> {
        if state.state == craft_theme::State::Pressed { self.kind_down.or(self.kind) } else { self.kind }
    }

    /// Sets the surface kind.
    pub fn kind(mut self, kind: SurfaceKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Sets the kind used while pressed.
    pub fn kind_down(mut self, kind: SurfaceKind) -> Self {
        self.kind_down = Some(kind);
        self
    }

    /// Sets the corner radius.
    pub fn radius(mut self, radius: CornerRadius) -> Self {
        self.radius = radius;
        self
    }

    /// Sets which corners are rounded.
    pub fn corners(mut self, corners: Corners) -> Self {
        self.corners = corners;
        self
    }

    /// The border replaces the kind's flat edge.
    pub fn border_replaces_edge(mut self) -> Self {
        self.border_replaces_edge = true;
        self
    }

    /// Sets the border colours and width.
    pub fn border(mut self, border: Stateful<StyleColor>, width: f32) -> Self {
        self.border = Some(border);
        self.border_width = width;
        self
    }
}

/// A component's text.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextLook {
    /// The type role.
    pub role: TypeRole,
    /// The text colour per state.
    pub color: Stateful<StyleColor>,
    /// The fill the text is painted on when that is not the component's surface fill.
    pub ground: Option<Stateful<StyleColor>>,
}

impl TextLook {
    /// A text look.
    pub fn new(role: TypeRole, color: Stateful<StyleColor>) -> Self {
        TextLook { role, color, ground: None }
    }

    /// Declares the fill the text is painted on, for a text that sits on a part's own fill.
    pub fn on(mut self, ground: Stateful<StyleColor>) -> Self {
        self.ground = Some(ground);
        self
    }
}

/// A component's icon.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct IconLook {
    /// The side of the icon box.
    pub side: f32,
    /// The icon colour per state.
    pub color: Stateful<StyleColor>,
    /// The fill the icon is painted on when that is not the component's surface fill.
    pub ground: Option<Stateful<StyleColor>>,
}

impl IconLook {
    /// An icon look.
    pub fn new(side: f32, color: Stateful<StyleColor>) -> Self {
        IconLook { side, color, ground: None }
    }

    /// Declares the fill the icon is painted on, for an icon that sits on a part's own fill.
    pub fn on(mut self, ground: Stateful<StyleColor>) -> Self {
        self.ground = Some(ground);
        self
    }
}

/// A value track: a groove with the part a value fills over it, and the marks and ticks along it. A slider, a
/// range slider, a thumb row and a progress bar paint the same track.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackLook {
    /// The groove: the part the value does not fill.
    pub unfilled: Stateful<StyleColor>,
    /// The part of the groove the value fills.
    pub filled: Stateful<StyleColor>,
    /// The edge around the groove, for a track drawn as a bar.
    pub edge: Stateful<StyleColor>,
    /// The width of the edge.
    pub edge_width: f32,
    /// The thickness of the groove.
    pub thickness: f32,
    /// The thickness of the groove when a gradient ramp is its face.
    pub ramp: f32,
    /// The corner radius of the groove and its fill.
    pub radius: CornerRadius,
    /// The length of a mark or tick below the groove.
    pub mark_length: f32,
    /// The thickness of a mark or tick.
    pub mark_width: f32,
    /// The colour of marks and ticks.
    pub mark_color: Stateful<StyleColor>,
    /// How near a mark, along the track, a drag snaps to it.
    pub snap: f32,
}

impl TrackLook {
    /// The groove across the middle of `rect`.
    pub fn groove(&self, rect: egui::Rect) -> egui::Rect {
        egui::Rect::from_center_size(rect.center(), egui::Vec2::new(rect.width(), self.thickness))
    }
}

/// A component's focus ring.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct FocusLook {
    /// The ring stroke.
    pub stroke: Stroke,
    /// How far the ring sits outside the shape.
    pub offset: f32,
}

impl FocusLook {
    /// A focus look.
    pub const fn new(stroke: Stroke, offset: f32) -> Self {
        FocusLook { stroke, offset }
    }
}

/// What `#[derive(ComponentStyle)]` writes for a component's `Style` type.
pub trait ComponentStyle: Clone + PartialEq + Send + Sync + 'static {
    /// The variants the component accepts; the first is the default.
    const VARIANTS: &'static [Variant] = &[];

    /// The component's surface, if it has one.
    fn surface(&self) -> Option<&SurfaceLook> {
        None
    }

    /// The component's focus ring, if it shows one.
    fn focus(&self) -> Option<&FocusLook> {
        None
    }

    /// The component's text looks, in field order.
    fn texts(&self) -> Vec<&TextLook> {
        Vec::new()
    }

    /// The component's icon looks, in field order.
    fn icons(&self) -> Vec<&IconLook> {
        Vec::new()
    }

    /// The component's layout metrics.
    fn metrics(&self) -> Metrics;
}
