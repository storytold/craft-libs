//! `GraphEditor`: keyframed values (or their speed) as curves over time (N1.08), a leaf with a content painter for the
//! plot and parts for the keys, the handles and the bottom bar.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::kurbo;
use craft_behavior::{
    CursorIcon, Granularity, HitLevel, HitPriority, IconId, Intent, MarqueeSelect, Metrics, NavShape, PartKey, PressPhase, PressState, Roving,
    Selection, SelectionChange, Sense,
};
use craft_keymap::{Chord, Key, Mods};
use craft_ui::buttons::{IconButton, ToggleButton};
use craft_ui::contract::Remember as _;
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{Dashes, GridInk, GridLines, IconSpec, PaintShape, RuleKind, StrokeWidth, TextRun, TickRank};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, Flags, FocusLook, Resolve as _, State, StateMix, Stateful, States, StyleColor, StyleCx, TextLook, TypeRole};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2};
use std::ops::Deref;

use super::DragEvent;
use super::keyframes::{KeyframeId, KeyframeInterpolation};
use super::ruler::{ruler_ladder, ruler_ticks};
use super::view::{TimeAxis, time_axis};
use crate::time::Tick;
use crate::timecode::format_time;

/// The value range shows about this many grid lines (EffectCraft `graph.rs:439`).
const GRID_LINES: f64 = 6.0;
/// The share of the value span the auto range adds each side (`graph.rs:352`).
const AUTO_MARGIN: f64 = 0.08;
/// The share of the span Fit Selection adds each side, and the least it adds in value units (`graph.rs:790`, `:789`).
const FIT_MARGIN: f64 = 0.15;
const FIT_MARGIN_MIN: f64 = 1.0;
/// The share of the plot's height a reference graph leaves free above and below (EffectCraft `graph.rs:318`: it is
/// drawn in 90 % of the height). A proportion of the plot, not a size.
const REFERENCE_PAD: f32 = 0.05;
/// The share of a curve's colour a reference graph keeps (EffectCraft `graph_tools.rs:320`).
const REFERENCE_FADE: f32 = 0.35;
/// The type role of the value labels.
const VALUE_ROLE: TypeRole = TypeRole::Caption;

/// The plot behind the keys: a press on empty plot clears the selection or sweeps a box.
pub const PLOT: PartKey = PartKey::new("plot");
/// A key: `KEY.at(n)` for the `n`-th key of the curves in drawing order.
pub const KEY: PartKey = PartKey::new("key");
/// A handle of a key: `HANDLE.at(2 n + side)` for the `n`-th key, side 0 the incoming handle and 1 the outgoing one.
pub const HANDLE: PartKey = PartKey::new("handle");
/// A button of the bottom bar: `BAR.at(i)` in reading order.
pub const BAR: PartKey = PartKey::new("bar");

/// The chords the editor claims while it is focused.
const EASY_EASE: Chord = Chord::new(Mods::NONE, Key::F9);
const EASY_EASE_IN: Chord = Chord::new(Mods::SHIFT, Key::F9);
const EASY_EASE_OUT: Chord = Chord::new(Mods { cmd: true, ctrl: false, alt: false, shift: true }, Key::F9);
const CLAIMED: [Chord; 3] = [EASY_EASE, EASY_EASE_IN, EASY_EASE_OUT];

craft_ui::words! {
    /// The editor's bottom-bar tooltips, English by default (EffectCraft `graph.rs:611`).
    pub struct GraphEditorWords {
        value_graph: "graphEditor.valueGraph" = "Edit Value Graph";
        speed_graph: "graphEditor.speedGraph" = "Edit Speed Graph";
        show_selected: "graphEditor.showSelected" = "Show Selected Properties (else all animated properties)";
        show_reference: "graphEditor.showReference" = "Show Reference Graph";
        auto_zoom: "graphEditor.autoZoom" = "Auto-zoom graph height";
        fit_selection: "graphEditor.fitSelection" = "Fit selection to view";
        fit_all: "graphEditor.fitAll" = "Fit all graphs to view";
        separate: "graphEditor.separate" = "Separate Dimensions";
        hold: "graphEditor.hold" = "Convert selected keyframes to Hold";
        linear: "graphEditor.linear" = "Convert selected keyframes to Linear";
        auto_bezier: "graphEditor.autoBezier" = "Convert selected keyframes to Auto Bezier";
        easy_ease: "graphEditor.easyEase" = "Easy Ease (F9)";
        easy_ease_in: "graphEditor.easyEaseIn" = "Easy Ease In (Shift+F9)";
        easy_ease_out: "graphEditor.easyEaseOut" = "Easy Ease Out (Ctrl+Shift+F9)";
        handle_in: "graphEditor.handleIn" = "incoming handle";
        handle_out: "graphEditor.handleOut" = "outgoing handle";
    }
}

/// Which graph shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GraphMode {
    #[default]
    /// The property's value over time.
    Value,
    /// The rate of change of the value over time.
    Speed,
}

/// A key on a curve, with its Bezier handles in graph units (time, value) when it has them.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct GraphKey {
    /// The keyframe's identity.
    pub id: KeyframeId,
    /// The keyframe's time.
    pub time: Tick,
    /// The keyframe's value.
    pub value: f64,
    /// How the curve leaves the keyframe.
    pub interpolation: KeyframeInterpolation,
    /// The incoming Bezier handle as an absolute (time, value) point; `None` without one.
    pub in_handle: Option<(Tick, f64)>,
    /// The outgoing Bezier handle as an absolute (time, value) point; `None` without one.
    pub out_handle: Option<(Tick, f64)>,
}

impl GraphKey {
    /// A key `id` at `time` with `value`, with no handles.
    pub fn new(id: KeyframeId, time: Tick, value: f64, interpolation: KeyframeInterpolation) -> Self {
        Self { id, time, value, interpolation, in_handle: None, out_handle: None }
    }

