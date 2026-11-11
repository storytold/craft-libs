//! Keyframe lanes, the keyframe navigator and the animation toggle (N1.04).

#[cfg(feature = "screen-testing")]
mod stories;

use std::sync::Arc;

use craft_behavior::{
    CancelReason, EditEvent, GestureId, Granularity, HitLevel, HitPriority, IconId, Intent, MarqueeSelect, Metrics, Mixed, PartKey, PointerButton,
    PressCancel, PressPhase, Selection, SelectionChange, Sense,
};
use craft_ui::buttons::{IconButton, ToggleButton};
use craft_ui::contract::{
    Access, Bind, Common, Component, Cx, EditResponse, Focus as _, Gap, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pattern, Pointer as _,
    Props as _, Remember as _, Role, SetPosition, Show as _, ShowContainer as _, clicked,
};
use craft_ui::layout_components::Row;
use craft_ui::primitives::{IconSpec, Mirror, PaintShape, StrokeWidth};
use craft_ui::runtime::edit::next_gesture_id;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2, pos2, vec2};

use super::clips::context_menu;
use super::{TimeAxis, time_axis};
use crate::time::{TICKS_PER_SECOND, Tick, TimeRange};
use crate::timecode::format_time;

/// A keyframe's id, the app's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyframeId(u64);

impl KeyframeId {
    /// The id `value`.
    pub const fn new(value: u64) -> Self {
        KeyframeId(value)
    }

    /// The id's number.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// How the segment leaving a keyframe interpolates (Premiere's temporal set; After Effects' Linear, Bezier and Hold
/// are a subset). Also picks the glyph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyframeInterpolation {
    #[default]
    /// Straight line to the next key.
    Linear,
    /// The value stays until the next key.
    Hold,
    /// A curve shaped by the key's two handles.
    Bezier,
    /// A Bezier whose handles the app sets for a smooth curve through the key.
    AutoBezier,
    /// A Bezier whose handles stay continuous across the key.
    ContinuousBezier,
    /// Eases into the key.
    EaseIn,
    /// Eases out of the key.
    EaseOut,
}

/// A keyframe on a lane.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct KeyframeMark {
    /// The keyframe's identity.
    pub id: KeyframeId,
    /// The keyframe's time.
    pub time: Tick,
    /// How the segment after the keyframe interpolates.
    pub interpolation: KeyframeInterpolation,
}

impl KeyframeMark {
    /// A keyframe `id` at `time` that interpolates as `interpolation`.
    pub const fn new(id: KeyframeId, time: Tick, interpolation: KeyframeInterpolation) -> Self {
        KeyframeMark { id, time, interpolation }
    }
}

/// The icon that marks a key of `interpolation` on a lane: a diamond for Linear, a square for Hold, a circle for the
/// Bezier kinds, with After Effects' own marks for Auto, Continuous and the eases.
pub(crate) fn keyframe_icon(interpolation: KeyframeInterpolation) -> IconSpec<'static> {
    let name = match interpolation {
        KeyframeInterpolation::Linear => "key_linear_filled",
        KeyframeInterpolation::Hold => "key_hold",
        KeyframeInterpolation::Bezier => "key_bezier",
        KeyframeInterpolation::ContinuousBezier => "key_continuous_bezier",
        KeyframeInterpolation::AutoBezier => "key_auto_bezier",
        KeyframeInterpolation::EaseIn => "key_ease_in",
        KeyframeInterpolation::EaseOut => "key_ease_out",
    };
    IconSpec::svg(IconId::new(name))
}

craft_ui::words! {
    /// The keyframe controls' words, English by default (FilmCraft's).
    #[non_exhaustive]
    pub struct KeyframeWords {
        previous: "keyframes.previous" = "Go to previous keyframe";
        next: "keyframes.next" = "Go to next keyframe";
        add_remove: "keyframes.addRemove" = "Add/Remove Keyframe";
        toggle_animation: "keyframes.toggleAnimation" = "Toggle animation";
    }
}

/// The key of one keyframe's part: `KEY.at(id)`.
const KEY: PartKey = PartKey::new("keyframe");

