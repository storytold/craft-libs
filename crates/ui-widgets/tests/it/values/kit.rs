//! A value component for the trials: a track that scrubs an `f32`, written with the derives.

use craft_behavior::{CancelReason, EditEvent, EditEvents, GesturePhase, PartKey, PressPhase, Sense};
use craft_ui::contract::{Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, PartsCx, Pointer as _, Role, Styled as _};
use craft_ui::style::{ComponentStyle, Flags, StyleCx};
use egui::{Pos2, Rect, Vec2};
use std::cell::Cell;

const TRACK: PartKey = PartKey::new("track");

/// A style with no look: the trials read events and values, not pixels.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PlainStyle {
    #[style(metrics)]
    metrics: craft_behavior::Metrics,
}

/// Where the component sits and what a track position means: the value is the pointer's offset from the left edge.
pub const AT: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(110.0, 30.0) };

/// Values the trials start from and drag to, named so the assertions compare relations. A drag along the track ends at
/// `DRAGGED`; the held drag of the mixed-composite trial stops at `HELD`.
pub const INITIAL: f32 = 10.0;

pub const APP_VALUE: f32 = 3.0;

pub const CAP: f32 = 30.0;

pub const DRAGGED: f32 = 50.0;

pub const HELD: f32 = 45.0;

const TOLERANCE: f32 = 1.5;

/// A value within pixel rounding of `expected`.
pub fn near(value: f32, expected: f32) -> bool {
    (value - expected).abs() < TOLERANCE
}

/// What the component recorded while it ran.
#[derive(Default)]
#[non_exhaustive]
pub struct Seen {
    pub flags: Cell<Flags>,
    pub shown: Cell<Option<f32>>,
    pub phase: Cell<Option<GesturePhase>>,
}

/// Scrubs a value by dragging along its track; a click without a drag does nothing.
#[derive(craft_ui::contract::Component)]
pub struct Scrub<'a> {
    #[common]
    common: Common,
    value: Bind<'a, f32>,
    seen: Option<&'a Seen>,
}

impl<'a> Scrub<'a> {
    pub fn new(value: Bind<'a, f32>) -> Self {
        Scrub { common: Common::new(), value, seen: None }
    }

    pub fn seen(mut self, seen: &'a Seen) -> Self {
        self.seen = Some(seen);
        self
    }
}

impl Component for Scrub<'_> {
    const ROLE: Role = Role::SLIDER;
    type Style = PlainStyle;
    type Output = EditResponse<f32>;

    fn style(_: &StyleCx<'_>, _: ScrubStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        AT.size()
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(TRACK, rect).sense(Sense::CLICK | Sense::DRAG).pointer_only("the track is dragged with the pointer");
    }

    fn bind(&mut self) -> Option<BindMut<'_, f32>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let left = cx.rect.left();
        let phase = cx.part(TRACK).press.phase;
        match phase {
            PressPhase::DragStart { pos, .. } => {
                cx.begin();
                cx.propose(pos.x - left);
            }
            PressPhase::Drag { pos, .. } => cx.propose(pos.x - left),
            PressPhase::Release { .. } if cx.gesture().is_some() => cx.commit(),
            _ => {}
        }
    }

    fn paint(&mut self, _: &mut craft_ui::contract::PaintUi<'_>, cx: &Cx<'_, Self>) {
        if let Some(seen) = self.seen {
            seen.flags.set(cx.state().flags);
            seen.shown.set(cx.value().copied());
            seen.phase.set(cx.gesture());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<f32> {
        cx.edit_response()
    }
}

/// What a response's events were, for the gesture trials.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Ev {
    Begin,
    Update(f32),
    Commit(f32),
    Cancel(CancelReason),
    Set(f32),
}

pub fn ev_of(events: &EditEvents<f32>) -> Vec<Ev> {
    events
        .iter()
        .filter_map(|event| {
            Some(match event {
                EditEvent::Begin { .. } => Ev::Begin,
                EditEvent::Update { value, .. } => Ev::Update(*value),
                EditEvent::Commit { after, .. } => Ev::Commit(*after),
                EditEvent::Cancel { reason, .. } => Ev::Cancel(*reason),
                EditEvent::Set { after, .. } => Ev::Set(*after),
                _ => return None,
            })
        })
        .collect()
}
