//! The component traits, context types, `PaintUi`, `Looks`, the pipeline and its step functions, the label, body and item steps, the mount and overlay hosts and `contract::tooltip`.

mod body;
mod collection;
mod container;
mod cx;
pub(crate) mod editor;
mod entry;
mod faces;
pub mod ghost;
mod group;
mod held;
mod items;
mod label;
mod lazy;
#[cfg(feature = "testing")]
mod level;
#[cfg(feature = "testing")]
mod level_table;
mod looks;
mod mount;
pub mod overlay;
mod paint_ui;
mod props;
mod registry;
#[doc(hidden)]
pub mod reorder;
mod run;
mod sampler;
mod settle;
mod shell;
mod steps;
pub(crate) mod text;
pub mod toast;
pub mod tooltip;
mod values;

pub use crate::layout::{BodyLayout, RowAlign};
pub use crate::primitives::FacesCx;
pub use crate::runtime::access::{Access, FieldLabel, GridPlace, Name, Numeric, Politeness, PopupKind, Role, SetPosition, SubNode, announce};
pub use crate::runtime::commands::{CommandSet, Commands};
pub use crate::runtime::keys::MenuKey;
pub use crate::runtime::memory::{Remember, Savable, SaveKey, Slot};
pub use crate::runtime::motion::{Animate, Interpolate, MotionSpec, Presence, Tween, spec};
pub use crate::runtime::pending::{Job, Pending, Progress};
#[doc(hidden)]
pub use body::REORDER_BAND;
#[doc(hidden)]
pub use body::pad_of;
pub use collection::{
    ActivateOn, CollectionEvent, CollectionProps, CollectionResponse, ColumnAlign, ColumnEvent, ColumnKind, ColumnSort, ColumnSpec,
    ContextMenuRequest, DragPayload, HeaderMenuRequest, ItemKeys, LeadOnOpen, LeadSource, ListDrag, OpenChange, Rename, ReorderMove, ReorderRule,
    RowState, SelectMode, ShownRow, SpaceAction, ToggleColumn, ToggleEdit,
};
#[doc(hidden)]
pub use container::InBar;
#[doc(hidden)]
pub use container::InMenu;
pub use container::{
    Collapse, ContainerProps, DragBand, EndRegion, FooterPlace, FooterRegion, Gap, HeaderRegion, LeastWidth, Mount, Notch, Pad, PaneEdge, Scroll,
    ScrollAnchor, ScrollAxes, ShellPane, Sizing, ThemeScope,
};
pub use craft_ui_macros::{CommandSet, Component, Pattern, Props, register_contract};
#[doc(hidden)]
pub use cx::THUMB;
pub use cx::{
    Accessible, Activation, BodyCx, Cx, CxCore, Edit, Focus, ItemCx, ItemOpener, MeasureCx, PartBuilder, PartsCx, Pointer, Styled, WordSetCx,
};
pub use entry::Entry;
pub use group::{GroupHandle, GroupKind, GroupState};
pub use held::Held;
#[doc(hidden)]
pub use items::lead_position;
#[doc(hidden)]
pub use items::put_toggle_names;
pub use items::{ColumnCommands, SweepApply, SweepStep, request_rename, section_part};
#[doc(hidden)]
pub use label::colon_of;
pub use label::{CellWidth, FieldLabelRole, FieldLayout, GlyphLabel, LabelAlign, LabelKind, Scrub, SideLabel};
pub use lazy::Lazy;
#[cfg(feature = "testing")]
pub(crate) use level::kit_module;
#[doc(hidden)]
#[cfg(feature = "testing")]
pub use level::{Catalog, CatalogEntry, Level, StoryLayout, Surface, catalog, story_title};
pub use looks::Looks;
pub use overlay::{Overlay, OverlayUi, Overlays, Popup, PopupHandle, SizeLimits, Trigger};
pub use paint_ui::PaintUi;
pub use props::{Common, Props, Resolved, Tooltip, TooltipAt, TooltipSpec, TooltipWhen};
#[doc(hidden)]
#[cfg(feature = "testing")]
pub use registry::registrations;
#[doc(hidden)]
pub use registry::{CollectionEntry, ContainerEntry, Exemption, LeafEntry, PatternKind, PatternProbe, Probe, Registration, ShowEntry};
pub use sampler::Sampler;
pub use settle::SettleCx;
pub use values::{Bind, BindMut, EditResponse, ItemOutput, NoValue, Outcome, Validity, ValueOf, clicked};

