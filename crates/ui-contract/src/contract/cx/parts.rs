//! `PartsCx` and `PartBuilder`: step 7's part declarations.

use super::measure::MeasureCx;
use super::scroll::{ScrollDecl, THUMB, thumb_covers_centre};
use crate::contract::{Component, Tooltip};
use crate::primitives::sensing::PartDecl;
use crate::runtime::focus::FocusShape;
use craft_behavior::{Axis, Cursor, CursorIcon, Flow, HitLevel, HitPriority, HitShape, PartKey, PartRef, Roving, ScrollSpan, Sense};
use craft_theme::Flags;
use egui::{Id, Rect};
use std::fmt::Debug;
use std::ops::Deref;

/// A part that opens and closes a popup of its component, from `PartBuilder::toggles`.
#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct Toggle {
    pub part: PartRef,
    pub decl: crate::contract::overlay::ToggleDecl,
    /// The popup's open state after step 9's toggles, which the part's sub-node reports as `expanded`.
    pub open: bool,
}

/// A part as `PartsCx::part` declares it; the builder sets its flags.
pub struct PartBuilder<'a> {
    pub(in crate::contract) decl: &'a mut PartDecl,
    pub(in crate::contract) declared: &'a mut PartExtras,
    /// The context and component id the exclusive list is written under; none in a sizing pass, which declares
    /// the same parts again in the real one.
    pub(in crate::contract) window: Option<(&'a egui::Context, Id)>,
}

/// The parts a component declared, each with what it declares beyond the sensing core's `PartDecl`. The two lists
/// stand side by side, so the sensing step borrows the `PartDecl`s as they are.
#[derive(Default)]
pub(crate) struct Decls {
    parts: Vec<PartDecl>,
    extras: Vec<PartExtras>,
}

impl Decls {
    fn push(&mut self, decl: PartDecl) -> (&mut PartDecl, &mut PartExtras) {
        let at = self.parts.len();
        self.parts.push(decl);
        self.extras.push(PartExtras::default());
        (&mut self.parts[at], &mut self.extras[at])
    }

    /// The sensing core's declarations, in declaration order.
    pub(crate) fn parts(&self) -> &[PartDecl] {
        &self.parts
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    pub(crate) fn iter(&self) -> std::iter::Zip<std::slice::Iter<'_, PartDecl>, std::slice::Iter<'_, PartExtras>> {
        self.parts.iter().zip(&self.extras)
    }
}

impl<'a> IntoIterator for &'a Decls {
    type Item = (&'a PartDecl, &'a PartExtras);
    type IntoIter = std::iter::Zip<std::slice::Iter<'a, PartDecl>, std::slice::Iter<'a, PartExtras>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// The parts of one kind a component declared (its value parts, its reset parts): seldom more than two.
pub(crate) type PartKeys = smallvec::SmallVec<[(PartKey, Option<u64>); 2]>;

/// What a part declares beyond the sensing core's `PartDecl`.
#[derive(Clone, Debug, Default)]
pub(crate) struct PartExtras {
    pub focusable: bool,
    pub focus_shape: Option<FocusShape>,
    pub drag_cursor: Option<CursorIcon>,
    pub tooltip: Option<Tooltip>,
    pub pointer_only: Option<&'static str>,
    /// The part senses a click but declares no AccessKit `Click`: a click would do nothing.
    pub no_access_click: bool,
    pub keyboard_via: Option<PartKey>,
    pub value: bool,
    pub own_reset: bool,
    pub state_layer: bool,
    pub selected: bool,
    pub default_stop: bool,
    pub nudges: bool,
    pub toggles: Option<crate::contract::overlay::ToggleDecl>,
    /// A header part at the end edge: its stop registers after the header's children.
    pub after_header: bool,
}

impl PartBuilder<'_> {
    /// What the part senses; hover only by default.
    pub fn sense(self, sense: Sense) -> Self {
        self.decl.sense = sense;
        self
    }

    /// The part's level and sub-level in the resolve order.
    pub fn priority(self, priority: HitPriority) -> Self {
        self.decl.priority = priority;
        self
    }