    /// The key with its incoming and outgoing handles.
    #[must_use]
    pub fn handles(mut self, incoming: (Tick, f64), outgoing: (Tick, f64)) -> Self {
        self.in_handle = Some(incoming);
        self.out_handle = Some(outgoing);
        self
    }
}

/// One curve: a property's dimension, sampled by the app (the app evaluates its own interpolation and expressions).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct GraphCurve {
    /// The property and dimension, the app's.
    pub id: u64,
    /// The curve's display name.
    pub name: String,
    /// `None`: the next colour of the per-dimension set (red, green, blue, yellow; EffectCraft `graph.rs:22`).
    pub color: Option<Color32>,
    /// `(time, value)` in time order, about one per two points of width.
    pub samples: Vec<(Tick, f64)>,
    /// The curve's keyframes, in time order.
    pub keys: Vec<GraphKey>,
}

impl GraphCurve {
    /// A curve `id` named `name` through `samples`, with `keys` on it.
    pub fn new(id: u64, name: impl Into<String>, samples: Vec<(Tick, f64)>, keys: Vec<GraphKey>) -> Self {
        Self { id, name: name.into(), color: None, samples, keys }
    }

    /// The curve drawn in `color`.
    #[must_use]
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }
}

/// The graph's value range and options; the app keeps it per panel.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct GraphRange {
    /// The lowest value shown at the bottom of the plot.
    pub low: f64,
    /// The highest value shown at the top of the plot.
    pub high: f64,
    /// Which graph shows.
    pub mode: GraphMode,
    /// The range follows the shown curves (After Effects' Auto-zoom graph height).
    pub auto_zoom: bool,
    /// Show only the selected properties' curves; otherwise all animated ones.
    pub selected_only: bool,
    /// Show each dimension of a property as its own curve.
    pub separate_dimensions: bool,
    /// Show Reference Graph: the other graph type, faint, scaled to the plot (EffectCraft `graph.rs:301`). The curves
    /// come from [`GraphEditor::reference`].
    pub reference: bool,
}

impl Default for GraphRange {
    fn default() -> Self {
        Self { low: 0.0, high: 100.0, mode: GraphMode::Value, auto_zoom: true, selected_only: false, separate_dimensions: false, reference: false }
    }
}

/// A move of the selected keys in time and value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GraphKeyMove {
    /// The shift in time.
    pub delta_time: Tick,
    /// The shift in value, in the curve's own units.
    pub delta_value: f64,
}

/// Which handle of a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HandleSide {
    /// The incoming handle.
    In,
    /// The outgoing handle.
    Out,
}

/// A handle dragged to `(time, value)`; with Alt the two handles break apart.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct HandleMove {
    /// The keyframe the handle belongs to.
    pub key: KeyframeId,
    /// Which of the key's handles moved.
    pub side: HandleSide,
    /// The handle's new time.
    pub time: Tick,
    /// The handle's new value.
    pub value: f64,
    /// The two handles of the key move independently (Alt held).
    pub broken: bool,
}

/// A bottom-bar button, or a key on the plot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GraphAction {
    /// Delete with keys selected.
    Delete,
    /// Fit the view to the selected keys.
    FitSelection,
    /// Fit the view to every curve.
    FitAll,
    /// Convert the selected keys to Hold.
    Hold,
    /// Convert the selected keys to Linear.
    Linear,
    /// Convert the selected keys to Auto Bezier.
    AutoBezier,
    /// Easy Ease on the selected keys.
    EasyEase,
    /// Easy Ease In on the selected keys.
    EasyEaseIn,
    /// Easy Ease Out on the selected keys.
    EasyEaseOut,
}

/// A button of the bottom bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Value,
    Speed,
    SelectedOnly,
    AutoZoom,
    FitSelection,
    FitAll,
    Reference,
    Separate,
    Hold,
    Linear,
    AutoBezier,
    EasyEase,
    EaseIn,
    EaseOut,
}

impl Button {
    /// The buttons in reading order; a wider gap separates neighbours of different groups.
    const ALL: [Button; 14] = [
        Button::Value,
        Button::Speed,
        Button::SelectedOnly,
        Button::AutoZoom,
        Button::FitSelection,
        Button::FitAll,
        Button::Reference,
        Button::Separate,
        Button::Hold,
        Button::Linear,
        Button::AutoBezier,
        Button::EasyEase,
        Button::EaseIn,
        Button::EaseOut,
    ];

    fn group(self) -> u8 {
        match self {
            Button::Value | Button::Speed => 0,
            Button::SelectedOnly | Button::AutoZoom | Button::FitSelection | Button::FitAll => 1,
            Button::Reference => 2,
            Button::Separate => 3,
            _ => 4,
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Button::Value => "chart_line",
            Button::Speed => "gauge",
            Button::SelectedOnly => "pointer",
            Button::AutoZoom => "scaling",
            Button::FitSelection => "scan",
            Button::FitAll => "maximize_2",
            Button::Reference => "layers",
            Button::Separate => "split",
            Button::Hold => "key_hold",
            Button::Linear => "key_linear",
            Button::AutoBezier => "key_auto_bezier",
            Button::EasyEase => "keyframe_easy_ease",
            Button::EaseIn => "key_ease_in",
            Button::EaseOut => "key_ease_out",
        }
    }

    fn name(self, words: &GraphEditorWords) -> &str {
        match self {
            Button::Value => words.value_graph.as_str(),
            Button::Speed => words.speed_graph.as_str(),
            Button::SelectedOnly => words.show_selected.as_str(),
            Button::AutoZoom => words.auto_zoom.as_str(),
            Button::FitSelection => words.fit_selection.as_str(),
            Button::FitAll => words.fit_all.as_str(),
            Button::Reference => words.show_reference.as_str(),
            Button::Separate => words.separate.as_str(),
            Button::Hold => words.hold.as_str(),
            Button::Linear => words.linear.as_str(),
            Button::AutoBezier => words.auto_bezier.as_str(),
            Button::EasyEase => words.easy_ease.as_str(),
            Button::EaseIn => words.easy_ease_in.as_str(),
            Button::EaseOut => words.easy_ease_out.as_str(),
        }
    }