use crate::runtime::memory::sub;
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, Flags, StyleCx, StyleKey};
pub use craft_behavior::Input;
use craft_behavior::{CellSpace, ItemLayout, ItemSource, Proposal, ScrollPos, SelectionModel, Sense};
use egui::{Id, IdSalt, Rect, Ui, Vec2};
use run::ShowAt;
use std::hash::Hash;
use std::ops::Deref;

pub use crate::runtime::drag::{DragPreview, DropMark, DropTarget, OsFile, OsFiles, WindowDropTarget};
#[cfg(feature = "testing")]
pub(crate) use body::MOVE_BAND;
pub use craft_behavior::ColumnKey;
pub use craft_behavior::DropOperation;
pub(crate) use run::measure_under;
pub(crate) use steps::keeping_ids;
#[cfg(feature = "testing")]
pub(crate) use steps::queue_reveal;
pub(crate) use steps::{peek_reveal, take_reveal};

/// Adds the runtime to `ctx`: `runtime::install` with the overlay host's ghost-layer painter in the end hook.
pub fn install(ctx: &egui::Context, spec: crate::runtime::InstallSpec) -> Result<(), crate::runtime::InstallError> {
    crate::runtime::install(ctx, spec.frame_layers(Some(overlay::paint_frame_layers)))
}

/// Where an element's id comes from.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub enum IdSource {
    /// Derived from the parent scope and the order of its siblings.
    #[default]
    Auto,
    /// A salt added to the parent scope's id.
    Salt(IdSalt),
    /// The caller's id.
    Id(Id),
}

/// The shapes of component: which show trait applies.
pub mod shape {
    /// A component with no body and no items.
    #[derive(Clone, Copy, Debug)]
    pub struct Leaf;
    /// A component whose caller supplies a body closure.
    #[derive(Clone, Copy, Debug)]
    pub struct Body;
    /// A component that shows an item per visible entry of a source.
    #[derive(Clone, Copy, Debug)]
    pub struct Items;
}

mod sealed {
    pub trait ShowShape {}
    impl ShowShape for super::shape::Leaf {}
    impl ShowShape for super::shape::Body {}
    impl ShowShape for super::shape::Items {}

    pub trait Show {}
    pub trait ShowContainer {}
    pub trait ShowCollection {}
}

/// The shape of a component, as a value: what a registration records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShapeKind {
    /// No body and no items.
    Leaf,
    /// The caller supplies a body.
    Body,
    /// An item per visible entry of a source.
    Items,
}

/// Which show trait applies, and the shape's default sense.
pub trait ShowShape: sealed::ShowShape {
    /// The shape as a value.
    const KIND: ShapeKind;
    /// Leaf `CLICK | FOCUS`; body empty; items `CLICK | DRAG | FOCUS | WHEEL | TAKE_FOCUS`.
    const DEFAULT_SENSE: Sense;
}

impl ShowShape for shape::Leaf {
    const KIND: ShapeKind = ShapeKind::Leaf;
    const DEFAULT_SENSE: Sense = Sense::CLICK.union(Sense::FOCUS);
}

impl ShowShape for shape::Body {
    const KIND: ShapeKind = ShapeKind::Body;
    const DEFAULT_SENSE: Sense = Sense::empty();
}

impl ShowShape for shape::Items {
    const KIND: ShapeKind = ShapeKind::Items;
    const DEFAULT_SENSE: Sense = Sense::CLICK.union(Sense::DRAG).union(Sense::FOCUS).union(Sense::TAKE_FOCUS);
}

/// A token only craft-ui can build: the private field and `pub(crate)` constructor let derive output in any
/// crate implement the accessors below, while only craft-ui can call them.
#[derive(Debug)]
pub struct Internal(());

impl Internal {
    pub(crate) const fn new() -> Self {
        Internal(())
    }
}

/// Written by `#[derive(Component)]`; never by hand.
pub trait Meta {
    /// The shape: leaf, body or items.
    type Shape: ShowShape;
    /// The instance inputs the style depends on.
    type StyleKey: StyleKey;
    /// A leaf the `Canvas` body shows (`#[component(canvas_control)]`): its default sense is empty, so a press away
    /// from its parts reaches the tool, and its type is a canvas control.
    const CANVAS_CONTROL: bool = false;