/// The style of a keyframe lane.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct KeyframeLaneStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: craft_ui::style::FocusLook,
    /// A key.
    key: StyleColor,
    /// A selected key.
    selected: StyleColor,
    /// A key on a disabled lane.
    disabled: StyleColor,
    /// The marquee's fill.
    marquee_fill: StyleColor,
    /// The marquee's outline.
    marquee_edge: StyleColor,
    /// The side of a key's glyph, and of its hit area.
    glyph: f32,
    /// How near the playhead or another key a Shift-dragged key snaps, in points.
    snap_reach: f32,
}

/// A key drag under way: the keys it moves and the press it measures from.
#[derive(Clone, Debug, Default)]
struct Drag {
    group: Vec<KeyframeId>,
    press_x: f32,
    /// The time of the key that was grabbed.
    anchor: Tick,
    id: GestureId,
    /// The delta the drag has reached, as drawn.
    now: Tick,
}

/// A marquee under way.
#[derive(Clone)]
struct Sweep(MarqueeSelect<KeyframeId>);

impl Default for Sweep {
    fn default() -> Self {
        Sweep(MarqueeSelect::begin(Pos2::ZERO, craft_keymap::Mods::NONE, Vec::new()))
    }
}

/// What the lane's paint reads of the pass's presses.
#[derive(Clone, Debug, Default)]
struct Seen {
    marquee: Option<Rect>,
    group: Vec<KeyframeId>,
    /// The delta the dragged keys are drawn moved by.
    shown: Tick,
}

/// The keyframes of one property row on its lane.
///
/// - The selection is the app's [`Selection`] of keys; clicks, the marquee and keys change it and the lane reports each
///   change as a [`SelectionChange`]: a click replaces, Shift+click and Cmd/Ctrl+click toggle the key; a drag on empty
///   lane draws a marquee ([`MarqueeSelect`]: Shift adds, Cmd/Ctrl toggles, Alt subtracts).
/// - Dragging a selected key moves every selected key by the same delta, snapped to frames and kept inside
///   [`bounds`](Self::bounds); with Shift it snaps to the [`playhead`](Self::playhead) and the other keys within the
///   snap reach. The delta is the lane's value: one gesture from press to release, Esc cancels. The lane draws the
///   selected keys moved by the delta, so the app applies the delta at the commit and keeps passing the keys as they
///   were.
/// - A double-click and a secondary click on a key are reported (After Effects opens the keyframe's dialog and menu,
///   `timeline.rs:4083`, `:4101`).
/// - Keyboard: one Tab stop when it has keys; Left and Right select the previous and next key, Shift+Left/Right nudge
///   the selected keys one frame (a `Set` of the delta), Delete and Backspace report
///   [`KeyframeLaneOutput::delete`].
/// - AccessKit: a list box named by the lane's label, one option per key named by its time as timecode.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct KeyframeLane<'a> {
    #[common]
    common: Common,
    keys: &'a [KeyframeMark],
    selection: &'a Selection<KeyframeId>,
    /// The playhead time; Shift-dragged keys snap to it.
    #[prop]
    playhead: Tick,
    /// Keys stay inside this range while dragged (the clip).
    #[prop]
    bounds: Option<TimeRange>,
    report: Report,
}

/// What the lane reports besides the delta.
#[derive(Clone, Debug, Default)]
struct Report {
    delta: Option<EditEvent<Tick>>,
    select: Option<SelectionChange<KeyframeId>>,
    double_clicked: Option<KeyframeId>,
    context: Option<KeyframeId>,
    delete: bool,
}

impl<'a> KeyframeLane<'a> {
    /// A lane named `label` for `keys`, with the app's key `selection`.
    pub fn new(label: impl Into<Label>, keys: &'a [KeyframeMark], selection: &'a Selection<KeyframeId>) -> Self {
        KeyframeLane { common: Common::new(), keys, selection, playhead: Tick::ZERO, bounds: None, report: Report::default() }.label(label).ltr_axis()
    }