    /// The cursor while the pointer is over the part.
    pub fn cursor(self, cursor: impl Into<Cursor>) -> Self {
        self.decl.cursor = Some(cursor.into());
        self
    }

    /// The cursor while the part is dragged.
    pub fn drag_cursor(self, cursor: CursorIcon) -> Self {
        self.declared.drag_cursor = Some(cursor);
        self
    }

    /// The part is a focus stop.
    pub fn focusable(self) -> Self {
        self.declared.focusable = true;
        self
    }

    /// The part is inactive (a switched-off clip): it paints with the Disabled state, and a `Draw` from
    /// `PaintUi::draw_part` fills data colours with the theme's disabled fill. The part keeps sensing and its access
    /// node stays enabled; whether it still acts is the component's rule.
    pub fn inactive(self, inactive: bool) -> Self {
        self.decl.inactive = inactive;
        self
    }

    /// The component holds a press on the part that the part does not sense itself (a collection's press on a row's
    /// toggle cell): the part paints pressed.
    pub fn held(self, held: bool) -> Self {
        self.decl.held = held;
        self
    }

    /// The part is the arrival of the component's part roving group: Tab and Shift+Tab into the group land here, not
    /// on the first member. The last part declared `selected` wins.
    pub fn selected(self) -> Self {
        self.declared.selected = true;
        self
    }

    /// The focused part takes the arrow keys, with any modifier, as nudges of its content (`Intents::take_nudge`),
    /// wherever it sits: in a collection item too.
    pub fn nudges(self) -> Self {
        self.declared.nudges = true;
        self
    }

    /// The part's stop is its scope's default (`Initial::Default`); the scope focuses the first default stop in tab order.
    pub fn default_stop(self) -> Self {
        self.declared.default_stop = true;
        self
    }

    /// The part is what `hit` says for the OS window: its move area, one of its edge zones or a caption button. It
    /// joins the exclusive list of the window's viewport, which `WindowHost::caption_regions` reads for the Windows
    /// hit-test sink. The rect is the part's shape bounds, which lie in viewport points for a window's own content;
    /// presses inside the app on the part follow the press-target rule.
    pub fn exclusive(self, hit: crate::runtime::windows::WindowHit) -> Self {
        if let Some((ctx, owner)) = self.window {
            let part = PartRef { key: self.decl.key, index: self.decl.index };
            crate::runtime::windows::note_exclusive(ctx, owner, part, hit, self.decl.shape.bounds());
        }
        self
    }

    /// The shape the focus ring takes around the part.
    pub fn focus_shape(self, shape: FocusShape) -> Self {
        self.declared.focus_shape = Some(shape);
        self
    }

    /// Value flags of the part, merged into its state at step 12 and its sub-node at step 16.
    pub fn flags(self, flags: Flags) -> Self {
        self.decl.flags |= flags;
        self
    }

    /// The part sits above the body or items in the stacking order and is painted by `paint_over`.
    pub fn over(self) -> Self {
        self.decl.over = true;
        self
    }

    /// The part's look is the plain state layer: the paint step fills its rect with `StyleCx::state_layer` in the
    /// part's state, under the component's paint, with control corners.
    pub fn state_layer(self) -> Self {
        self.declared.state_layer = true;
        self
    }

    /// The part's tooltip.
    pub fn tooltip(self, tooltip: impl Into<Tooltip>) -> Self {
        self.declared.tooltip = Some(tooltip.into());
        self
    }

    /// The part shows the component's value: a double click on it resets the value, as one on the root does. A
    /// stepper button or a scroll thumb is no value part.
    pub fn value_part(self) -> Self {
        self.declared.value = true;
        self
    }

    /// A reset aimed at the part (Reset while it is the focused part, a double click on it) does not reset the
    /// binding: `Cx::reset_part` names the part and `update` proposes the value with that part at its own default.
    /// Implies `value_part`.
    pub fn own_reset(self) -> Self {
        self.declared.value = true;
        self.declared.own_reset = true;
        self
    }

