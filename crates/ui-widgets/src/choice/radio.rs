//! `RadioGroup` and `Radio`: the group form of 20.3. The group is a container and arena root; its members, shown
//! anywhere in its body, join its arena and register with its `GroupHandle`. A radio draws only its button; its text
//! is the label step's, as the check box's is.

use crate::contract::{
    Bind, BindMut, Common, Component, Container, ContainerProps, Cx, Edit as _, EditResponse, GroupHandle, GroupKind, GroupState, Input, MeasureCx,
    PaintUi, Pointer as _, Props as _, Remember as _, Role, Styled as _, Validity,
};
use crate::primitives::{PaintShape, StrokeWidth, glyph};
use crate::runtime::env::EnvValue;
use crate::runtime::focus::{ActivateKeys, KeyUse};
use crate::style::{ComponentStyle, Flags, FocusLook, StyleColor, StyleCx};
use craft_behavior::{Metrics, NavShape, Roving, Value};
use craft_theme::{Stateful, States, variant::Tone};
use egui::{CornerRadius, Rect, Vec2};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

/// The group kind of radios whose members stand for values of type `T`.
pub struct RadioKind<T>(PhantomData<fn() -> T>);

impl<T: Value + 'static> GroupKind for RadioKind<T> {
    type Member = T;
}

/// Whether the group's value is refused, read by its members so each radio shows the invalid look.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct GroupInvalid(bool);

impl EnvValue for GroupInvalid {
    fn root() -> Self {
        GroupInvalid(false)
    }
}

/// The group's registrations, kept in its shown memory so the members registered last pass are readable this pass.
struct GroupMemory<T: Value + 'static> {
    live: Arc<Mutex<GroupState<T>>>,
}

impl<T: Value + 'static> Default for GroupMemory<T> {
    fn default() -> Self {
        GroupMemory { live: Arc::default() }
    }
}

impl<T: Value + 'static> Clone for GroupMemory<T> {
    fn clone(&self) -> Self {
        GroupMemory { live: Arc::clone(&self.live) }
    }
}

/// The style of the group: it has no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GroupStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A group of radio buttons over one value: one Tab stop, arrows move focus, Space or a click selects.
#[derive(crate::contract::Component)]
#[component(container, probe(RadioGroup<'static, u8>))]
pub struct RadioGroup<'a, T: Value + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    value: Bind<'a, T>,
}

impl<'a, T: Value + 'static> RadioGroup<'a, T> {
    /// A group named `label` over `value`.
    pub fn new(label: impl Into<crate::runtime::env::Label>, value: Bind<'a, T>) -> Self {
        let props = ContainerProps::new().arena(true).roving(Roving::new(NavShape::Flat));
        RadioGroup { common: Common::new(), props, value }.label(label)
    }

    /// How the arrows move through the members: `Flat` by default; a group that wraps its members in rows takes
    /// `Spatial`, so Up and Down move to the member above or below.
    pub fn nav(mut self, nav: NavShape) -> Self {
        self.props = self.props.roving(Roving::new(nav));
        self
    }
}

impl<T: Value + 'static> Component for RadioGroup<'_, T> {
    const ROLE: Role = Role::RADIO_GROUP;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Space);
    const PARTS: bool = false;
    type Style = GroupStyle;
    type Output = EditResponse<T>;

    fn style(_: &StyleCx<'_>, _: RadioGroupStyleKey<T>) -> GroupStyle {
        GroupStyle::default()
    }

    fn sense(&self) -> craft_behavior::Sense {
        craft_behavior::Sense::empty()
    }

    fn bind(&mut self) -> Option<BindMut<'_, T>> {
        Some(self.value.reborrow())
    }

    /// The pick (20.3 item 1): the member activated this pass, found in last pass's registrations, goes to the
    /// binding. Tab arrives at the selected member.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let memory = cx.memory::<GroupMemory<T>>().clone();
        let handle = GroupHandle::<RadioKind<T>>::new(Arc::clone(&memory.live));
        let activated = cx.activated_member();
        if let Some(picked) = activated.and_then(|id| handle.member(id)) {
            cx.propose(picked);
            cx.commit();
        }
        let selected = cx.value().cloned();
        if let Some(id) = selected.as_ref().and_then(|value| handle.member_id(value)) {
            cx.roving_arrival(id);
        }
        handle.select(selected);
    }

    fn content_env(&self, cx: &Cx<'_, Self>, values: &mut crate::runtime::env::AppValues) {
        let handle = GroupHandle::<RadioKind<T>>::new(Arc::clone(&cx.recall::<GroupMemory<T>>().live));
        // Step 13 runs right before the body: the members the body shows register again, and a pass that runs no
        // body leaves the registrations of the last one. A measuring run registers no member, so it leaves them too.
        if !crate::runtime::measure::active(&cx.core.ctx) {
            handle.begin_registration();
        }
        values.set(handle);
        values.set(GroupInvalid(matches!(cx.validity(), Validity::Refuse(_))));
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<T> {
        cx.edit_response()
    }
}

