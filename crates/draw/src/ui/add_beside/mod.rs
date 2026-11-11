//! [`AddBesideHandles`]: the "+" buttons beside the four sides of a selected object, which add an artboard
//! (Photoshop) or a connected object (FigJam) there. One canvas control for the Artboard tool and the connector
//! tools: its buttons are parts in the canvas's arena, so a press on one reaches the control before the tool.

#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod words;

use craft_behavior::kurbo::Rect as DocRect;
use craft_behavior::{CanvasView, CursorIcon, HitLevel, HitPriority, HitShape, Metrics, NavShape, PartKey, PointerButton, PressPhase, Roving, Sense};
use craft_ui::canvas::canvas_space;
use craft_ui::contract::{
    Access, Common, Component, Cx, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{FocusKind, FocusRing, PaintShape, StrokeWidth};
use craft_ui::runtime::commands::FireSource;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, KeyUse};
use craft_ui::style::{ComponentStyle, FocusLook, StyleColor, StyleCx};
use egui::{Pos2, Rect, Response, Vec2};

pub use words::AddBesideWords;

/// The button on one side: `BUTTON.at(i)` with `i` the side's place in [`AddSide::ALL`].
const BUTTON: PartKey = PartKey::new("button");

/// The least cosine between an arrow's direction and the way to a button for the arrow to reach it.
const ARROW_ALIGNMENT: f32 = 0.5;

/// The plus's arm as a share of the button's radius.
const PLUS_ARM_SHARE: f32 = 0.5;

/// A side of the object, in document space (never mirrored right to left).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AddSide {
    /// The top side.
    Top,
    /// The right side.
    Right,
    /// The bottom side.
    Bottom,
    /// The left side.
    Left,
}

impl AddSide {
    /// Clockwise from the top: the buttons' keyboard order.
    pub const ALL: [AddSide; 4] = [AddSide::Top, AddSide::Right, AddSide::Bottom, AddSide::Left];

    fn index(self) -> u64 {
        match self {
            AddSide::Top => 0,
            AddSide::Right => 1,
            AddSide::Bottom => 2,
            AddSide::Left => 3,
        }
    }

    fn at(index: u64) -> Option<AddSide> {
        AddSide::ALL.get(usize::try_from(index).ok()?).copied()
    }
}

/// The style of the "+" buttons: where they sit, and the disc and plus in rest and hot looks.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct AddBesideStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// From the object's edge to a button's centre, screen points.
    offset: f32,
    /// A button's radius.
    radius: f32,
    /// The plus and the ring.
    accent: StyleColor,
    /// The disc.
    fill: StyleColor,
    /// A hovered or focused disc.
    hot_fill: StyleColor,
    /// The plus on a hovered or focused disc.
    hot_plus: StyleColor,
}

/// The screen centres of the four buttons around `object` (document units) in [`AddSide::ALL`] order: each on the
/// middle of its side, `offset` screen points outside the edge, through `view`, so a turned view turns the buttons
/// with the object. A button of a degenerate object lies on the object's middle. Hiding is [`AddBesideHandles`]'.
pub fn add_beside_places(view: &CanvasView, object: DocRect, offset: f32) -> [(AddSide, Pos2); 4] {
    let [top_left, top_right, bottom_right, bottom_left] = view.quad(object.abs());
    let centre = top_left.lerp(bottom_right, 0.5);
    let middles =
        [top_left.lerp(top_right, 0.5), top_right.lerp(bottom_right, 0.5), bottom_right.lerp(bottom_left, 0.5), bottom_left.lerp(top_left, 0.5)];
    let mut places = AddSide::ALL.map(|side| (side, centre));
    for (place, middle) in places.iter_mut().zip(middles) {
        let out = (middle - centre).normalized();
        place.1 = if out.is_finite() { middle + out * offset } else { middle };
    }
    places
}

/// What [`AddBesideHandles`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct AddBesideOutput {
    /// The group's response.
    pub response: Response,
    /// A button was clicked, or Enter or Space pressed on a focused button.
    pub added: Option<AddSide>,
    /// Alt was held with the click: add with contents (an artboard's copy). A key press never asks for a copy.
    pub copy: bool,
    /// The button under the pointer or focused (the tool draws its ghost).
    pub hovered: Option<AddSide>,
}