    /// The style key of this instance.
    fn style_key(&self) -> Self::StyleKey;
    #[doc(hidden)]
    fn common(&self, _: Internal) -> &Common;
    #[doc(hidden)]
    fn common_mut(&mut self, _: Internal) -> &mut Common;
}

/// Written by the derive from a `#[container]` field, for containers and collections.
pub trait HasProps {
    #[doc(hidden)]
    fn props(&self, _: Internal) -> &ContainerProps;
}

/// The container show output: the component's own output and the body's result.
#[derive(Debug)]
pub struct Inner<O, R> {
    /// The component's own output.
    pub output: O,
    /// The body's result; `None` when the body did not run. A body may run twice in a pass (the measuring run, then
    /// the real run), and `inner` is the last run's value. A state change or a report made from it goes in
    /// [`Inner::settle`], which runs once per pass after the real run.
    pub inner: Option<R>,
    #[doc(hidden)]
    pub settle: Option<settle::SettleAt>,
}

impl<O, R> Inner<O, R> {
    /// Runs `f` once, after the pass's real run, with the body's memory (`keep`, `memory`, `saved`, `pending`,
    /// `take`, as the body has it) and the real run's result. It is not called when no real run ran the body (an
    /// inert container, a sizing parent, a closed gate) or when the body returned nothing. Writes made in `f` show
    /// from the next pass.
    pub fn settle<T>(&mut self, f: impl FnOnce(&mut SettleCx<'_>, &mut R) -> T) -> Option<T> {
        let at = self.settle.take()?;
        let inner = self.inner.as_mut()?;
        Some(at.run(|cx| f(cx, inner)))
    }

    /// Maps the component's own output and keeps the body's result and its settle handle.
    pub fn map_output<O2>(self, f: impl FnOnce(O) -> O2) -> Inner<O2, R> {
        Inner { output: f(self.output), inner: self.inner, settle: self.settle }
    }
}

impl<O, R> Deref for Inner<O, R> {
    type Target = O;

    fn deref(&self) -> &O {
        &self.output
    }
}

/// The most a component may measure on either side, in points.
pub const MAX_EXTENT: f32 = 32_768.0;

/// A type that only assembles components: a composition with no contract shape of its own (24.4).
///
/// A pattern has no pipeline, no style, no sense and no props of the contract. It shows components, and the
/// AccessKit node of its root container is the pattern's node. `#[derive(Pattern)]` writes this impl and registers
/// the type, so the components index lists it and the suite runs the cases that apply to what it shows: the
/// container and collection cases of its contents, and the role and name checks of every node. A type is a
/// `Component` or a `Pattern`, never both: a type with its own sensing, painting, popups or focus is a component.
///
/// A canvas tool and a stack dialog are patterns of their own kind (`#[pattern(tool)]`, `#[pattern(dialog)]`): they
/// are not components, and their stories show them through a canvas (`testing::tool_story`) or
/// a dialog.
pub trait Pattern {}

/// The core trait: every UI type is a `Component`.
pub trait Component: Meta + Sized {
    /// The AccessKit role.
    const ROLE: Role;
    /// Roles `access` may pick per instance.
    const ALT_ROLES: &'static [Role] = &[];
    /// How input reaches the component.
    const INPUT: Input = Input::Pointer;
    /// Whether the component hosts popups: `false` skips the overlay bookkeeping of steps 9, 10, 12, 15 and 16.
    const OVERLAYS: bool = true;
    /// Whether the component declares parts, a text part or an editor part: `false` skips the parts step (the parts
    /// hook, the measure view it reads and the memory of the last state) and leaves the sense step with no part
    /// regions. A shape that declares parts of its own (a body's scroll parts, an item layout) still runs its hook.
    const PARTS: bool = true;
    /// Whether the component or its parts can take key focus: `false` skips the stop registration of step 7, except
    /// for the roving group of a body the shape declares.
    const STOPS: bool = true;
    /// Keys the focused component keeps from egui and the shortcut dispatcher.
    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(Self::INPUT);
    /// The keys this instance keeps in its current state. The default is `KEYS`; a component whose key use depends
    /// on its state (a field that records chords while recording) overrides it, so one component serves every state.
    fn keys(&self) -> crate::runtime::focus::KeyUse {
        Self::KEYS
    }
    /// Whether the component paints its label as visible text. `false` for a face that shows only an icon or a
    /// glyph: with no tooltip set, its label is then its tooltip, since the label is the only visible name the
    /// pointer can reach. A component whose answer varies per instance overrides `shows_label`.
    const SHOWS_LABEL: bool = true;
    /// Whether the control draws only its mark and its text is the label step's, placed after it: a check box, a
    /// radio button or a switch. `Show::show` then runs the label step for an instance that carries a label and sits
    /// in no label scope, and a click on the label in any scope activates the control as a click on it does.
    const LABELLED_AFTER: bool = false;
    /// Whether the component edits one value that several selected things cannot disagree on (a colour picker's
    /// area, strip or ramp: it edits the colour it is given). Such a type has no mixed state and no mixed story.
    /// Two sources: a canvas control (`#[component(canvas_control)]`) is single-value by kind, since it edits one
    /// layer's geometry; any other component declares it here.
    const SINGLE_VALUE: bool = false;

