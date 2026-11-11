//! The pass core: `CxCore`, `Cx` over it, their stage access and the inherent methods of `Cx`.

use super::parts::Toggle;
use super::pointer::Pointer;
use super::scroll::ScrollMove;
use crate::contract::{Component, Resolved, Validity, ValueOf};
use crate::primitives::sensing::PartResults;
use crate::primitives::{PaintRecord, TextLayout};
use crate::runtime::commands::{self, CommandRef, FireSource, ResolvedCommand};
use crate::runtime::env::{Env, EnvView};
use crate::runtime::focus::{self};
use crate::runtime::keys::Intents;
use crate::runtime::memory::{MemoryStage, StageAccess, sealed};
use crate::style::{Ink, StateSet};
use craft_behavior::{Cursor, CursorPriority, EditEvent, Flow, PartKey, PartRef, PressCancel, PressState, Sense};
use craft_keymap::Chord;
use egui::{Context, Id, Rect, Response};
use std::any::Any;
use std::cell::RefCell;
use std::fmt::Debug;
use std::ops::{Deref, Range};
use std::sync::Arc;

/// How a component was activated this pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Activation {
    /// By `Intent::Activate` (Enter, Space or an AccessKit click) rather than a primary click.
    pub by_key: bool,
    /// The modifiers held at the activation: with the click for a pointer activation, in the pass for a key or AccessKit one.
    pub mods: craft_keymap::Mods,
}

/// The text part's result of a pass as the core holds it: `Cx::text` reads the typed outcome, the response reads
/// the rest without the value type.
pub(crate) trait TextResult {
    /// The modifiers of the Enter that submitted the part this pass; none when it did not submit.
    fn submit_mods(&self) -> craft_keymap::Mods;
    /// Whether Tab (`Some(true)`) or Shift+Tab (`Some(false)`) moved focus off the part this pass.
    fn left_by_tab(&self) -> Option<bool>;
    fn as_any(&self) -> &dyn Any;
}

pub(crate) struct RecordedText<V> {
    pub(crate) outcome: crate::primitives::TextOutcome<V>,
    pub(crate) submit_mods: craft_keymap::Mods,
    pub(crate) left_by_tab: Option<bool>,
}

