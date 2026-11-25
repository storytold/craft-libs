//! `BodyCx`: what a container's body closure and region hooks read.

use crate::contract::label;
use crate::contract::{Container, Resolved, Show, SideLabel};
use crate::runtime::env::{EnvView, Label, Message};
use crate::runtime::memory::{MemoryStage, StageAccess, sealed};
use crate::runtime::overlay::CloseReason;
use egui::{Id, Rect, Ui};
use std::any::Any;
use std::fmt::Debug;
use std::hash::Hash;

/// What a body closure and the header, footer and empty hooks read: a view over the pipeline's memory stage
/// keyed by the body's base id.
#[non_exhaustive]
pub struct BodyCx<'a> {
    /// The body's base id: `sub(id, "body")`.
    pub id: Id,
    /// What the inherited context holds that component code may read.
    pub env: EnvView<'a>,
    /// The container's `Common` after step 1.
    pub resolved: &'a Resolved<'a>,
    /// The body's rect.
    pub rect: Rect,
    #[doc(hidden)]
    pub memory: &'a mut MemoryStage,
    open: &'a mut bool,
    gate: Option<&'a mut bool>,
    gate_mixed: bool,
    close: Option<CloseReason>,
    exiting: bool,
    field_message: Option<(Message, craft_theme::variant::Tone)>,
    owner: Option<BodyOwner<'a>>,
}

impl<'a> BodyCx<'a> {
    #[allow(clippy::too_many_arguments, reason = "one argument per field the region's context starts from")]
    pub(crate) fn new(
        id: Id,
        env: EnvView<'a>,
        resolved: &'a Resolved<'a>,
        rect: Rect,
        memory: &'a mut MemoryStage,
        open: &'a mut bool,
        gate: Option<&'a mut bool>,
        shared: Option<RegionShare<'a>>,
    ) -> Self {
        memory.claim(id);
        let (gate_mixed, field_message, owner) =
            shared.map_or((false, None, None), |shared| (shared.gate_mixed, shared.field_message, Some(shared.owner)));
        BodyCx { id, env, resolved, rect, memory, open, gate, gate_mixed, close: None, exiting: false, field_message, owner }
    }

    /// Asks the platform to open `url`, in a new tab where the platform has tabs and `new_tab` is set: the opener
    /// `Link` uses. The footer's closure gets this context too.
    pub fn open_url(&mut self, url: &str, new_tab: bool) {
        crate::runtime::egui::output::open_url(self.memory.ctx(), url, new_tab);
    }

    /// The host file service's answer to the picker `ticket`, taken once. A measuring run gets none and leaves
    /// the answer in the service, so the real run of the body finds it.
    pub fn take_pick_answer(&self, ticket: craft_host::PickId) -> Option<craft_host::Picked> {
        if crate::runtime::measure::active(self.memory.ctx()) {
            return None;
        }
        self.env.host()?.files().take_answer(ticket)
    }

    pub(crate) fn with_exiting(mut self, exiting: bool) -> Self {
        self.exiting = exiting;
        self
    }

    /// Whether the container has closed and its body only stays on screen while it fades out. `is_open` reads true
    /// then; a region that belongs to the open container (a companion layer) is not shown.
    pub fn is_exiting(&self) -> bool {
        self.exiting
    }

    /// The validity message of the first control in the container's label scope, recorded last frame, with the
    /// tone it shows in: a container whose footer declares `FooterRegion::status` shows it as its status text. `None`
    /// when the container has no label or no control reported one.
    pub fn field_message(&self) -> Option<(Message, craft_theme::variant::Tone)> {
        self.field_message.clone()
    }

    /// Whether a drag of the container's move band (`DragBand::Move`) moves its layer in this pass, from the pass the
    /// drag starts to the one before its release: a grip in the body shows the move.
    pub fn moving(&self) -> bool {
        let band = crate::primitives::sensing::part_press(self.memory.ctx(), self.resolved.id, crate::contract::MOVE_BAND.into());
        matches!(band.press.phase, craft_behavior::PressPhase::DragStart { .. } | craft_behavior::PressPhase::Drag { .. })
    }

    /// Whether the body is open.
    pub fn is_open(&self) -> bool {
        *self.open
    }

    /// The open state: a title row's disclosure binds to it, and step 15 writes a change to its owner.
    pub fn open(&mut self) -> &mut bool {
        self.open
    }

    /// The gate's shown copy, for a container whose value is a bool: a title row's check box binds to it, and
    /// step 15 applies a change. Under a gate that is off the body is disabled.
    pub fn gate(&mut self) -> Option<&mut bool> {
        self.gate.as_deref_mut()
    }

    /// Whether the gate's value is mixed: `gate` then reads off, and a title row's check box shows the mixed face.
    pub fn gate_is_mixed(&self) -> bool {
        self.gate_mixed
    }

    /// Asks the container to close, as `CloseReason::Requested`: step 15 closes it.
    pub fn request_close(&mut self) {
        self.request_close_as(CloseReason::Requested);
    }