    /// The style type.
    type Style: ComponentStyle;
    /// `Response`, `EditResponse<T>` or a richer type.
    type Output: Outcome;

    /// Whether this instance paints its label as visible text; `SHOWS_LABEL` unless an instance prop decides, as
    /// `icon_only` does. It reads props only, never the paint record.
    fn shows_label(&self) -> bool {
        Self::SHOWS_LABEL
    }

    /// How far the component paints outside its rect by design, in points (a graph node's ports sit on its edge).
    /// Steps 13 and 14 clip to the rect grown by this much; the paint-bounds suite case reads it.
    fn paint_outset(_tokens: &crate::style::UiTokens) -> f32 {
        0.0
    }

    /// Pure function of the resolved context and the key.
    fn style(cx: &StyleCx<'_>, key: Self::StyleKey) -> Self::Style;

    /// Content size only; the pipeline adds padding and minimum size.
    fn measure(&mut self, _cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// The axis along which the component takes all the room the flow offers when that is more than it measured,
    /// as a container with `Sizing::Fill` does: a vertical rule fills its row's height.
    fn fills(&self) -> Option<craft_behavior::Axis> {
        None
    }

    /// The root's sense; the shape's default unless a canvas control.
    fn sense(&self) -> Sense {
        if Self::CANVAS_CONTROL { Sense::empty() } else { <Self::Shape as ShowShape>::DEFAULT_SENSE }
    }

    /// Named sub-rects: thumbs, chevrons, handles, segments and the roving groups over them.
    fn parts(&mut self, _rect: Rect, _cx: &mut PartsCx<'_, Self>) {}

    /// Value components return their binding; the same binding on every call, with no side effects.
    fn bind(&mut self) -> Option<BindMut<'_, ValueOf<Self>>> {
        None
    }

    /// Whether `host`, a value the app wrote back, is the value `proposed` as the component holds values: a host that
    /// stores a proposal rounded to the component's precision has not changed the document under a gesture. The
    /// gesture's host-change check compares with this.
    fn same_value(&self, proposed: &ValueOf<Self>, host: &ValueOf<Self>) -> bool {
        proposed == host
    }

    /// Lets a drag on the field's side label change the value; the pipeline runs the gesture (16.3, B2).
    fn scrub(&self) -> Option<Scrub<ValueOf<Self>>> {
        None
    }

    /// The scroll offset's owner when the app drives scrolling; `None` keeps it in session memory. The app restores a
    /// scroll position, or keeps the content under the pointer fixed across a zoom, by writing it.
    fn scroll(&mut self) -> Option<BindMut<'_, ScrollPos>> {
        None
    }

    /// App values for content the component does not build: its body or its items. Runs once, at step 13.
    fn content_env(&self, _cx: &Cx<'_, Self>, _values: &mut crate::runtime::env::AppValues) {}

    /// After sensing, before paint: value maths, memory, popup triggers.
    fn update(&mut self, _cx: &mut Cx<'_, Self>) {}

    /// Anchored popups. Runs before flags and paint.
    fn overlay(&mut self, _ui: &mut OverlayUi<'_>, _cx: &mut Cx<'_, Self>) {}

    /// Value-driven flags (checked, open, invalid, busy).
    fn flags(&self) -> Flags {
        Flags::empty()
    }

    /// When a bound command fires; the default is a primary click or `Intent::Activate`.
    fn activated(&self, cx: &Cx<'_, Self>) -> bool {
        cx.activation().is_some()
    }

    /// Content only; surface, state fill and focus ring are painted by the pipeline.
    fn paint(&mut self, _ui: &mut PaintUi<'_>, _cx: &Cx<'_, Self>) {}

    /// Content above the body or items.
    fn paint_over(&mut self, _ui: &mut PaintUi<'_>, _cx: &Cx<'_, Self>) {}

    /// Value, range, relations, sub-nodes and custom actions beyond role, label and flags.
    fn access(&self, _access: &mut Access, _cx: &Cx<'_, Self>) {}

    /// The shown value as the component words it ("40.0", "20 to 60"), which assistive technology reads and a viewer
    /// shows beside the component (`Outcome::value_text`); `None` for a component that words no value. Every pass
    /// asks it, measuring runs included, so it reads the props, the shown value and the environment only. Step 16
    /// gives it to the node, as the numeric value's text or else as the node's value.
    fn value_text(&self, _cx: &Cx<'_, Self>) -> Option<String> {
        None
    }

    /// Builds the output from the finished context.
    fn finish(self, cx: Cx<'_, Self>) -> Self::Output;
}

/// Implemented by the allowlisted components that paint content with a raw painter (a preview, a waveform).
/// `PaintUi::content_painter` takes no other component.
pub trait ContentPainter: Component {}

/// A component whose caller supplies a body.
pub trait Container: Component + HasProps + Meta<Shape = shape::Body> {
    /// The open state's owner: the caller's binding. `None`: a session slot under the id.
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        None
    }

    /// Fills the header region the body step laid out.
    fn header(&mut self, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

    /// Fills the header's trailing region.
    fn controls(&mut self, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

    /// Fills the footer region.
    fn footer(&mut self, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

    /// Fills the end region, after the body.
    fn end(&mut self, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

    /// The container closed, and why: a layer's dismissal rule, a close the body or footer asked for with
    /// `BodyCx::request_close_as`, `overlay::request_close`, or the OS closing its window. Called once for a close,
    /// at step 15 or, for a dismissal rule, right after the pass.
    fn closed(&mut self, _why: CloseReason) {}

    /// The user moved or resized the container's layer: by the move band, a grip or the keyboard Move and Size
    /// modes. Called for each step of the gesture, so a container whose layer the caller places (`Overlay::placed`)
    /// can take the rect it reports. A viewport mount in an OS window is called once per change of the window's outer
    /// rect, with `Commit`.
    fn placement_changed(&mut self, _change: &overlay::PlacementChange) {}
}

/// A component that shows an item per visible entry of a source.
pub trait Collection: Component + HasProps + Meta<Shape = shape::Items> {
    /// The item key.
    type Key: Clone + Eq + Hash + Send + Sync + 'static;
    /// Kept in session memory so step 8 can hit-test against last frame's layout.
    type ItemLayout: ItemLayout + Clone + Send + Sync + 'static;
    /// The selection model.
    type Selection: SelectionModel<Self::Key>;
    /// The item component.
    type Item<'r>: Component<Output: ItemOutput<Self::ItemEvent>> + Meta<Shape = shape::Leaf>;
    /// Events an item reports.
    type ItemEvent: 'static;

    /// Plain data: selection mode, reorder, rename, columns.
    fn items(&self) -> &CollectionProps;

    /// The selection binding; `None` keeps it in session memory.
    fn selection(&mut self) -> Option<BindMut<'_, Self::Selection>> {
        None
    }

    /// The grid a model of cells selects in (17.7). The item step passes the grid it knows, the source's rows and the
    /// layout's columns with the rows the layout hides left out; a sheet adds its merged cells, hidden columns and
    /// locked cells from its model.
    fn cell_space<'s>(&'s self, space: CellSpace<'s, Self::Key>) -> CellSpace<'s, Self::Key> {
        space
    }

    /// A one-shot request to scroll an item into view.
    fn reveal(&self) -> Option<&Self::Key> {
        None
    }

    /// The layout of `source` at `width`.
    fn item_layout(&self, source: &dyn ItemSource<Self::Key>, width: f32, cx: &StyleCx<'_>) -> Self::ItemLayout;

    /// The layout of `source` at `width`, given the layout of the last pass. An implementor may reuse `previous`
    /// when the width and the rows stamp are the same and the source's revision has not moved; the default builds
    /// the layout anew.
    fn item_layout_from(
        &self,
        _previous: Option<Self::ItemLayout>,
        source: &dyn ItemSource<Self::Key>,
        width: f32,
        cx: &StyleCx<'_>,
    ) -> Self::ItemLayout {
        self.item_layout(source, width, cx)
    }

    /// The width the collection asks for when it takes its width from its content, over its widest row: a grid of
    /// cards asks for its declared number of columns. 0 asks for nothing.
    fn ideal_width(&self, _cx: &StyleCx<'_>) -> f32 {
        0.0
    }

    /// A view of `source` for this pass, such as the rows a filter keeps; `None` shows `source` as it is.
    fn filtered<'s>(&self, _source: &'s dyn ItemSource<Self::Key>, _env: &crate::runtime::env::Env) -> Option<Box<dyn ItemSource<Self::Key> + 's>> {
        None
    }

    /// Adjusts the item the closure built for a row, from the row's state.
    fn dress<'r>(&self, item: Self::Item<'r>, _row: &RowState) -> Self::Item<'r> {
        item
    }

    /// The rename editor at `label`: the label rect the renaming item reports (`ItemOutput::label_rect`), else its row.
    /// `name` is the item's name before the rename; the editor is named after it.
    fn rename_editor(&mut self, _ui: &mut Ui, _draft: Bind<'_, String>, _label: Rect, _name: &str) -> Option<EditResponse<String>> {
        None
    }

    /// Whether the item `key` may be renamed; a rename request on another item does nothing.
    fn can_rename(&self, _key: &Self::Key) -> bool {
        true
    }

    /// Whether the item `key` may be dragged (reordered, moved in a free layout, or dragged to another collection).
    fn can_drag(&self, _key: &Self::Key) -> bool {
        true
    }

    /// Whether a drag from the collection `from` may drop here.
    fn accept_from(&self, _from: Id) -> bool {
        true
    }

    /// Whether the dragged items may land before `insert` (a position of the display order, `over` is the item the
    /// pointer is on, none over an empty collection). A refused spot shows no drop mark.
    fn accepts(&self, _drag: &ListDrag<Self::Key>, _over: Option<&Self::Key>, _insert: usize) -> bool {
        true
    }

    /// Whether the collection takes files dragged in from the OS: a drop reports `CollectionEvent::FilesDropped` at the
    /// insert position under the pointer, with the same drop mark an internal drag gets.
    fn takes_files(&self) -> bool {
        false
    }

    /// Whether the collection takes the zoom share of the wheel (Ctrl or Cmd with the wheel): it reports
    /// `CollectionEvent::Zoom` and the app sets its zoom and, through `Component::scroll`, the scroll position that
    /// keeps the content under the pointer fixed.
    fn zooms(&self) -> bool {
        false
    }

    /// The height of the empty state at `width`: a `Sizing::Content` collection with no rows is this tall. The
    /// default is the height of `CollectionProps::empty` laid out at `width`; an override replaces the empty text, so
    /// it replaces `empty` as well.
    fn empty_height(&self, width: f32, ui: &Ui, _cx: &StyleCx<'_>) -> f32 {
        self.items().empty.as_ref().map_or(0.0, |text| items::empty_text::height(ui, text, width))
    }

    /// The empty state at the rect the item step placed. The default shows `CollectionProps::empty`.
    fn empty(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        if let Some(text) = self.items().empty.clone() {
            items::empty_text::show(ui, sub(cx.id, "empty_text"), text, cx.rect);
        }
    }
}

/// Shows a leaf.
pub trait Show: Component + Meta<Shape = shape::Leaf> + sealed::Show {
    /// Shows the component in the flow of `ui`.
    fn show(self, ui: &mut Ui) -> Self::Output {
        label::show_leaf_in_flow(self, ui)
    }

    /// Shows the component at `rect`, with no allocation.
    fn show_at(self, ui: &mut Ui, rect: Rect) -> Self::Output {
        run::show_leaf(self, ui, ShowAt::Rect(rect), None)
    }

    /// Shows the component with rich tooltip content from the same call.
    fn show_with_tip(self, ui: &mut Ui, tip: impl FnOnce(&mut Ui)) -> Self::Output {
        run::show_leaf(self, ui, ShowAt::Flow, Some(Box::new(tip)))
    }

    /// Shows the component at `rect` with rich tooltip content.
    fn show_at_with_tip(self, ui: &mut Ui, rect: Rect, tip: impl FnOnce(&mut Ui)) -> Self::Output {
        run::show_leaf(self, ui, ShowAt::Rect(rect), Some(Box::new(tip)))
    }

    /// Measures without allocating: steps 1 to 5 under the id `show` would use next. It registers no id, opens no
    /// memory stage and reads memory through a peek that moves no slot.
    fn measure_in(&mut self, ui: &Ui, proposal: Proposal) -> Vec2 {
        run::measure_under(self, ui, None, proposal, None)
    }
}

/// Shows a container with its body.
pub trait ShowContainer: Container + sealed::ShowContainer {
    /// Shows the container in the flow of `ui`.
    fn show<R>(self, ui: &mut Ui, body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Inner<Self::Output, R> {
        run::show_container(self, ui, ShowAt::Flow, body, None)
    }

    /// Shows the container at `rect`, with no allocation: a frame over a split or a layer the parent places.
    fn show_at<R>(self, ui: &mut Ui, rect: Rect, body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Inner<Self::Output, R> {
        run::show_container(self, ui, ShowAt::Rect(rect), body, None)
    }

    /// Shows the container at `rect` and takes that rect's room in the flow of `ui`: what is shown after it lies
    /// past it, as after [`show`](Self::show).
    fn show_at_taking_room<R>(self, ui: &mut Ui, rect: Rect, body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Inner<Self::Output, R> {
        run::show_container_taking(self, ui, rect, body)
    }

    /// Measures the container with `body` without allocating: steps 1 to 5 under the id `show` would use next, with
    /// the body, header, footer and end regions run once in an invisible sizing pass, laid out as `show` lays them
    /// out. The size comes from this pass's content alone; along an axis the body scrolls on it is at most the room.
    fn measure_in<R>(&mut self, ui: &Ui, proposal: Proposal, body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Vec2 {
        run::measure_container(self, ui, proposal, body)
    }

    /// Shows the container with rich tooltip content.
    fn show_with_tip<R>(self, ui: &mut Ui, body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R, tip: impl FnOnce(&mut Ui)) -> Inner<Self::Output, R> {
        run::show_container(self, ui, ShowAt::Flow, body, Some(Box::new(tip)))
    }

    /// Shows the container at `rect` with rich tooltip content.
    fn show_at_with_tip<R>(
        self,
        ui: &mut Ui,
        rect: Rect,
        body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R,
        tip: impl FnOnce(&mut Ui),
    ) -> Inner<Self::Output, R> {
        run::show_container(self, ui, ShowAt::Rect(rect), body, Some(Box::new(tip)))
    }
}

/// Shows a collection's items.
pub trait ShowCollection: Collection + sealed::ShowCollection {
    /// Shows the visible items of `source`, one `item` call each.
    fn show_items<'r>(self, ui: &mut Ui, source: &dyn ItemSource<Self::Key>, item: &mut ItemFn<'r, Self>) -> Self::Output {
        run::show_items(self, ui, source, item)
    }
}

impl<C: Component + Meta<Shape = shape::Leaf>> sealed::Show for C {}
impl<C: Container> sealed::ShowContainer for C {}
impl<C: Collection> sealed::ShowCollection for C {}
impl<C: Component + Meta<Shape = shape::Leaf>> Show for C {}
impl<C: Container> ShowContainer for C {}
impl<C: Collection> ShowCollection for C {}

/// The item closure of `ShowCollection::show_items`: one call per visible key.
pub type ItemFn<'r, C> = dyn FnMut(&<C as Collection>::Key, &mut ItemCx<'_, 'r, <C as Collection>::Key>) -> <C as Collection>::Item<'r> + 'r;

/// A tooltip closure the pipeline runs at step 16.
pub(crate) type TipFn<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;