    fn sorted(&self) -> Vec<&'a KeyframeMark> {
        let mut sorted: Vec<&KeyframeMark> = self.keys.iter().collect();
        sorted.sort_by_key(|key| key.time);
        sorted
    }

    /// The range of deltas that keeps all of `group` inside the bounds.
    fn limits(&self, group: &[KeyframeId]) -> (i64, i64) {
        let (mut low, mut high) = (i64::MIN, i64::MAX);
        if let Some(bounds) = self.bounds {
            for key in self.keys.iter().filter(|key| group.contains(&key.id)) {
                low = low.max((bounds.start - key.time).0);
                high = high.min((bounds.end() - key.time).0);
            }
        }
        if low > high { (0, 0) } else { (low, high) }
    }

    /// The delta of a drag that has reached screen `x`: on the frame grid and inside the bounds; with `snapping` on
    /// the playhead or an unmoved key when one is near enough.
    fn delta_at(&self, drag: &Drag, x: f32, snapping: bool, axis: &TimeAxis, reach: f32) -> Tick {
        let (low, high) = self.limits(&drag.group);
        let raw = Tick((f64::from(x - drag.press_x) * TICKS_PER_SECOND as f64 / axis.view.points_per_second).round() as i64);
        let requested = axis.format.rate.snap_nearest(drag.anchor + raw) - drag.anchor;
        let mut delta = Tick(requested.0.clamp(low, high));
        if snapping {
            let points_per_tick = axis.view.points_per_second / TICKS_PER_SECOND as f64;
            let mut closest = f64::from(reach);
            let targets: Vec<Tick> =
                std::iter::once(self.playhead).chain(self.keys.iter().filter(|k| !drag.group.contains(&k.id)).map(|k| k.time)).collect();
            for moved in self.keys.iter().filter(|k| drag.group.contains(&k.id)) {
                for target in &targets {
                    let snap = (*target - moved.time).0;
                    let distance = ((snap - requested.0) as f64 * points_per_tick).abs();
                    if distance <= closest && (low..=high).contains(&snap) {
                        closest = distance;
                        delta = Tick(snap);
                    }
                }
            }
        }
        delta
    }

    fn glyph_rect(&self, key: &KeyframeMark, axis: &TimeAxis, lane: Rect, glyph: f32) -> Rect {
        Rect::from_center_size(pos2(axis.x_of(key.time, lane.left()), lane.center().y), vec2(glyph, glyph))
    }
}