    /// Asks the container to close and tells it why (an OK button is `Activated`, Enter is `Enter`): step 15
    /// closes it and `Container::closed` gets `why`. The first request of a pass keeps its reason.
    pub fn request_close_as(&mut self, why: CloseReason) {
        self.close.get_or_insert(why);
    }

    pub(crate) fn close_requested(&self) -> Option<CloseReason> {
        self.close
    }

    /// Runs the label step (B2, 16.3) for one leaf child in the body's flow: `label` sits beside the child or above
    /// it, with the icon and detail of `side`, and the child takes the width the label leaves. The row is as high as
    /// the placement measured: the child (plus the label line when the label stacks), and the label when it is taller.
    /// `ui` is the body's `Ui`; `key` makes the child's id.
    pub fn labelled<C: Show>(&mut self, ui: &mut Ui, key: impl Hash + Debug, label: Label, side: SideLabel, child: C) -> C::Output {
        label::flow_leaf(ui, (self.id.with(key), self.resolved.id), (self.resolved.enabled, self.resolved.size), label, side, child, None)
    }

    /// `labelled`, with `content` shown inside the label after its text (a sample of what the option picks). The
    /// content belongs to the label: for a child that draws only its mark (a radio button, a check box) a press on it
    /// is the child's, as a press on the label is, so the content takes none itself. With a `LabelKind::Hidden` label
    /// the content is the whole label: a picture in place of the option's name.
    pub fn labelled_with<C: Show, D: Show>(
        &mut self,
        ui: &mut Ui,
        key: impl Hash + Debug,
        label: Label,
        side: SideLabel,
        child: C,
        content: D,
    ) -> C::Output {
        let id = self.id.with(key);
        let under = (self.resolved.enabled, self.resolved.size);
        let content_id = crate::runtime::memory::sub(id, "content");
        let mut content = content;
        let size = crate::contract::run::measure_under(
            &mut content,
            &*ui,
            Some(content_id),
            craft_behavior::Proposal::Ideal,
            Some(&super::Under::new(under.0, under.1)),
        );
        let show = Box::new(move |ui: &mut Ui, rect: Rect| {
            let _ = crate::contract::run::show_leaf(
                &mut Some(crate::contract::Props::id(content, content_id)),
                ui,
                crate::contract::run::ShowAt::Rect(rect),
                None,
            );
        });
        label::flow_leaf(ui, (id, self.resolved.id), under, label, side, child, Some(label::Trailing { size, show }))
    }
}

/// What a container's regions share in their `BodyCx`: whether its gate is mixed (the header's only), its field's
/// message and the container that owns them.
pub(crate) struct RegionShare<'a> {
    pub gate_mixed: bool,
    pub field_message: Option<(Message, craft_theme::variant::Tone)>,
    pub owner: BodyOwner<'a>,
}

/// What a container's body reads of its container: the style step 2 resolved for this pass, and the body gap.
#[derive(Clone, Copy)]
pub(crate) struct BodyOwner<'a> {
    style: &'a (dyn Any + Send + Sync),
    gap: f32,
}

impl<'a> BodyOwner<'a> {
    /// The container's `style` and the gap its body lays its children out with.
    pub(crate) fn new<S: crate::style::ComponentStyle>(style: &'a S, gap: f32) -> Self {
        BodyOwner { style, gap }
    }
}

impl<'a> BodyCx<'a> {
    /// The style of the container `C` whose body this is, as step 2 resolved it for this pass: the one way a body
    /// reads what its container's style holds (a dock root's docking sizes). `None` when the body's container has
    /// another style type, and for a body with no container: a popup's body or a collection's empty state.
    pub fn style<C: Container>(&self) -> Option<&'a C::Style> {
        self.owner.and_then(|owner| owner.style.downcast_ref::<C::Style>())
    }

    /// The gap the body's layout puts between its children (`ContainerProps::body_gap`), for a body that places
    /// content of its own beside them. `None` for a body with no container: a popup's body or a collection's empty
    /// state.
    pub fn gap(&self) -> Option<f32> {
        self.owner.map(|owner| owner.gap)
    }
}

impl BodyCx<'_> {
    /// The value this body derives from `key`: kept while `key` is unchanged, else built by `build`
    /// ([`runtime::derived`](crate::runtime::derived::derived), under the body's id).
    pub fn derived<K: crate::contract::MemoProps, V: Send + Sync + 'static>(&self, key: &K, build: impl FnOnce() -> V) -> std::sync::Arc<V> {
        crate::runtime::derived::derived(self.memory.ctx(), self.id, key, build)
    }

    /// Takes the Escape of this pass that no component took: a field that clears on Escape passes it on once it
    /// is empty. `true` once per Escape; the enclosing scope (a dialog, a popover) then does not act on it.
    /// Components shown earlier in the body have run by the time the body asks.
    pub fn take_cancel(&mut self) -> bool {
        crate::runtime::keys::take_cancel(self.memory.ctx())
    }
}

impl sealed::Sealed for BodyCx<'_> {}

impl StageAccess for BodyCx<'_> {
    fn stage(&self) -> &MemoryStage {
        self.memory
    }

    fn stage_mut(&mut self) -> &mut MemoryStage {
        self.memory
    }

    fn base(&self) -> Id {
        self.id
    }
}