    /// Whether the button is on, for the toggles; `None` for an action.
    fn on(self, range: &GraphRange) -> Option<bool> {
        match self {
            Button::Value => Some(range.mode == GraphMode::Value),
            Button::Speed => Some(range.mode == GraphMode::Speed),
            Button::SelectedOnly => Some(range.selected_only),
            Button::AutoZoom => Some(range.auto_zoom),
            Button::Reference => Some(range.reference),
            Button::Separate => Some(range.separate_dimensions),
            _ => None,
        }
    }

    /// The action an action button reports.
    fn action(self) -> Option<GraphAction> {
        match self {
            Button::FitSelection => Some(GraphAction::FitSelection),
            Button::FitAll => Some(GraphAction::FitAll),
            Button::Hold => Some(GraphAction::Hold),
            Button::Linear => Some(GraphAction::Linear),
            Button::AutoBezier => Some(GraphAction::AutoBezier),
            Button::EasyEase => Some(GraphAction::EasyEase),
            Button::EaseIn => Some(GraphAction::EasyEaseIn),
            Button::EaseOut => Some(GraphAction::EasyEaseOut),
            _ => None,
        }
    }

    /// The range after a press on the button.
    fn pressed(self, range: &GraphRange, curves: &[GraphCurve], selection: &Selection<KeyframeId>) -> GraphRange {
        let mut next = *range;
        match self {
            Button::Value => next.mode = GraphMode::Value,
            Button::Speed => next.mode = GraphMode::Speed,
            Button::SelectedOnly => next.selected_only = !next.selected_only,
            Button::AutoZoom => next.auto_zoom = !next.auto_zoom,
            Button::Reference => next.reference = !next.reference,
            Button::Separate => next.separate_dimensions = !next.separate_dimensions,
            Button::FitSelection => {
                if let Some((low, high)) = fit_selection(curves, selection, range.mode) {
                    (next.low, next.high, next.auto_zoom) = (low, high, false);
                }
            }
            Button::FitAll => next.auto_zoom = true,
            _ => {}
        }
        next
    }
}

/// What a drag holds from its first pass to its last. The value range stays what it was when a key or handle drag
/// began, so auto-zoom does not rescale the plot under the pointer.
#[derive(Clone, Debug, Default)]
enum Hold {
    #[default]
    Idle,
    Keys {
        range: (f64, f64),
    },
    Handle {
        time: Tick,
        value: f64,
        broken: bool,
        range: (f64, f64),
    },
    Marquee(MarqueeSelect<KeyframeId>),
}

/// The style of the graph editor.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GraphEditorStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    well: StyleColor,
    /// A key; the on state is a selected key's.
    key: Stateful<StyleColor>,
    handle: StyleColor,
    playhead: StyleColor,
    marquee_fill: StyleColor,
    marquee_edge: StyleColor,
    bar_fill: StyleColor,
    red: StyleColor,
    green: StyleColor,
    blue: StyleColor,
    yellow: StyleColor,
    label: TextLook,
    /// The height of the bottom bar and the side of its buttons.
    button: f32,
    /// The side of a key's square, and the diameter of a handle's dot.
    key_side: f32,
    /// A key's and a handle's pointer reach.
    hit: f32,
    /// Between the buttons of a group, and between groups.
    gap: f32,
    section_gap: f32,
    /// Between the plot's edge and a value label.
    pad: f32,
    /// The reference graph's dashes.
    dash: f32,
    dash_gap: f32,
}

impl GraphEditorStyle {
    /// The colour of the `index`-th dimension: red, green, blue, yellow, again.
    fn dimension(&self, index: usize) -> Color32 {
        [self.red, self.green, self.blue, self.yellow][index % 4].egui()
    }
}

/// The value / speed graph editor, After Effects' (EffectCraft).
///
/// - Value grid lines at a nice step ([`nice_step`]) with labels; time lines follow the timeline's ruler; the
///   playhead as a vertical line.
/// - Curves from the app's samples; keys as squares, selected ones filled with their handles drawn (a line and a
///   dot).
/// - The selection is the app's [`Selection`] of keys (shared with the keyframe lanes), changed through the
///   [`SelectionChange`] in [`GraphEditorOutput::selection`]: a click replaces, Shift+click toggles (After Effects'),
///   a drag on empty graph sweeps a [`MarqueeSelect`] box (Shift adds). Dragging selected keys moves them in time
///   and value (Shift constrains to the axis moved most), reported as a [`DragEvent<GraphKeyMove>`]; dragging a
///   handle reports a [`DragEvent<HandleMove>`]. Esc cancels a drag.
/// - Ctrl/Cmd+wheel (and a pinch) zooms the value range about its middle and turns auto-zoom off; auto-zoom fits the
///   shown curves ([`auto_range`]) and holds the range steady while a key or handle drag runs.
/// - With [`GraphRange::reference`] and [`reference`](Self::reference) curves, the other graph type is drawn faint
///   behind the curves, scaled to the plot.
/// - The bottom bar: Value / Speed graph, selected only, auto-zoom, fit selection, fit all, reference graph,
///   separate dimensions, Hold, Linear, Auto Bezier, Easy Ease, Ease In, Ease Out; icon buttons with tooltips, one
///   Tab stop.
/// - Keyboard: the plot is one Tab stop; the keys and handles are pointer-only. Arrows move the selected keys (a
///   frame / a value step), F9, Shift+F9 and Ctrl+Shift+F9 report the Easy Ease actions; Delete and Backspace report
///   [`GraphAction::Delete`]. Every chord matches its modifiers exactly.
/// - AccessKit: the plot is a group named by the label with the selected keys' count as its value.
#[derive(Component, Props)]
#[component(content_painter)]
pub struct GraphEditor<'a> {
    #[common]
    common: Common,
    range: Bind<'a, GraphRange>,
    curves: &'a [GraphCurve],
    selection: &'a Selection<KeyframeId>,
    /// The curves of the other graph type, which [`GraphRange::reference`] shows faint (the speed curves under the
    /// value graph, and the reverse). The app samples them as it does `curves`.
    #[prop]
    reference: &'a [GraphCurve],
    /// The playhead time, drawn as a vertical line.
    #[prop]
    playhead: Tick,
    picked: Option<SelectionChange<KeyframeId>>,
    keys: Option<DragEvent<GraphKeyMove>>,
    handle: Option<DragEvent<HandleMove>>,
    actions: Vec<GraphAction>,
    shown: Selection<KeyframeId>,
    frozen: Option<(f64, f64)>,
    sweep: Option<Rect>,
}