impl Component for KeyframeLane<'_> {
    const ROLE: Role = Role::LIST_BOX;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_deletes().with_chrome_key(false);
    const SINGLE_VALUE: bool = true;
    type Style = KeyframeLaneStyle;
    type Output = KeyframeLaneOutput;

    fn style(cx: &StyleCx<'_>, _: KeyframeLaneStyleKey) -> KeyframeLaneStyle {
        let tokens = cx.tokens;
        KeyframeLaneStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            key: cx.color(tokens.color.text.muted),
            selected: cx.color(tokens.color.accent.text),
            disabled: cx.color(tokens.color.text.disabled),
            marquee_fill: cx.color(tokens.color.accent.soft),
            marquee_edge: cx.color(tokens.color.accent.border),
            glyph: tokens.size.icon.md,
            snap_reach: tokens.size.hit.target,
        }
    }

    fn sense(&self) -> Sense {
        let press = Sense::CLICK | Sense::DRAG | Sense::CONTEXT_CLICK;
        if self.keys.is_empty() { press } else { press | Sense::FOCUS | Sense::TAKE_FOCUS }
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        Some(craft_behavior::Axis::X)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(0.0, cx.style.glyph)
    }

    /// The keys, above the lane's own press: a drag that starts on a key moves it.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let glyph = cx.style.glyph;
        let keys = self.sorted();
        for key in keys {
            let at = self.glyph_rect(key, &axis, rect, glyph);
            if !rect.expand(glyph / 2.0).intersects(at) {
                continue;
            }
            let part = cx
                .part(KEY.at(key.id.get()), at)
                .sense(Sense::CLICK | Sense::DRAG | Sense::CONTEXT_CLICK | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .pointer_only("the lane's arrow keys select and move the keys");
            if self.selection.contains(&key.id) {
                part.selected();
            }
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let (glyph, reach) = (cx.style.glyph, cx.style.snap_reach);
        let lane = cx.rect;
        let sorted = self.sorted();
        let mut out = Report::default();
        let mut seen = Seen::default();
        let selection = self.selection;

        for key in &sorted {
            let part = KEY.at(key.id.get());
            let Some(result) = cx.part_if_declared(part) else { continue };
            let press = result.press;
            if press.double_pressed() {
                out.double_clicked = Some(key.id);
            }
            if context_menu(cx, part).is_some() {
                out.context = Some(key.id);
            }
            let Some(start) = press.phase.start().filter(|start| start.button == PointerButton::Primary) else { continue };
            let additive = press.mods.shift || press.mods.cmd;
            match press.phase {
                PressPhase::DragStart { .. } => {
                    let mut group = selection.items.clone();
                    if !selection.contains(&key.id) {
                        if additive {
                            group.push(key.id);
                            out.select = Some(SelectionChange::Toggle(key.id));
                        } else {
                            group = vec![key.id];
                            out.select = Some(SelectionChange::Set(vec![key.id]));
                        }
                    }
                    let id = next_gesture_id(&cx.response.ctx);
                    *cx.press_slot::<Drag>() = Some(Drag { group, press_x: start.origin.x, anchor: key.time, id, now: Tick::ZERO });
                    out.delta = Some(EditEvent::begin(id, Mixed::Same(Tick::ZERO), Arc::from(cx.resolved.label.to_owned()), None));
                }
                PressPhase::Drag { pos, .. } => {
                    if let Some(drag) = cx.press_slot::<Drag>().clone() {
                        let now = self.delta_at(&drag, pos.x, press.mods.shift, &axis, reach);
                        if let Some(held) = cx.press_slot::<Drag>().as_mut() {
                            held.now = now;
                        }
                        out.delta = Some(EditEvent::update(drag.id, now));
                    }
                }
                PressPhase::Release { click: false, .. } => {
                    if let Some(drag) = cx.press_slot::<Drag>().take() {
                        out.delta = Some(EditEvent::commit(drag.id, Mixed::Same(Tick::ZERO), drag.now));
                    }
                }
                PressPhase::Cancel { reason, .. } => {
                    if let Some(drag) = cx.press_slot::<Drag>().take() {
                        out.delta = Some(EditEvent::cancel(drag.id, cancel_reason(reason), Some(Mixed::Same(Tick::ZERO))));
                    }
                }
                PressPhase::Release { click: true, .. } => {
                    out.select = Some(if additive { SelectionChange::Toggle(key.id) } else { SelectionChange::Set(vec![key.id]) });
                }
                _ => {}
            }
        }
        if let Some(drag) = cx.press_slot::<Drag>().clone() {
            seen.group = drag.group;
            seen.shown = drag.now;
        }

        // The marquee and clicks on the empty lane.
        let press = cx.press();
        let mods = press.mods;
        match press.phase {
            PressPhase::DragStart { start, pos, .. } if start.button == PointerButton::Primary => {
                let mut marquee = MarqueeSelect::begin(start.origin, mods, selection.items.clone());
                marquee.update(pos);
                *cx.press_slot::<Sweep>() = Some(Sweep(marquee));
            }
            PressPhase::Drag { start, pos, .. } if start.button == PointerButton::Primary => {
                if let Some(sweep) = cx.press_slot::<Sweep>().as_mut() {
                    sweep.0.update(pos);
                }
            }
            PressPhase::Cancel { .. } => {
                if let Some(Sweep(marquee)) = cx.press_slot::<Sweep>().take() {
                    out.select = restore(marquee.cancel(), selection);
                }
            }
            PressPhase::Release { click: true, start, .. }
                if start.button == PointerButton::Primary && out.select.is_none() && !(mods.shift || mods.cmd) && !selection.items.is_empty() =>
            {
                out.select = Some(SelectionChange::Clear);
            }
            _ => {}
        }
        if let Some(Sweep(marquee)) = cx.press_slot::<Sweep>().clone() {
            let items = sorted.iter().map(|key| (key.id, self.glyph_rect(key, &axis, lane, glyph)));
            let keys = marquee.selection(items);
            if keys != selection.items {
                out.select = Some(if keys.is_empty() { SelectionChange::Clear } else { SelectionChange::Set(keys) });
            }
            seen.marquee = Some(marquee.rect().intersect(lane));
        }
        *cx.memory::<Seen>() = seen;

        // The keyboard.
        if cx.is_focused() && !sorted.is_empty() {
            self.run_keys(cx, &axis, &sorted, &mut out);
        }
        self.report = out;
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let (style, lane) = (cx.style, cx.rect);
        let seen = cx.recall::<Seen>();
        let shown = seen.shown;
        let enabled = cx.resolved.enabled;
        let draw = ui.draw();
        for key in self.sorted() {
            let on = self.selection.contains(&key.id);
            let moved = if on && seen.group.contains(&key.id) { key.time + shown } else { key.time };
            let at = Rect::from_center_size(pos2(axis.x_of(moved, lane.left()), lane.center().y), vec2(style.glyph, style.glyph));
            if !lane.expand(style.glyph / 2.0).intersects(at) {
                continue;
            }
            let color = match (enabled, on) {
                (false, _) => style.disabled,
                (true, true) => style.selected,
                (true, false) => style.key,
            };
            draw.icon(at, &keyframe_icon(key.interpolation).color(color));
        }
        if let Some(area) = seen.marquee {
            let shape = PaintShape::Rect(area, CornerRadius::ZERO);
            draw.fill(&shape, style.marquee_fill);
            draw.stroke(&shape, StrokeWidth::Hairline, style.marquee_edge);
        }
    }

    /// A list box, one option per key named by its time.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let sorted = self.sorted();
        for (at, key) in sorted.iter().enumerate() {
            let node = access.node(KEY.at(key.id.get()), Role::LIST_BOX_OPTION);
            node.name = Name::Text(format_time(key.time, &axis.format));
            node.position = Some(SetPosition::new(at).of(sorted.len()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> KeyframeLaneOutput {
        let Report { delta, select, double_clicked, context, delete } = self.report;
        KeyframeLaneOutput { response: cx.response.clone(), delta, select, double_clicked, context, delete }
    }
}

fn cancel_reason(cancel: PressCancel) -> CancelReason {
    match cancel {
        PressCancel::Escape => CancelReason::Escape,
        PressCancel::Removed => CancelReason::Removed,
        _ => CancelReason::Blur,
    }
}

/// The selection change `base` makes against `current`: the selection a cancelled marquee restores.
fn restore(base: Vec<KeyframeId>, current: &Selection<KeyframeId>) -> Option<SelectionChange<KeyframeId>> {
    if base == current.items {
        None
    } else if base.is_empty() {
        Some(SelectionChange::Clear)
    } else {
        Some(SelectionChange::Set(base))
    }
}

impl KeyframeLane<'_> {
    /// Left and Right select the previous and next key, Shift+Left and Right nudge the selected keys a frame, Delete
    /// asks to delete them.
    fn run_keys(&self, cx: &mut Cx<'_, Self>, axis: &TimeAxis, sorted: &[&KeyframeMark], out: &mut Report) {
        if let Some((direction, mods)) = cx.intents().take_nudge()
            && direction.x != 0.0
        {
            let toward = if direction.x < 0.0 { -1 } else { 1 };
            if mods.shift {
                let selected: Vec<KeyframeId> = sorted.iter().filter(|key| self.selection.contains(&key.id)).map(|key| key.id).collect();
                let (low, high) = self.limits(&selected);
                let step = Tick(axis.frame().0 * toward);
                if !selected.is_empty() && (low..=high).contains(&step.0) {
                    out.delta = Some(EditEvent::set(Mixed::Same(Tick::ZERO), step, Arc::from(cx.resolved.label.to_owned()), None));
                }
            } else {
                let last = sorted.len() - 1;
                let lead = self.selection.lead.and_then(|lead| sorted.iter().position(|key| key.id == lead));
                let at = if toward > 0 { lead.map_or(0, |at| (at + 1).min(last)) } else { lead.map_or(last, |at| at.saturating_sub(1)) };
                out.select = Some(SelectionChange::Set(vec![sorted[at].id]));
            }
        }
        let forward = cx.intents().take_exact(&Intent::delete(true, Granularity::Unit));
        let backward = cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
        out.delete = (forward || backward) && !self.selection.items.is_empty();
    }
}

/// What a keyframe lane did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct KeyframeLaneOutput {
    /// The lane's response.
    pub response: Response,
    /// The delta the dragged keys move by (the selected keys, all by the same delta): one gesture from the drag's
    /// start to its release, or a `Set` of one frame from the keyboard.
    pub delta: Option<EditEvent<Tick>>,
    /// A click, marquee or key changed the selection.
    pub select: Option<SelectionChange<KeyframeId>>,
    /// The key that was double-clicked.
    pub double_clicked: Option<KeyframeId>,
    /// The key that asked for its menu; the app opens it.
    pub context: Option<KeyframeId>,
    /// Delete or Backspace was pressed with keys selected.
    pub delete: bool,
}

impl Outcome for KeyframeLaneOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        let mut facts = craft_ui::testing::StoryOutput::default();
        facts.id = Some(self.response.id);
        facts.clicked = clicked(&self.response);
        facts.edits = self.delta.iter().filter_map(craft_ui::testing::EditKind::of).collect();
        facts
    }
}

