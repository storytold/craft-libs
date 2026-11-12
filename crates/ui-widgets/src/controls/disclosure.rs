//! `Disclosure`: the open/closed toggle of a section, a title row's lead, a tree row's expander and a row's trailing
//! disclosure.

use crate::contract::{
    Animate as _, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, PaintUi, Pointer as _, Role, Styled as _, spec,
};
use crate::primitives::{Glyph, PaintShape, StrokeWidth};
use crate::style::{ComponentStyle, IconLook, StyleCx, UiTokens, ink_color, rgba_color};
use craft_behavior::{Metrics, Sense};
use craft_i18n::Direction;
use egui::emath::GuiRounding as _;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

/// A quarter turn: closed points to the end, open points down.
const QUARTER_TURN: f32 = std::f32::consts::FRAC_PI_2;
/// The chevron's arms, as a share of the glyph rect's side: it spans two fifths of the side along the arrow and a
/// fifth across.
const ARM: f32 = 0.1;
/// The submenu arrow's base and depth, as shares of the glyph rect's side.
const ARROW_BASE: f32 = 0.55;
const ARROW_DEPTH: f32 = 0.35;
/// The disclosure triangle's base and depth, as shares of the glyph rect's side.
const TRIANGLE_BASE: f32 = 0.52;
const TRIANGLE_DEPTH: f32 = 0.31;

/// Which arrow a [`DisclosureGlyph`] draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArrowForm {
    /// The filled triangle of everything that opens or unfolds in place: a section, a tree row, a row's trailing
    /// disclosure.
    Disclosure,
    /// The larger filled arrow at the end of a menu row that opens a submenu.
    Submenu,
    /// The stroked chevron after the text of a field, a picker or a button that opens a menu.
    Dropdown,
}

/// The arrow every component shows for something that opens or unfolds. The disclosure is a filled triangle that
/// turns from pointing to the end of the line (closed) to pointing down (open) as `open` goes from 0 to 1; a
/// dropdown's arrow is the open chevron; a submenu's is the filled arrow to the end of the line. Components name these
/// through [`DisclosureGlyph::new`], [`DisclosureGlyph::dropdown`], [`DisclosureGlyph::menu_button`] and
/// [`DisclosureGlyph::submenu`] and never an icon of their own.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DisclosureGlyph {
    /// The turn, 0 closed to 1 open.
    pub open: f32,
    /// Which arrow it is.
    pub form: ArrowForm,
    /// The dropdown chevron's line width: a field's or picker's arrow is a fine line, a button's a firmer one.
    pub stroke: StrokeWidth,
}

/// The arrow of a field or picker.
static DROPDOWN: DisclosureGlyph = DisclosureGlyph::dropdown_form();
/// The arrow of a button that opens a menu.
static MENU_BUTTON: DisclosureGlyph = DisclosureGlyph { open: 1.0, form: ArrowForm::Dropdown, stroke: StrokeWidth::Medium };
/// The submenu arrow.
static SUBMENU: DisclosureGlyph = DisclosureGlyph { open: 0.0, form: ArrowForm::Submenu, stroke: StrokeWidth::Thin };

impl DisclosureGlyph {
    /// The disclosure triangle `open` of the way turned.
    pub const fn new(open: f32) -> Self {
        DisclosureGlyph { open, form: ArrowForm::Disclosure, stroke: StrokeWidth::Thin }
    }

    /// The dropdown chevron in the fine line, for a component that draws the arrow's path itself.
    pub(crate) const fn dropdown_form() -> Self {
        DisclosureGlyph { open: 1.0, form: ArrowForm::Dropdown, stroke: StrokeWidth::Thin }
    }

    /// Whether the arrow is a filled shape rather than a stroked chevron.
    const fn filled(&self) -> bool {
        !matches!(self.form, ArrowForm::Dropdown)
    }