impl<'a> GraphEditor<'a> {
    /// A graph editor named `label` for `curves` with the app's key `selection` and the graph `range`, which the
    /// editor changes (zoom, mode, options). The time axis comes from the `TimelineView` around it. Show it with
    /// `show_at` over the whole editor, plot and bar.
    pub fn new(label: impl Into<Label>, range: Bind<'a, GraphRange>, curves: &'a [GraphCurve], selection: &'a Selection<KeyframeId>) -> Self {
        GraphEditor {
            common: Common::new(),
            range,
            curves,
            selection,
            reference: &[],
            playhead: Tick::ZERO,
            picked: None,
            keys: None,
            handle: None,
            actions: Vec::new(),
            shown: Selection::default(),
            frozen: None,
            sweep: None,
        }
        .label(label)
        .ltr_axis()
    }

    /// The plot and the bar inside `rect`.
    fn split(rect: Rect, style: &GraphEditorStyle) -> (Rect, Rect) {
        let bar = Rect::from_min_max(Pos2::new(rect.left(), (rect.bottom() - style.button).max(rect.top())), rect.max);
        (Rect::from_min_max(rect.min, Pos2::new(rect.right(), bar.top())), bar)
    }

    /// The bottom bar's buttons that fit in `bar`, each with its place in [`Button::ALL`] and its cell.
    fn bar_cells(bar: Rect, style: &GraphEditorStyle) -> Vec<(usize, Button, Rect)> {
        let (mut x, mut previous) = (bar.left(), None::<u8>);
        let mut cells = Vec::new();
        for (at, button) in Button::ALL.into_iter().enumerate() {
            if let Some(group) = previous {
                x += if group == button.group() { style.gap } else { style.section_gap };
            }
            previous = Some(button.group());
            let cell = Rect::from_min_size(Pos2::new(x, bar.top()), Vec2::splat(style.button)).intersect(bar);
            x += style.button;
            if cell.width() > 0.0 {
                cells.push((at, button, cell));
            }
        }
        cells
    }

    /// The keys: a click picks, a drag moves the selected keys.
    fn press_keys(&mut self, cx: &mut Cx<'_, Self>, axis: &TimeAxis, frame: &Frame, left: f32, shown_range: (f64, f64)) {
        let keys: Vec<(usize, KeyframeId)> = self.keys_in_order().map(|(n, key)| (n, key.id)).collect();
        for (n, id) in keys {
            let Some(result) = cx.part_if_declared(KEY.at(n as u64)) else { continue };
            let press = result.press;
            if !primary(&press) {
                continue;
            }
            let additive = press.mods.shift || press.mods.cmd;
            match press.phase {
                PressPhase::DragStart { .. } => {
                    if !self.selection.contains(&id) {
                        self.picked = Some(if additive { SelectionChange::Toggle(id) } else { SelectionChange::Set(vec![id]) });
                    }
                    *cx.press_slot::<Hold>() = Some(Hold::Keys { range: shown_range });
                    self.keys = Some(DragEvent::Begin(GraphKeyMove::default()));
                }
                PressPhase::Drag { start, pos, .. } | PressPhase::Release { start, pos, click: false, .. } => {
                    let (time_ok, value_ok) = axes(press.mods.shift, pos.x - start.origin.x, pos.y - start.origin.y);
                    let delta_time = if time_ok { axis.tick_at(pos.x, left) - axis.tick_at(start.origin.x, left) } else { Tick::ZERO };
                    let delta_value = if value_ok { frame.value_at(pos.y) - frame.value_at(start.origin.y) } else { 0.0 };
                    let moved = GraphKeyMove { delta_time, delta_value };
                    self.keys =
                        Some(if matches!(press.phase, PressPhase::Release { .. }) { DragEvent::Commit(moved) } else { DragEvent::Update(moved) });
                }
                // Shift toggles a key as Cmd/Ctrl does (After Effects').
                PressPhase::Release { click: true, .. } => {
                    self.picked = Some(if additive { SelectionChange::Toggle(id) } else { SelectionChange::Set(vec![id]) });
                }
                PressPhase::Cancel { .. } => self.keys = Some(DragEvent::Cancel),
                _ => {}
            }
        }
    }

    /// The handles: a drag moves one, or with Alt breaks the pair.
    fn press_handles(&mut self, cx: &mut Cx<'_, Self>, hold: &Hold, axis: &TimeAxis, frame: &Frame, left: f32, shown_range: (f64, f64)) {
        let handles: Vec<(usize, u64, KeyframeId, (Tick, f64))> = self
            .keys_in_order()
            .flat_map(|(n, key)| {
                [(0, key.in_handle), (1, key.out_handle)].into_iter().filter_map(move |(side, handle)| handle.map(|handle| (n, side, key.id, handle)))
            })
            .collect();
        for (n, side, id, (time, value)) in handles {
            let Some(result) = cx.part_if_declared(HANDLE.at(handle_index(n, side))) else { continue };
            let press = result.press;
            if !primary(&press) {
                continue;
            }
            let (grab_time, grab_value, broken) = match hold {
                Hold::Handle { time, value, broken, .. } => (*time, *value, *broken),
                _ => (time, value, press.mods.alt),
            };
            let at = |dt: Tick, dv: f64| HandleMove { key: id, side: side_of(side), time: grab_time + dt, value: grab_value + dv, broken };
            match press.phase {
                PressPhase::DragStart { .. } => {
                    *cx.press_slot::<Hold>() = Some(Hold::Handle { time, value, broken: press.mods.alt, range: shown_range });
                    self.handle = Some(DragEvent::Begin(HandleMove { key: id, side: side_of(side), time, value, broken: press.mods.alt }));
                }
                PressPhase::Drag { start, pos, .. } | PressPhase::Release { start, pos, click: false, .. } => {
                    let (time_ok, value_ok) = axes(press.mods.shift, pos.x - start.origin.x, pos.y - start.origin.y);
                    let dt = if time_ok { axis.tick_at(pos.x, left) - axis.tick_at(start.origin.x, left) } else { Tick::ZERO };
                    let dv = if value_ok { frame.value_at(pos.y) - frame.value_at(start.origin.y) } else { 0.0 };
                    self.handle = Some(if matches!(press.phase, PressPhase::Release { .. }) {
                        DragEvent::Commit(at(dt, dv))
                    } else {
                        DragEvent::Update(at(dt, dv))
                    });
                }
                PressPhase::Cancel { .. } => self.handle = Some(DragEvent::Cancel),
                _ => {}
            }
        }
    }