    /// Whether the part's node declares AccessKit `Click` when it senses a click; a part whose click does nothing in
    /// this pass turns it off.
    #[doc(hidden)]
    pub fn access_click(self, on: bool) -> Self {
        self.declared.no_access_click = !on;
        self
    }

    /// A part with no focus stop names its keyboard path: the intent or command that does the same.
    pub fn pointer_only(self, reason: &'static str) -> Self {
        self.declared.pointer_only = Some(reason);
        self
    }

    /// The part opens and closes the component's popup `popup` (the name given to `cx.popup`): a press on it toggles
    /// the popup, so press, drag and release over a row picks it, and a click leaves it open (`on_click` acts on the
    /// release instead), AccessKit Expand and Collapse on its sub-node open and close it, Alt+Down from the part (or from
    /// the component when no toggling part is focused) opens the first toggled popup that is closed and Alt+Up closes
    /// the open ones, and the sub-node reports `expanded`. The component declares the sub-node with `Access::node`.
    pub fn toggles(self, popup: &'static str) -> Self {
        self.toggles_at(popup, 0)
    }

    /// A part with no focus stop of its own does the same as the focusable part `part` of this component, which
    /// carries the keyboard path.
    pub fn keyboard_via(self, part: PartKey) -> Self {
        self.declared.keyboard_via = Some(part);
        self
    }
}

/// Step 7's context: the step-5 read view plus the rect geometry and the part builders.
#[non_exhaustive]
pub struct PartsCx<'a, C: Component> {
    /// The rect minus padding, mirrored for right-to-left.
    pub content_rect: Rect,
    /// Start and end geometry of the rect.
    pub flow: Flow,
    pub(crate) decls: &'a mut Decls,
    pub(crate) rovings: &'a mut Vec<Roving>,
    pub(crate) scrolls: &'a mut Vec<ScrollDecl>,
    pub(crate) text: &'a mut crate::contract::text::TextStep,
    pub(crate) last_state: (bool, bool),
    pub(crate) measure: MeasureCx<'a, C>,
    /// What the shape laid out before `parts`: a collection's rows as the item step places them this pass, a
    /// container's header slots.
    pub(crate) items: Option<std::rc::Rc<dyn std::any::Any>>,
}

impl<C: crate::contract::Container> PartsCx<'_, C> {
    /// The slot at `edge` of the header's line that `Container::header_parts` keeps for the container's own parts;
    /// `None` when it keeps none there.
    pub fn header_slot(&self, edge: crate::contract::HeaderEdge) -> Option<Rect> {
        let slots = self.items.as_ref()?.downcast_ref::<crate::contract::body::HeaderSlots>()?;
        match edge {
            crate::contract::HeaderEdge::Start => slots.start,
            crate::contract::HeaderEdge::End => slots.end,
        }
    }

    /// Declares a part in the header slot at `edge`. Its stop sits where the part sits: at the start edge before the
    /// header's children, at the end edge after them.
    pub fn header_part(&mut self, edge: crate::contract::HeaderEdge, part: impl Into<PartRef>, shape: impl Into<HitShape>) -> PartBuilder<'_> {
        let builder = self.part(part, shape);
        builder.declared.after_header = edge == crate::contract::HeaderEdge::End;
        builder
    }
}

impl<C: crate::contract::Collection> PartsCx<'_, C> {
    /// The rect of the item `key` this pass, in the coordinates the item step lays rows out in (a transformed
    /// collection's, before the transform). `None` for an item the step does not build: neither in view nor kept.
    pub fn rect_of(&self, key: &C::Key) -> Option<Rect> {
        let rows = self.items.as_ref()?.downcast_ref::<crate::contract::items::Rows<C::Key>>()?;
        rows.rects.iter().find(|(held, _)| held == key).map(|(_, rect)| *rect)
    }

    /// How far the rows are scrolled this pass.
    pub fn offset(&self) -> craft_behavior::ScrollPos {
        self.items
            .as_ref()
            .and_then(|rows| rows.downcast_ref::<crate::contract::items::Rows<C::Key>>())
            .map_or_else(Default::default, |rows| rows.offset)
    }

    /// Where the rows show this pass, less the scroll bar while it shows. Content the collection shows over its rows
    /// stays inside it, so it neither covers the bar nor takes its presses. `None` in a pass the item step has not
    /// laid out yet, which shows no scroll bar either.
    pub fn rows_view(&self) -> Option<Rect> {
        self.items.as_ref().and_then(|rows| rows.downcast_ref::<crate::contract::items::Rows<C::Key>>()).map(|rows| rows.view)
    }
}