impl<V: 'static> TextResult for RecordedText<V> {
    fn submit_mods(&self) -> craft_keymap::Mods {
        self.submit_mods
    }

    fn left_by_tab(&self) -> Option<bool> {
        self.left_by_tab
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The non-generic part of every `Cx`; the step functions take this. It is a read-only output: component code
/// changes nothing here except through a chapter trait.
#[non_exhaustive]
pub struct CxCore<'a> {
    /// What the inherited context holds that component code may read.
    pub env: EnvView<'a>,
    /// `Common` after step 1.
    pub resolved: &'a Resolved<'a>,
    /// The component's id.
    pub id: Id,
    /// The allocated rect.
    pub rect: Rect,
    /// The rect minus padding, mirrored for right-to-left.
    pub content_rect: Rect,
    /// The rect intersected with the clip rect.
    pub visible: Rect,
    /// Start and end geometry of the rect.
    pub flow: Flow,
    /// The root widget's response.
    pub response: Response,
    /// The bound command, resolved at step 1.
    pub command: Option<Arc<ResolvedCommand>>,
    /// Written by step 12; read with `Styled::state`.
    pub(crate) state: StateSet,
    /// True once step 12 wrote `state` in this pass.
    pub(crate) stated: bool,
    /// Read with `Styled::ink`, valid from step 12.
    pub(crate) ink: Ink,
    /// An owned copy of the intents routed to this id.
    #[doc(hidden)]
    pub intents: Intents,
    #[doc(hidden)]
    pub memory: &'a mut MemoryStage,
    #[doc(hidden)]
    pub ctx: Context,
    pub(crate) paint: RefCell<PaintRecord>,
    pub(crate) results: PartResults,
    /// A face reads idle results for parts it has not registered.
    pub(crate) face_mode: bool,
    /// The active part while the component holds focus.
    pub(crate) active: Option<PartRef>,
    /// What focus holds of this component, read once at step 8.
    #[doc(hidden)]
    pub focus: crate::runtime::focus::Facts,
    /// The arena root this component is a member of; `None` for a component with its own egui widget.
    pub(crate) member_of: Option<Id>,
    pub(crate) activation: Option<Activation>,
    /// The activation `Cx::activate` injected this pass, which the pipeline runs as it runs a click.
    pub(crate) injected: Option<FireSource>,
    /// The activation leaves the layer chain open (`Cx::keep_open`).
    pub(crate) keep_open: bool,
    /// An AccessKit click named a part this pass: the response reports it as a click, as a pointer click on the part does.
    pub(crate) part_clicked: bool,
    /// For an arena root: the member activated this pass.
    pub(crate) activated_member: Option<Id>,
    pub(crate) press: PressState,
    pub(crate) cursor: Option<(Cursor, CursorPriority)>,
    /// The declared cursors of the parts hovered this pass, recorded at step 8 and written at step 16.
    pub(crate) part_cursors: Vec<Cursor>,
    pub(crate) claims: Vec<Chord>,
    pub(crate) reveal: Option<Rect>,
    pub(crate) press_cancel: Option<PressCancel>,
    /// The visible range each scroll part moved to this pass, from step 8's results.
    pub(crate) scroll_moves: Vec<ScrollMove>,
    /// The parts that show the value, whose double click resets it like the root's.
    pub(crate) value_parts: Vec<(PartKey, Option<u64>)>,
    /// The value parts that declared `own_reset`.
    pub(crate) own_reset_parts: Vec<(PartKey, Option<u64>)>,
    /// The parts that declared `toggles`, with whether their popup is open as of step 9.
    #[doc(hidden)]
    pub toggles: Vec<Toggle>,
    /// The part a reset targeted that declared `own_reset`; step 9 leaves its value to `update`.
    pub(crate) reset_part: Option<PartRef>,
    /// The sense the root's egui widget carries, for the forwarding widget of a transformed body.
    pub(crate) root_sense: Sense,
    /// The member of the container's roving group Tab arrives at, set by the group's pick.
    pub(crate) roving_arrival: Option<Id>,
    /// The member of the container's roving group that moves start from, set by `Cx::roving_lead`.
    pub(crate) roving_lead: Option<Id>,
    /// The collection's selection (a `Selection<K>`) and lead as the item step holds them, for `Cx::collection_selection`.
    pub(crate) collection_selection: Option<Box<dyn std::any::Any>>,
    /// The events and visible range the item step left for `collection_response`.
    pub(crate) collection_events: Option<Box<dyn std::any::Any>>,
    pub(crate) visible_rows: Range<usize>,
    /// What the collection's props allow, for `collection_response`.
    pub(crate) gestures: crate::contract::collection::Gestures,
    /// The text part's result of this pass, from step 8b.
    pub(crate) texts: Option<(PartKey, Box<dyn TextResult>)>,
    /// Where the text part reserved the surface in the painter, so the field's surface paints beneath its text.
    pub(crate) surface_slot: Option<egui::layers::ShapeIdx>,
    /// The editor part as step 8b set it up.
    pub(crate) editor: Option<crate::contract::editor::EditorInfo>,
    /// The editor part's inputs of this pass, until `editor_input` takes them.
    pub(crate) editor_inputs: Vec<craft_behavior::EditorInput>,
    /// The text part's draft is not a value of its codec.
    pub(crate) text_invalid: bool,
    /// The row and column counts of the collection, which the item step records for its node.
    pub(crate) grid: crate::runtime::access::GridPlace,
    /// The text part's completion list is open: Escape belongs to its popup, not to the component's gesture.
    pub(crate) escape_to_list: bool,
    /// The text facts step 16 gives AccessKit, from the text adapter or the editor's `editor_state`.
    pub(crate) text_access: RefCell<Option<crate::runtime::access::text::TextFacts>>,
    /// The value as the component words it (`Component::value_text`), noted at step 16 or in a sizing pass for the
    /// output.
    pub(crate) value_text: RefCell<Option<String>>,
    /// Enter was pressed in the text part.
    pub(crate) submitted: bool,
    /// Escape abandoned the text part's edit.
    pub(crate) escaped: bool,
}