impl Outcome for AddBesideOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        let mut facts = craft_ui::testing::StoryOutput::default();
        facts.id = Some(self.response.id);
        facts.clicked = self.added.is_some();
        facts
    }
}

/// The "+" buttons beside the four sides of `object` (document units).
///
/// - Place: each centred on its side, [`AddBesideStyle`]'s offset outside the edge, through the view (a turned view
///   turns the sides with the object); see [`add_beside_places`].
/// - Hidden: all of them when the object on screen is smaller than two radii on a side, or [`hidden`](Self::hidden)
///   is set (a drag runs); one when its disc touches a [`clear_of`](Self::clear_of) rectangle (a size label, a
///   toolbar) or leaves the canvas.
/// - Pointer: the pointing hand; a click reports [`AddBesideOutput::added`], with Alt held also `copy`; the hot look
///   (the accent disc) on hover and focus.
/// - Keyboard: the group is one Tab stop named by the label (the "Add a connected shape" word after
///   [`connecting`](Self::connecting)); the arrows move the focus to the button that lies in their screen
///   direction, Enter or Space adds; each button is a `Button` node ("Add above", ...).
#[must_use = "show the buttons with `show_at`"]
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct AddBesideHandles<'a> {
    #[common]
    common: Common,
    object: DocRect,
    /// Screen rectangles no button may cover.
    #[prop]
    clear_of: &'a [Rect],
    /// Hides every button (while a drag runs).
    #[prop(flag)]
    hidden: bool,
    /// Names the group for adding a connected shape, as the connector tools do.
    #[prop(flag)]
    connecting: bool,
    report: Report,
}

#[derive(Default)]
struct Report {
    added: Option<AddSide>,
    copy: bool,
    hovered: Option<AddSide>,
}

impl<'a> AddBesideHandles<'a> {
    /// The buttons around `object`, in document units, the group named `label`.
    pub fn new(label: impl Into<Label>, object: DocRect) -> Self {
        AddBesideHandles { common: Common::new(), object, clear_of: &[], hidden: false, connecting: false, report: Report::default() }.label(label)
    }

    /// The visible buttons' screen centres.
    fn visible(&self, view: &CanvasView, style: &AddBesideStyle, canvas: Rect) -> Vec<(AddSide, Pos2)> {
        let [top_left, top_right, _, bottom_left] = view.quad(self.object.abs());
        let finite = [top_left, top_right, bottom_left].iter().all(|p| p.x.is_finite() && p.y.is_finite());
        // Side lengths, not the bounds: a turned view keeps the test the same.
        let small = (top_right - top_left).length() < 2.0 * style.radius || (bottom_left - top_left).length() < 2.0 * style.radius;
        if self.hidden || small || !finite {
            return Vec::new();
        }
        let inside = canvas.shrink(style.radius);
        add_beside_places(view, self.object, style.offset)
            .into_iter()
            .filter(|&(_, at)| inside.contains(at) && !self.clear_of.iter().any(|rect| rect.expand(style.radius).contains(at)))
            .collect()
    }
}