    /// The value range the plot shows: the frozen one while a drag runs, else the fitted one with auto-zoom, else the
    /// range's own.
    fn shown_range(&self, range: Option<&GraphRange>) -> (f64, f64) {
        let range = range.copied().unwrap_or_default();
        self.frozen.unwrap_or_else(|| if range.auto_zoom { auto_range(self.curves, range.mode) } else { (range.low, range.high) })
    }

    /// The curves' keys in drawing order.
    fn keys_in_order(&self) -> impl Iterator<Item = (usize, &GraphKey)> {
        self.curves.iter().flat_map(|curve| curve.keys.iter()).enumerate()
    }

    /// The curves' keys in drawing order, each with its curve.
    fn keys_of_curves(&self) -> impl Iterator<Item = (usize, &GraphCurve, &GraphKey)> {
        self.curves.iter().flat_map(|curve| curve.keys.iter().map(move |key| (curve, key))).enumerate().map(|(n, (curve, key))| (n, curve, key))
    }
}

/// Where the plot puts values.
struct Frame {
    plot: Rect,
    low: f64,
    span: f64,
}

impl Frame {
    fn new(plot: Rect, (low, high): (f64, f64)) -> Self {
        Frame { plot, low, span: (high - low).max(f64::EPSILON) }
    }

    fn y_of(&self, value: f64) -> f32 {
        self.plot.bottom() - ((value - self.low) / self.span) as f32 * self.plot.height()
    }

    fn value_at(&self, y: f32) -> f64 {
        self.low + f64::from(self.plot.bottom() - y) / f64::from(self.plot.height().max(f32::EPSILON)) * self.span
    }
}

/// The index of the handle part of the `n`-th key on `side`.
fn handle_index(n: usize, side: u64) -> u64 {
    2 * n as u64 + side
}

fn side_of(side: u64) -> HandleSide {
    if side == 0 { HandleSide::In } else { HandleSide::Out }
}

/// Whether the press is the primary button's.
fn primary(press: &PressState) -> bool {
    press.phase.start().is_some_and(|start| start.button == craft_behavior::PointerButton::Primary)
}

/// Which axes a drag may move on (time, value): both, or with Shift the one the pointer moved more along (EffectCraft
/// `graph.rs:853`).
fn axes(shift: bool, dx: f32, dy: f32) -> (bool, bool) {
    if shift { (dx.abs() >= dy.abs(), dy.abs() > dx.abs()) } else { (true, true) }
}

/// `selection` after `change`.
fn applied(selection: &Selection<KeyframeId>, change: &SelectionChange<KeyframeId>) -> Selection<KeyframeId> {
    let mut next = selection.clone();
    match change {
        SelectionChange::Set(items) => {
            next.items.clone_from(items);
            next.lead = items.last().copied();
            next.anchor = items.first().copied();
        }
        SelectionChange::Toggle(key) => {
            if let Some(at) = next.items.iter().position(|held| held == key) {
                next.items.remove(at);
            } else {
                next.items.push(*key);
            }
            next.lead = Some(*key);
            next.anchor = Some(*key);
        }
        SelectionChange::Clear => next = Selection::default(),
        _ => {}
    }
    next
}