/// Premiere's keyframe navigator at a property row: a previous-key button, a diamond that adds a key at the playhead
/// or removes the one there, and a next-key button. A button without a key that way is disabled; the diamond is filled
/// when a key is at the playhead, else an outline. Three Tab stops, Space or Enter presses.
#[derive(Pattern)]
pub struct KeyframeNavigator {
    label: Label,
    animated: bool,
    has_previous: bool,
    has_next: bool,
    at_key: bool,
}

impl KeyframeNavigator {
    /// A navigator named `label`. `has_previous` / `has_next`: a key before / after the playhead; `at_key`: one at it.
    pub fn new(label: impl Into<Label>, has_previous: bool, has_next: bool, at_key: bool) -> Self {
        KeyframeNavigator { label: label.into(), animated: true, has_previous, has_next, at_key }
    }

    /// Without animation only the diamond shows (it turns animation on).
    pub fn animated(mut self, on: bool) -> Self {
        self.animated = on;
        self
    }

    /// Shows the navigator and reports the button pressed.
    // filmcraft: crates/ui-egui/src/panels/effect_controls.rs:487 `keyframe_nav`.
    pub fn show(self, ui: &mut egui::Ui) -> KeyframeNavigatorOutput {
        let KeyframeNavigator { label, animated, has_previous, has_next, at_key } = self;
        let words = craft_ui::runtime::env::Env::of(ui).words::<KeyframeWords>();
        let mut out = KeyframeNavigatorOutput::default();
        let shown = Row::new().label(label).gap(Gap::Tight).show(ui, |ui, _| {
            if animated {
                // The time axis is never mirrored, so the arrows keep their directions.
                let previous = IconSpec::svg(IconId::new("chevron_left")).mirror(Mirror::Never);
                let step = IconButton::new(previous).label(words.previous.clone()).enabled(has_previous).show(ui);
                out.previous = clicked(&step);
            }
            let diamond = IconId::new(if at_key { "key_linear_filled" } else { "diamond" });
            out.toggle = clicked(&IconButton::new(diamond).label(words.add_remove.clone()).show(ui));
            if animated {
                let next = IconSpec::svg(IconId::new("chevron_right")).mirror(Mirror::Never);
                out.next = clicked(&IconButton::new(next).label(words.next.clone()).enabled(has_next).show(ui));
            }
        });
        out.response = Some(shown.output);
        out
    }
}