impl<'a, C: Component> Deref for PartsCx<'a, C> {
    type Target = MeasureCx<'a, C>;

    fn deref(&self) -> &MeasureCx<'a, C> {
        &self.measure
    }
}

impl<C: Component> PartsCx<'_, C> {
    /// Last pass's pointer-over state of this component, so `parts` can register hover-revealed parts.
    pub fn hovered(&self) -> bool {
        self.last_state.0
    }

    /// Last pass's focus-visible state of this component.
    pub fn focus_visible(&self) -> bool {
        self.last_state.1
    }

    /// Declares a part; a repeated part is named with its index: `cx.part(POINT.at(id), shape)`.
    pub fn part(&mut self, part: impl Into<PartRef>, shape: impl Into<HitShape>) -> PartBuilder<'_> {
        let part = part.into();
        let mut decl = PartDecl::new(part.key, shape, Sense::HOVER);
        decl.index = part.index;
        decl.priority = HitPriority::new(HitLevel::Body, 0);
        if part.key == PartKey::CELL
            && let Some(column) = part.index
        {
            decl.flags |= crate::contract::items::rows::cell_flags(self.measure.ui.ctx(), self.measure.id, column);
        }
        let (decl, declared) = self.decls.push(decl);
        let window = (!self.measure.ui.is_sizing_pass()).then(|| (self.measure.ui.ctx(), self.measure.id));
        PartBuilder { decl, declared, window }
    }

    /// Declares `rect`, the slot where the component shows its inside label (`EnvView::inside_label`), as the label's
    /// part (`InsideLabel::PART`): it shows the label's tooltip and, for a scrubbing label, takes the press a
    /// `Scrub::by_label` scrubs with, in edit too. Declares nothing without an inside label.
    pub fn label_part(&mut self, rect: Rect) {
        let Some(inside) = self.measure.env.inside_label() else { return };
        let sense = if inside.scrubs() { Sense::CLICK | Sense::DRAG } else { Sense::HOVER };
        let tooltip = inside.tooltip().cloned();
        let builder = self
            .part(crate::contract::InsideLabel::PART, rect)
            .sense(sense)
            .priority(HitPriority::new(HitLevel::Handle, 0))
            .pointer_only("the control's own keys change its value as the label's drag does");
        if let Some(tooltip) = tooltip {
            builder.tooltip(tooltip);
        }
    }

    /// Declares a roving group over part and child keys.
    pub fn roving(&mut self, roving: Roving) {
        self.rovings.push(roving);
    }

    /// Declares one axis's scroll bar: the track as part `key` and the thumb as part `key.at(THUMB)`, inside
    /// `rect`. The pipeline runs thumb drag, end drag when the span resizes its ends, and page press on the
    /// track; `Pointer::scroll_moved(key)` reads the range the part moved to.
    pub fn scroll(&mut self, key: PartKey, rect: Rect, axis: Axis, span: ScrollSpan) {
        let rtl = self.flow.is_rtl();
        let min_thumb = crate::style::min_thumb(self.measure.tokens);
        let thumb = crate::primitives::ScrollBar::thumb(min_thumb, rect, axis, rtl, &span);
        self.part(key, rect)
            .sense(Sense::CLICK)
            .access_click(!thumb_covers_centre(rect, thumb, axis))
            .priority(HitPriority::new(HitLevel::Body, 0))
            .pointer_only("Page Up and Page Down scroll the region");
        self.part(key.at(THUMB), thumb)
            .sense(Sense::DRAG)
            .priority(HitPriority::new(HitLevel::Body, 1))
            .pointer_only("Page Up and Page Down scroll the region");
        self.scrolls.push(ScrollDecl { key, axis, rtl, track: rect, thumb, span, min_thumb });
    }
}