impl Component for GraphEditor<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_chrome_key(false).with_deletes().with_claims(&CLAIMED);
    type Style = GraphEditorStyle;
    type Output = GraphEditorOutput;

    fn style(cx: &StyleCx<'_>, _: GraphEditorStyleKey) -> GraphEditorStyle {
        let c = &cx.tokens.color;
        let tokens = cx.tokens;
        GraphEditorStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            well: cx.color(c.surface.sunken),
            key: {
                let (normal, selected) = (cx.color(c.text.normal), cx.color(c.accent.solid));
                Stateful::new(normal).on(States::new(selected, selected, selected, selected))
            },
            handle: cx.color(c.data.yellow),
            playhead: cx.color(c.accent.solid),
            marquee_fill: cx.color(c.accent.soft),
            marquee_edge: cx.color(c.accent.border),
            bar_fill: cx.color(c.surface.panel),
            red: cx.color(c.data.red),
            green: cx.color(c.data.green),
            blue: cx.color(c.data.blue),
            yellow: cx.color(c.data.yellow),
            label: TextLook::new(VALUE_ROLE, Stateful::new(cx.color(c.text.faint))),
            button: cx.control().height,
            key_side: tokens.size.handle.sm,
            hit: tokens.size.hit.target,
            gap: tokens.space.gap.inline,
            section_gap: tokens.space.gap.section,
            pad: tokens.space.gap.inline,
            dash: tokens.stroke.dash,
            dash_gap: tokens.stroke.dash_gap,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(0.0, cx.style.button)
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS | Sense::WHEEL_ZOOM
    }

    fn bind(&mut self) -> Option<BindMut<'_, GraphRange>> {
        Some(self.range.reborrow())
    }

    /// The plot, the keys and handles above it, and the bottom bar's buttons as one roving group.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let (plot, bar) = Self::split(rect, cx.style);
        let frame = Frame::new(plot, self.shown_range(cx.value()));
        let hit = cx.style.hit;
        let selection = self.selection;
        cx.part(PLOT, plot).sense(Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS).pointer_only("the keyframe lanes pick keys by keyboard");
        for (n, key) in self.keys_in_order() {
            let centre = Pos2::new(axis.x_of(key.time, plot.left()), frame.y_of(key.value));
            if !plot.expand(hit).contains(centre) {
                continue;
            }
            cx.part(KEY.at(n as u64), Rect::from_center_size(centre, Vec2::splat(hit)))
                .sense(Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .cursor(CursorIcon::Grab)
                .pointer_only("the arrow keys move the selected keys");
            if selection.contains(&key.id) {
                for (side, handle) in [(0, key.in_handle), (1, key.out_handle)] {
                    let Some((time, value)) = handle else { continue };
                    let at = Pos2::new(axis.x_of(time, plot.left()), frame.y_of(value));
                    cx.part(HANDLE.at(handle_index(n, side)), Rect::from_center_size(at, Vec2::splat(hit)))
                        .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                        .priority(HitPriority::new(HitLevel::Handle, 1))
                        .cursor(CursorIcon::Crosshair)
                        .pointer_only("the interpolation buttons shape the handles");
                }
            }
        }
        let names: Vec<String> = {
            let words = cx.words::<GraphEditorWords>();
            Button::ALL.iter().map(|button| button.name(&words).to_owned()).collect()
        };
        for (at, _, cell) in Self::bar_cells(bar, cx.style) {
            cx.part(BAR.at(at as u64), cell).sense(Sense::CLICK | Sense::FOCUS).focusable().tooltip(Label::from(names[at].clone()));
        }
        cx.roving(Roving::new(NavShape::Flat));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let (plot, _) = Self::split(cx.rect, cx.style);
        let range_now = cx.value().copied().unwrap_or_default();
        let hold = cx.press_slot::<Hold>().clone().unwrap_or_default();
        self.frozen = match &hold {
            Hold::Keys { range } | Hold::Handle { range, .. } => Some(*range),
            _ => None,
        };
        let shown_range = self.shown_range(Some(&range_now));
        let frame = Frame::new(plot, shown_range);
        let left = plot.left();
        let step = nice_step((shown_range.1 - shown_range.0) / GRID_LINES);
        self.shown = self.selection.clone();

        self.press_keys(cx, &axis, &frame, left, shown_range);

        self.press_handles(cx, &hold, &axis, &frame, left, shown_range);

        // The plot itself: a click on empty graph clears, a drag sweeps a box.
        let press = cx.part(PLOT).press;
        if primary(&press) {
            match press.phase {
                PressPhase::DragStart { start, .. } => {
                    *cx.press_slot::<Hold>() = Some(Hold::Marquee(MarqueeSelect::begin(start.origin, press.mods, self.selection.items.clone())));
                }
                PressPhase::Drag { pos, .. } | PressPhase::Release { pos, click: false, .. } => {
                    if let Hold::Marquee(mut marquee) = hold.clone() {
                        marquee.update(pos);
                        let centres: Vec<(KeyframeId, Rect)> = self
                            .keys_in_order()
                            .map(|(_, key)| (key.id, Rect::from_center_size(Pos2::new(axis.x_of(key.time, left), frame.y_of(key.value)), Vec2::ZERO)))
                            .collect();
                        let items = marquee.selection(centres);
                        self.sweep = matches!(press.phase, PressPhase::Drag { .. }).then(|| marquee.rect());
                        if items != self.selection.items {
                            self.picked = Some(if items.is_empty() { SelectionChange::Clear } else { SelectionChange::Set(items) });
                        }
                        *cx.press_slot::<Hold>() = Some(Hold::Marquee(marquee));
                    }
                }
                PressPhase::Cancel { .. } => {
                    if let Hold::Marquee(marquee) = hold.clone() {
                        let base = marquee.cancel();
                        if base != self.selection.items {
                            self.picked = Some(if base.is_empty() { SelectionChange::Clear } else { SelectionChange::Set(base) });
                        }
                    }
                }
                PressPhase::Release { click: true, .. } if !(press.mods.shift || press.mods.cmd) && !self.selection.items.is_empty() => {
                    self.picked = Some(SelectionChange::Clear);
                }
                _ => {}
            }
        }
        if let Some(change) = &self.picked {
            self.shown = applied(self.selection, change);
        }

        // Ctrl/Cmd+wheel (and a pinch) zooms the value range about its middle and ends auto-zoom.
        if let Some(wheel) = cx.take_wheel()
            && wheel.zoom != 1.0
            && wheel.zoom.is_finite()
            && wheel.zoom > 0.0
        {
            let factor = 1.0 / f64::from(wheel.zoom);
            let (low, high) = shown_range;
            let middle = (low + high) / 2.0;
            let mut next = range_now;
            (next.low, next.high, next.auto_zoom) = (middle - (middle - low) * factor, middle + (high - middle) * factor, false);
            cx.set(next);
        }

        // The bottom bar.
        for (at, button) in Button::ALL.into_iter().enumerate() {
            if cx.part_activated(BAR.at(at as u64)).is_none() {
                continue;
            }
            let next = button.pressed(&range_now, self.curves, &self.shown);
            if next != range_now {
                cx.set(next);
            }
            if let Some(action) = button.action() {
                self.actions.push(action);
            }
        }

        // The keys: arrows move the selected keys, F9 the Easy Ease family, Delete deletes.
        if let Some((direction, _)) = cx.intents().take_nudge().filter(|(_, mods)| mods.is_none())
            && !self.shown.items.is_empty()
        {
            let delta_time = if direction.x == 0.0 { Tick::ZERO } else { Tick(axis.frame().0 * if direction.x < 0.0 { -1 } else { 1 }) };
            self.keys = Some(DragEvent::Set(GraphKeyMove { delta_time, delta_value: -f64::from(direction.y) * step }));
        }
        for (chord, action) in
            [(EASY_EASE_OUT, GraphAction::EasyEaseOut), (EASY_EASE_IN, GraphAction::EasyEaseIn), (EASY_EASE, GraphAction::EasyEase)]
        {
            if cx.take_claimed(chord) {
                self.actions.push(action);
            }
        }
        let forward = cx.intents().take_exact(&Intent::delete(true, Granularity::Unit));
        let back = cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
        if (forward || back) && !self.shown.items.is_empty() {
            self.actions.push(GraphAction::Delete);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let (plot, bar) = Self::split(cx.rect, style);
        let draw = ui.draw();
        draw.fill(&PaintShape::Rect(plot, CornerRadius::ZERO), style.well);
        let Some(axis) = time_axis(&cx.env) else { return };
        let range = cx.value().copied().unwrap_or_default();
        let shown_range = self.shown_range(Some(&range));
        let frame = Frame::new(plot, shown_range);
        let step = nice_step((shown_range.1 - shown_range.0) / GRID_LINES);
        let x_of = |time: Tick| axis.x_of(time, plot.left());
        let painter = ui.content_painter(cx);

        // The grid: time lines at the ruler's labelled ticks, value lines at the nice step.
        let mut grid = GridLines::default();
        grid.extent = Some(kurbo::Rect::new(f64::from(plot.left()), f64::from(plot.top()), f64::from(plot.right()), f64::from(plot.bottom())));
        let ladder = ruler_ladder(axis.format.rate.timecode_base(), style.button);
        grid.x.major = ruler_ticks(&axis, plot.width(), ladder)
            .iter()
            .filter(|tick| matches!(tick.rank, TickRank::Major))
            .map(|tick| f64::from(x_of(tick.at)))
            .collect();
        let numbers = cx.env.units();
        let decimals = if step >= 1.0 { 0 } else { (-step.log10()).ceil() as usize };
        let color = cx.resolve(&style.label.color);
        let mut labels = Vec::new();
        let mut line = (shown_range.0 / step).ceil() * step;
        while line <= shown_range.1 && labels.len() < craft_ui::primitives::MAX_LINES {
            let y = frame.y_of(line);
            grid.y.major.push(f64::from(y));
            labels.push((y, numbers.number(line, decimals..=decimals)));
            line += step;
        }
        painter.grid_lines(&grid, &GridInk::new());
        for (y, text) in labels {
            let layout = draw.layout(&TextRun::new(text).role(style.label.role).color(color), f32::INFINITY);
            draw.galley(Pos2::new(plot.left() + style.pad, y - layout.size().y), &layout);
        }

        // The reference graph, faint and dashed, scaled to the plot.
        let reference = if range.reference { self.reference } else { &[] };
        let values = || reference.iter().flat_map(|curve| curve.samples.iter().map(|sample| sample.1));
        let (low, high) = values().fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| (low.min(value), high.max(value)));
        if low.is_finite() {
            let span = (high - low).max(1e-9);
            let free = plot.height() * REFERENCE_PAD;
            for (index, curve) in reference.iter().enumerate() {
                let ink = curve.color.unwrap_or_else(|| style.dimension(index)).gamma_multiply(REFERENCE_FADE);
                let points: Vec<Pos2> = curve
                    .samples
                    .iter()
                    .map(|&(time, value)| Pos2::new(x_of(time), plot.bottom() - ((value - low) / span) as f32 * (plot.height() - 2.0 * free) - free))
                    .collect();
                painter.dashed(&points, Dashes::new(style.dash, style.dash_gap), StrokeWidth::Medium, ink);
            }
        }

        // The curves, keys and handles.
        let mut next_color = 0;
        for curve in self.curves {
            let ink = curve.color.unwrap_or_else(|| {
                next_color += 1;
                style.dimension(next_color - 1)
            });
            let points: Vec<Pos2> = curve.samples.iter().map(|&(time, value)| Pos2::new(x_of(time), frame.y_of(value))).collect();
            painter.outline_in(&[PaintShape::Path(points)], ink);
            for key in &curve.keys {
                let centre = Pos2::new(x_of(key.time), frame.y_of(key.value));
                if !plot.expand(style.hit).contains(centre) {
                    continue;
                }
                let on = self.shown.contains(&key.id);
                if on {
                    for handle in [key.in_handle, key.out_handle].into_iter().flatten() {
                        let end = Pos2::new(x_of(handle.0), frame.y_of(handle.1));
                        draw.stroke(&PaintShape::Path(vec![centre, end]), StrokeWidth::Thin, style.handle);
                        draw.fill(&PaintShape::Circle(end, style.key_side / 2.0), style.handle);
                    }
                }
                let square = PaintShape::Rect(Rect::from_center_size(centre, Vec2::splat(style.key_side)), CornerRadius::ZERO);
                draw.fill(&square, style.key.resolve(State::Rest, Flags::empty(), StateMix::new(0.0, 0.0, if on { 1.0 } else { 0.0 })));
                painter.outline_in(&[square], ink);
            }
        }
        if let Some(sweep) = self.sweep {
            let area = PaintShape::Rect(sweep.intersect(plot), CornerRadius::ZERO);
            draw.fill(&area, style.marquee_fill);
            draw.stroke(&area, StrokeWidth::Thin, style.marquee_edge);
        }
        let x = x_of(self.playhead);
        draw.stroke(&PaintShape::Path(vec![Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())]), StrokeWidth::Thin, style.playhead);

        // The bottom bar, over whatever the curves drew past the plot.
        draw.fill(&PaintShape::Rect(bar, CornerRadius::ZERO), style.bar_fill);
        draw.rule(RuleKind::Divider, bar.left_top(), bar.right_top());
        let words = cx.words::<GraphEditorWords>();
        for (at, button, _) in Self::bar_cells(bar, style) {
            let icon = IconSpec::svg(IconId::new(button.icon()));
            let name = Label::from(button.name(&words).to_owned());
            match button.on(&range) {
                Some(on) => ui.face(BAR.at(at as u64), ToggleButton::new(name, Bind::given(on)).icon(icon).icon_only()),
                None => ui.face(BAR.at(at as u64), IconButton::new(icon).label(name)),
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        access.value(self.shown.items.len().to_string());
        let words = cx.words::<GraphEditorWords>();
        for (at, button) in Button::ALL.into_iter().enumerate() {
            if cx.part_if_declared(BAR.at(at as u64)).is_some() {
                access.node(BAR.at(at as u64), Role::BUTTON).name = Name::Text(button.name(&words).to_owned());
            }
        }
        let Some(axis) = time_axis(&cx.env) else { return };
        for (n, curve, key) in self.keys_of_curves() {
            let name = format!("{} {}", curve.name, format_time(key.time, &axis.format));
            if cx.part_if_declared(KEY.at(n as u64)).is_some() {
                access.node(KEY.at(n as u64), Role::GRAPHICS_OBJECT).name = Name::Text(name.clone());
            }
            for (side, handle_word) in [(0, &words.handle_in), (1, &words.handle_out)] {
                if cx.part_if_declared(HANDLE.at(handle_index(n, side))).is_some() {
                    access.node(HANDLE.at(handle_index(n, side)), Role::GRAPHICS_OBJECT).name =
                        Name::Text(format!("{name} {}", handle_word.as_str()));
                }
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> GraphEditorOutput {
        GraphEditorOutput {
            range: cx.edit_response(),
            selection: self.picked,
            keys_moved: self.keys,
            handle_moved: self.handle,
            actions: self.actions,
        }
    }
}

/// The value range around the selected keys (and their handles), in Fit Selection to View: a lone key also frames its
/// neighbours, and the speed graph frames the curve between the first and last selected key (EffectCraft
/// `graph.rs:740`).
pub fn fit_selection(curves: &[GraphCurve], selection: &Selection<KeyframeId>, mode: GraphMode) -> Option<(f64, f64)> {
    let mut bounds: Option<(f64, f64)> = None;
    let mut include = |value: f64| {
        bounds = Some(bounds.map_or((value, value), |(low, high)| (low.min(value), high.max(value))));
    };
    for curve in curves {
        let chosen: Vec<usize> = (0..curve.keys.len()).filter(|&at| selection.contains(&curve.keys[at].id)).collect();
        for &at in &chosen {
            let key = &curve.keys[at];
            include(key.value);
            for (_, value) in key.in_handle.iter().chain(key.out_handle.iter()) {
                include(*value);
            }
        }
        let (Some(&first), Some(&last)) = (chosen.first(), chosen.last()) else { continue };
        let (from, to) = if first == last { (first.saturating_sub(1), (last + 1).min(curve.keys.len() - 1)) } else { (first, last) };
        if first == last {
            include(curve.keys[from].value);
            include(curve.keys[to].value);
        }
        if mode == GraphMode::Speed {
            let (start, end) = (curve.keys[from].time, curve.keys[to].time);
            for &(_, value) in curve.samples.iter().filter(|(time, _)| *time >= start && *time <= end) {
                include(value);
            }
        }
    }
    let (low, high) = bounds?;
    let margin = ((high - low) * FIT_MARGIN).max(FIT_MARGIN_MIN);
    Some((low - margin, high + margin))
}

/// A grid step near `raw`: 1, 2, 5 or 10 times the power of ten below `|raw|` (cut-offs 1.5, 3.5, 7.5); `|raw|` at least
/// 1e-9.
// effectcraft: crates/ui-egui/src/panels/graph.rs:805 `nice_step`.
pub fn nice_step(raw: f64) -> f64 {
    let raw = raw.abs().max(1e-9);
    let magnitude = 10f64.powf(raw.log10().floor());
    let scaled = raw / magnitude;
    magnitude
        * if scaled < 1.5 {
            1.0
        } else if scaled < 3.5 {
            2.0
        } else if scaled < 7.5 {
            5.0
        } else {
            10.0
        }
}

/// The value range that shows `curves` (samples, keys and handles) with an 8 % margin each side: 0…100 without
/// values, at least 0 at the bottom for `mode` Speed, a flat range one unit high.
// effectcraft: crates/ui-egui/src/panels/graph.rs:328 `auto_range`.
pub fn auto_range(curves: &[GraphCurve], mode: GraphMode) -> (f64, f64) {
    let mut values = curves.iter().flat_map(|curve| {
        let keys = curve.keys.iter().flat_map(|key| [Some(key.value), key.in_handle.map(|handle| handle.1), key.out_handle.map(|handle| handle.1)]);
        curve.samples.iter().map(|sample| sample.1).chain(keys.flatten())
    });
    let Some(first) = values.next() else { return (0.0, 100.0) };
    let (mut low, mut high) = values.fold((first, first), |(low, high), value| (low.min(value), high.max(value)));
    if mode == GraphMode::Speed {
        low = low.min(0.0);
    }
    if (high - low).abs() < 1e-9 {
        high = low + 1.0;
    }
    let margin = (high - low) * AUTO_MARGIN;
    (low - margin, high + margin)
}

/// What the graph editor did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct GraphEditorOutput {
    /// The value range: a wheel zoom's or a button's change.
    pub range: EditResponse<GraphRange>,
    /// The selection change a click or box selection made, for the app to apply to its selection.
    pub selection: Option<SelectionChange<KeyframeId>>,
    /// The key move gesture, one update per frame and a commit on release; a `Set` for an arrow key.
    pub keys_moved: Option<DragEvent<GraphKeyMove>>,
    /// The handle move gesture, one update per frame and a commit on release.
    pub handle_moved: Option<DragEvent<HandleMove>>,
    /// This pass's actions, in order.
    pub actions: Vec<GraphAction>,
}

impl Deref for GraphEditorOutput {
    type Target = EditResponse<GraphRange>;

    fn deref(&self) -> &Self::Target {
        &self.range
    }
}

impl Outcome for GraphEditorOutput {
    type Value = GraphRange;

    fn response(&self) -> &Response {
        &self.range.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.range.story_facts()
    }
}