/// Which navigator button was pressed.
#[derive(Debug, Default)]
#[must_use]
#[non_exhaustive]
pub struct KeyframeNavigatorOutput {
    /// The previous-key button was pressed.
    pub previous: bool,
    /// The next-key button was pressed.
    pub next: bool,
    /// The diamond: add a key at the playhead, or remove the one there.
    pub toggle: bool,
    /// The navigator's response.
    pub response: Option<Response>,
}

/// The stopwatch that turns a property's animation on and off (Premiere's Toggle animation, After Effects' Time-Vary
/// Stopwatch). A toggle button: one Tab stop, Space toggles; reports the new state as a `Set`.
#[derive(Pattern)]
pub struct AnimationToggle<'a> {
    label: Option<Label>,
    on: Bind<'a, bool>,
}

impl<'a> AnimationToggle<'a> {
    /// A toggle for `on`, the property's animation state, named by the keyframe words unless `label` names it.
    pub fn new(on: Bind<'a, bool>) -> Self {
        AnimationToggle { label: None, on }
    }

    /// The toggle's accessible name.
    pub fn label(mut self, label: impl Into<Label>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Shows the toggle; the response carries the new state when it changed.
    // effectcraft: crates/ui-egui/src/panels/timeline.rs:3922 `prop_stopwatch`.
    pub fn show(self, ui: &mut egui::Ui) -> EditResponse<bool> {
        let AnimationToggle { label, on } = self;
        let label = label.unwrap_or_else(|| craft_ui::runtime::env::Env::of(ui).words::<KeyframeWords>().toggle_animation.clone());
        ToggleButton::new(label, on).icon(IconSpec::svg(IconId::new("clock"))).icon_only().show(ui)
    }
}