impl<C: Component> Cx<'_, C> {
    /// The part a reset aimed at, when it declared `own_reset`; `update` proposes the value with that part at its
    /// default. Reads `None` when the reset was the whole binding's, which step 9 applied.
    pub fn reset_part(&self) -> Option<PartRef> {
        self.core.reset_part
    }

    /// Lays document `text` out `text.wrap` wide: the call `ContentPainterCx::layout` and `MeasureCx::layout_doc`
    /// make, so `update` finds a character at a point in the galley that `paint` draws.
    pub fn layout_doc(&self, text: &crate::primitives::DocText<'_>) -> TextLayout {
        text.layout(&self.core.ctx, &Env::of_root(&self.core.ctx).tokens, craft_i18n::Direction::Ltr)
    }

    /// Asks the platform to open `url`, in a new tab where the platform has tabs and `new_tab` is set.
    pub fn open_url(&mut self, url: &str, new_tab: bool) {
        crate::runtime::egui::output::open_url(&self.core.ctx, url, new_tab);
    }

    /// Makes this component a member of the roving group of the body it is shown in: its stop joins the group and
    /// Tab visits the group once. A component that never calls it keeps its own Tab stop.
    pub fn join_roving(&mut self) {
        if let Some(group) = self.core.env.roving() {
            crate::contract::steps::join_group(&self.core.ctx, group, self.core.id);
            focus::registration::join_part_group(&self.core.ctx, group, self.core.id, crate::runtime::memory::sub(self.core.id, "roving"));
        }
    }

    /// Runs `command` as this component's own activation would, for a command that is not the component's bound
    /// one: resolved against the instances above the component, and not fired when disabled. `source` is where the
    /// activation came from (`part_activated` gives it). Returns whether the command was queued.
    pub fn fire(&mut self, command: impl Into<CommandRef>, source: FireSource) -> bool {
        let Some(resolved) = commands::resolve_scoped(&self.core.ctx, self.core.env.command_chain(), &command.into()) else { return false };
        commands::fire_resolved(&self.core.ctx, &resolved, source)
    }

    /// Where an activation of `part` came from this pass: `Key` for a keyboard activation while `part` is the
    /// active part, `Pointer` for a click on it, `Access` for an AccessKit click on its sub-node; `None` when
    /// `part` was not activated. A part that is not declared is never activated.
    pub fn part_activated(&self, part: impl Into<PartRef>) -> Option<FireSource> {
        let part = part.into();
        let result = self.core.part_if_declared(part)?;
        if self.core.activation.is_some_and(|activation| activation.by_key) && self.core.active_now() == Some(part) {
            Some(FireSource::Key)
        } else if result.press.phase.is_click() {
            Some(FireSource::Pointer)
        } else if self.core.intents.access_click(part) {
            Some(FireSource::Access)
        } else {
            None
        }
    }

    /// Names the member of this container's roving group (`ContainerProps.roving`) that Tab arrives at; the
    /// first member when none is named.
    pub fn roving_arrival(&mut self, member: Id) {
        self.core.roving_arrival = Some(member);
    }

    /// Names the member of this container's roving group that the next move starts from, in place of the focused one:
    /// a menu's row that the pointer lit. The group registers after `update`, so a member named from the body's state
    /// as `update` reads it takes effect from the next pass.
    #[doc(hidden)]
    pub fn roving_lead(&mut self, member: Id) {
        self.core.roving_lead = Some(member);
    }

    /// Moves focus to one of this component's focusable parts, in this pass: the rest of the pass reads the new
    /// focus. For `update` after a press on the component or while it holds focus; it never takes focus from elsewhere.
    pub fn focus_part(&mut self, part: impl Into<PartRef>) {
        let part = part.into();
        debug_assert!(
            self.core.focus.focused || self.core.press_held() || self.core.results.values().any(|result| result.press.phase.is_press()),
            "`focus_part` is for a component that holds focus or was just pressed; `update` never steals focus"
        );
        focus::request(&self.core.ctx, focus::FocusTarget::Part(self.core.id, part));
        self.core.sync_focus();
    }

    /// Moves focus to the component's own stop (not a part), under the rule of `focus_part`.
    pub fn focus_own(&mut self) {
        debug_assert!(
            self.core.focus.focused || self.core.press_held(),
            "`focus_own` is for a component that holds focus or was just pressed; `update` never steals focus"
        );
        focus::request(&self.core.ctx, focus::FocusTarget::Id(self.core.id));
        self.core.sync_focus();
    }
}

impl sealed::Sealed for CxCore<'_> {}

impl StageAccess for CxCore<'_> {
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

/// What component code reads and calls. It derefs to [`CxCore`] for reading only.
///
/// ```compile_fail,E0594
/// # extern crate craft_ui_contract as craft_ui;
/// use craft_ui::contract::{Component, Cx};
/// fn update<C: Component>(cx: &mut Cx<'_, C>) {
///     cx.rect = egui::Rect::NOTHING;
/// }
/// ```
#[non_exhaustive]
pub struct Cx<'a, C: Component> {
    #[doc(hidden)]
    pub core: CxCore<'a>,
    /// The component's cached style.
    pub style: &'a C::Style,
    /// The shown copy of the value, shared with shown memory.
    pub(crate) shown: Option<Arc<ValueOf<C>>>,
    /// The binding holds a mixed value; `shown` is empty.
    #[doc(hidden)]
    pub mixed: bool,
    /// The edit events of this pass, in order; `applied` of them have reached the binding.
    pub(crate) events: Vec<EditEvent<ValueOf<C>>>,
    pub(crate) applied: usize,
    /// What the binding's check says about the shown value.
    pub(crate) validity: Validity<ValueOf<C>>,
}

impl<'a, C: Component> Deref for Cx<'a, C> {
    type Target = CxCore<'a>;

    fn deref(&self) -> &CxCore<'a> {
        &self.core
    }
}

impl<C: Component> sealed::Sealed for Cx<'_, C> {}

impl<C: Component> StageAccess for Cx<'_, C> {
    fn stage(&self) -> &MemoryStage {
        self.core.memory
    }

    fn stage_mut(&mut self) -> &mut MemoryStage {
        self.core.memory
    }

    fn base(&self) -> Id {
        self.core.id
    }
}

/// An env derived for children `PaintUi` builds: the composite's resolved `enabled` and `size`.
pub(crate) fn child_env(env: &Env, enabled: bool, size: craft_theme::ControlSize) -> Env {
    env.derive_local(|scoped| {
        scoped.enabled &= enabled;
        scoped.size = size;
    })
}