    /// The icon of the arrow after the text of a picker's face or a field with a list.
    pub fn dropdown() -> crate::primitives::IconSpec<'static> {
        crate::primitives::IconSpec::glyph(&DROPDOWN)
    }

    /// The icon of the arrow after the text of a button that opens a menu: the dropdown arrow in the firmer line.
    pub fn menu_button() -> crate::primitives::IconSpec<'static> {
        crate::primitives::IconSpec::glyph(&MENU_BUTTON)
    }

    /// The icon of the arrow at the end of a menu row that opens a submenu.
    pub fn submenu() -> crate::primitives::IconSpec<'static> {
        crate::primitives::IconSpec::glyph(&SUBMENU)
    }

    /// The outline in `rect`, reading in `direction`: a closed glyph points to the end of the line.
    pub fn path(&self, rect: Rect, direction: Direction) -> Vec<Pos2> {
        self.outline(rect, direction == Direction::Rtl)
    }

    fn outline(&self, rect: Rect, rtl: bool) -> Vec<Pos2> {
        let side = rect.width().min(rect.height());
        let turn = self.open * QUARTER_TURN * if rtl { -1.0 } else { 1.0 };
        let (sin, cos) = turn.sin_cos();
        let flip = if rtl { -1.0 } else { 1.0 };
        let center = rect.center();
        let points: [Vec2; 3] = if self.filled() {
            let (base, depth) = if self.form == ArrowForm::Submenu { (ARROW_BASE, ARROW_DEPTH) } else { (TRIANGLE_BASE, TRIANGLE_DEPTH) };
            let (half, depth) = (side * base / 2.0, side * depth / 2.0);
            [Vec2::new(-depth, -half), Vec2::new(depth, 0.0), Vec2::new(-depth, half)]
        } else {
            let arm = side * ARM;
            [Vec2::new(-arm, -arm * 2.0), Vec2::new(arm, 0.0), Vec2::new(-arm, arm * 2.0)]
        };
        points
            .map(|point| {
                let point = Vec2::new(point.x * flip, point.y);
                center + Vec2::new(point.x * cos - point.y * sin, point.x * sin + point.y * cos)
            })
            .to_vec()
    }
}

impl DisclosureGlyph {
    /// The side of the square the glyph is drawn in, from the tokens and never from the slot it is placed in: the
    /// icon side for the disclosure and the chevron (a `Disclosure` at the standard size), the menu text size for the
    /// submenu arrow. Every placement shows the same shape.
    fn side(&self, tokens: &UiTokens) -> f32 {
        if self.form == ArrowForm::Submenu { tokens.typography.body.size } else { tokens.size.icon.md }
    }

    /// The glyph's colour: the muted text colour in the place of the normal one. A state that recolours the ink (a
    /// disabled control, a selected row, text on a fill) keeps its colour. The submenu arrow reads in its row's text
    /// colour.
    fn tone(&self, tokens: &UiTokens, ink: Color32) -> Color32 {
        let quiet = self.form != ArrowForm::Submenu;
        if quiet && ink == ink_color(tokens, crate::style::Ink::OnPanel) { rgba_color(tokens.color.text.muted) } else { ink }
    }
}

impl Glyph for DisclosureGlyph {
    fn directional(&self) -> bool {
        self.open < 1.0
    }

    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, color: Color32, mirrored: bool) {
        let rect = Rect::from_center_size(rect.center(), Vec2::splat(self.side(tokens)));
        let mut points = self.outline(rect, mirrored);
        let color = self.tone(tokens, color);
        if self.filled() {
            if self.open == 0.0 {
                // The base is the arrow's vertical edge: on a pixel boundary it paints no half-covered column.
                let x = points[0].x.round_to_pixels(painter.pixels_per_point());
                points[0].x = x;
                points[2].x = x;
            }
            painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
        } else {
            let width = self.stroke.points(tokens, painter.pixels_per_point());
            painter.add(Shape::line(points, Stroke::new(width, color)));
        }
    }
}

/// The style of a disclosure.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct DisclosureStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The triangle.
    #[style(icon)]
    glyph: IconLook,
}

/// A button bound to an open state, with the expanded state. A face shown by a title row or a tree row has no node.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Disclosure<'a> {
    #[common]
    common: Common,
    open: Bind<'a, bool>,
    /// Whether the disclosure is its own AccessKit node; a part of another component's node turns it off.
    #[prop]
    node: bool,
}

impl<'a> Disclosure<'a> {
    /// A disclosure of `open`.
    pub fn new(open: Bind<'a, bool>) -> Self {
        Disclosure { common: Common::new(), open, node: true }
    }
}

impl Component for Disclosure<'_> {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    type Style = DisclosureStyle;
    type Output = EditResponse<bool>;

    fn style(cx: &StyleCx<'_>, _: DisclosureStyleKey) -> DisclosureStyle {
        let side = cx.control().icon;
        DisclosureStyle { metrics: Metrics::new().min_size(Vec2::splat(side)), glyph: IconLook::new(side, cx.quiet_ink()) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.icon())
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn bind(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if cx.activation().is_some() {
            let open = cx.value().copied().unwrap_or(false);
            cx.set(!open);
        }
    }

    fn access(&self, access: &mut crate::contract::Access, cx: &Cx<'_, Self>) {
        if !self.node {
            access.role(Role::None);
        }
        access.expanded(cx.value().copied().unwrap_or(false));
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let open = cx.anim_bool("open", cx.value().copied().unwrap_or(false), spec::OPEN_RISE);
        let rect = Rect::from_center_size(cx.rect.center(), Vec2::splat(cx.style.glyph.side));
        let points = DisclosureGlyph::new(open).path(rect, cx.env.direction());
        ui.draw().fill(&PaintShape::Polygon(points), cx.resolve(&cx.style.glyph.color));
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<bool> {
        cx.edit_response()
    }
}