impl<T: Value + 'static> Container for RadioGroup<'_, T> {}

/// The style of a radio button.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct RadioStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    icon: f32,
    /// The circle's fill.
    fill: Stateful<StyleColor>,
    /// The circle's outline.
    ring: Stateful<StyleColor>,
    /// The mark inside a selected radio.
    dot: Stateful<StyleColor>,
}

/// A member of a `RadioGroup`: selected when the group's pick is its value.
#[derive(crate::contract::Component)]
#[component(probe(Radio<u8>))]
pub struct Radio<T: Value + 'static> {
    #[common]
    common: Common,
    value: T,
    checked: bool,
    invalid: bool,
}

impl<T: Value + 'static> Radio<T> {
    /// A radio button standing for `value`.
    pub fn new(value: T) -> Self {
        Radio { common: Common::new(), value, checked: false, invalid: false }
    }
}

impl<T: Value + 'static> Component for Radio<T> {
    const ROLE: Role = Role::RADIO_BUTTON;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Space);
    const LABELLED_AFTER: bool = true;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    type Style = RadioStyle;
    type Output = egui::Response;

    fn style(cx: &StyleCx<'_>, _: RadioStyleKey<T>) -> RadioStyle {
        let control = cx.control();
        let colors = &cx.tokens.color;
        let (solid, bad) = (cx.color(colors.accent.solid), cx.tone(Tone::Bad));
        let disabled = cx.color(colors.text.disabled);
        RadioStyle {
            metrics: Metrics::new().min_size(Vec2::new(0.0, control.height)),
            focus: cx.focus_look(),
            icon: control.icon,
            fill: super::well(cx).on(States::new(solid, solid, solid, disabled)),
            ring: super::outline(cx).on(States::new(solid, solid, solid, disabled)).when(Flags::INVALID, bad),
            dot: super::mark(cx),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.icon())
    }

    /// Reads "selected" and "invalid" from the group and registers with it.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let group = cx.env.get::<GroupHandle<RadioKind<T>>>();
        group.register(self.value.clone(), cx.id, cx.resolved.enabled);
        cx.join_roving();
        self.checked = group.is(&self.value);
        self.invalid = cx.env.get::<GroupInvalid>().0;
    }

    fn flags(&self) -> Flags {
        let mut flags = Flags::empty();
        flags.set(Flags::CHECKED, self.checked);
        flags.set(Flags::INVALID, self.invalid);
        flags
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let center = cx.rect.center();
        let draw = ui.draw();
        // A rectangle with full corner radius strokes inside its bounds, so the ring keeps the cell's width.
        let circle = PaintShape::Rect(Rect::from_center_size(center, Vec2::splat(cx.style.icon)), CornerRadius::from(cx.style.icon / 2.0));
        draw.fill(&circle, cx.resolve(&cx.style.fill));
        draw.stroke(&circle, StrokeWidth::Medium, cx.resolve(&cx.style.ring));
        if self.checked {
            draw.fill(&glyph::dot(center, cx.style.icon), cx.resolve(&cx.style.dot));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

#[cfg(feature = "testing")]
mod stories;