impl Component for AddBesideHandles<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS.with_activate(ActivateKeys::Both).with_moves().with_chrome_key(false);
    const OVERLAYS: bool = false;
    type Style = AddBesideStyle;
    type Output = AddBesideOutput;

    fn style(cx: &StyleCx<'_>, _: AddBesideHandlesStyleKey) -> AddBesideStyle {
        let tokens = cx.tokens;
        AddBesideStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            offset: tokens.space.gap.section,
            radius: tokens.shape.radius.surface,
            accent: cx.color(tokens.color.accent.solid),
            fill: cx.color(tokens.color.surface.raised),
            hot_fill: cx.color(tokens.color.accent.solid),
            hot_plus: cx.color(tokens.color.text.on_strong),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// The visible buttons: click parts at handle priority in one roving group, in screen directions.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let (view, radius) = (space.view, cx.style.radius);
        let buttons = self.visible(&view, cx.style, rect);
        for &(side, at) in &buttons {
            cx.part(BUTTON.at(side.index()), HitShape::circle(at, radius))
                .sense(Sense::CLICK | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .cursor(CursorIcon::PointingHand)
                .focusable()
                .nudges();
        }
        if !buttons.is_empty() {
            cx.roving(Roving::new(NavShape::Spatial));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let focused = cx.active_part().filter(|part| part.key == BUTTON).and_then(|part| part.index).and_then(AddSide::at);
        for side in AddSide::ALL {
            let Some(result) = cx.part_if_declared(BUTTON.at(side.index())) else { continue };
            if result.hovered {
                self.report.hovered = Some(side);
            }
            if let PressPhase::Release { start, click: true, .. } = result.press.phase
                && start.button == PointerButton::Primary
            {
                self.report.added = Some(side);
                self.report.copy = result.press.mods.alt;
            } else if self.report.added.is_none() && matches!(cx.part_activated(BUTTON.at(side.index())), Some(FireSource::Key | FireSource::Access))
            {
                self.report.added = Some(side);
            }
        }
        if self.report.hovered.is_none() {
            self.report.hovered = focused;
        }
        // The arrows move the focus to the button that lies in their screen direction.
        if let Some(current) = focused
            && let Some(space) = canvas_space(&cx.env)
            && let Some((direction, _)) = cx.intents().take_nudge()
        {
            let places = self.visible(&space.view, cx.style, cx.rect);
            let ahead = direction.normalized();
            let from = places.iter().find(|(side, _)| *side == current).map(|&(_, at)| at);
            let next = from.and_then(|from| {
                places
                    .iter()
                    .filter(|(side, _)| *side != current)
                    .map(|&(side, at)| (side, (at - from).normalized().dot(ahead)))
                    .filter(|&(_, alignment)| alignment > ARROW_ALIGNMENT)
                    .max_by(|a, b| a.1.total_cmp(&b.1))
            });
            if let Some((side, _)) = next {
                cx.focus_part(BUTTON.at(side.index()));
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let style = &cx.style;
        let focused = cx.active_part().filter(|part| part.key == BUTTON).and_then(|part| part.index).and_then(AddSide::at);
        let ring = cx.focus_visible();
        let buttons = self.visible(&space.view, style, cx.rect);
        let draw = ui.draw();
        for &(side, at) in &buttons {
            let hot = self.report.hovered == Some(side) || focused == Some(side);
            let (fill, plus) = if hot { (style.hot_fill, style.hot_plus) } else { (style.fill, style.accent) };
            let disc = PaintShape::Circle(at, style.radius);
            draw.fill(&disc, fill);
            draw.stroke(&disc, StrokeWidth::Thin, style.accent);
            let arm = style.radius * PLUS_ARM_SHARE;
            draw.stroke(&PaintShape::Path(vec![at - Vec2::new(arm, 0.0), at + Vec2::new(arm, 0.0)]), StrokeWidth::Medium, plus);
            draw.stroke(&PaintShape::Path(vec![at - Vec2::new(0.0, arm), at + Vec2::new(0.0, arm)]), StrokeWidth::Medium, plus);
        }
        let Some(&(_, at)) = buttons.iter().find(|(side, _)| ring && focused == Some(*side)) else { return };
        let painter = ui.content_painter(cx);
        let ppp = painter.pixels_per_point();
        painter.painter.extend(FocusRing::shapes(&style.focus, FocusKind::Halo, &PaintShape::Circle(at, style.radius), ppp));
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let words = cx.words::<AddBesideWords>();
        if self.connecting {
            access.name(Name::Text(words.add_connected.as_str().to_owned()));
        }
        for (side, _) in self.visible(&space.view, cx.style, cx.rect) {
            let name = match side {
                AddSide::Top => &words.above,
                AddSide::Right => &words.right,
                AddSide::Bottom => &words.below,
                AddSide::Left => &words.left,
            };
            access.node(BUTTON.at(side.index()), Role::BUTTON).name = Name::Text(name.as_str().to_owned());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> AddBesideOutput {
        let Report { added, copy, hovered } = self.report;
        AddBesideOutput { response: cx.response.clone(), added, copy, hovered }
    }
}
