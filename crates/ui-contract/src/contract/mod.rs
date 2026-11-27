//! The component traits, context types, `PaintUi`, `Looks`, the pipeline and its step functions, the label, body and item steps, the mount and overlay hosts and `contract::tooltip`.

pub(crate) mod body;
mod collection;
mod container;
mod cx;
pub(crate) mod editor;
pub use editor::{EditorHandle, EditorStep};
mod entry;
mod faces;
pub(crate) mod footer_slot;
pub mod ghost;
mod group;
mod held;
mod hosted;
pub(crate) mod items;
pub(crate) mod label;
mod lazy;
#[cfg(feature = "suite")]
pub(crate) mod level;
#[cfg(feature = "suite")]
mod level_table;
mod looks;
mod memo;
mod memo_view;
mod mount;
pub mod overlay;
mod pages;
mod paint_ui;
mod props;
mod registry;
#[doc(hidden)]
pub mod reorder;
mod run;
mod sampler;
mod settle;
mod shell;
pub(crate) mod steps;
pub(crate) mod text;
pub mod toast;
pub mod tooltip;
mod values;

pub use crate::bind_tracked;
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
pub use body::grip_band;
pub use body::keep_inside;
pub use body::pad_of;
#[doc(hidden)]
pub use body::{STRIP_BACK, STRIP_FORWARD};
pub use collection::{
    ActivateOn, CollectionEvent, CollectionProps, CollectionResponse, ColumnAlign, ColumnEvent, ColumnKind, ColumnLayout, ColumnRemove,
    ColumnRequest, ColumnSort, ColumnSpec, ContextMenuRequest, DeleteTarget, DragPayload, DropAt, DropKind, DropPayload, DropPlace, EdgePress,
    HeaderMenuRequest, HoverLead, ItemKeys, LeadOnOpen, LeadSource, ListDrag, OpenChange, PendingDrop, Rename, ReorderMove, ReorderRule, RowHeight,
    RowPressed, RowPreview, RowState, SelectMode, ShownRow, SpaceAction, ToggleColumn, ToggleEdit,
};
#[doc(hidden)]
pub use container::InBar;
#[doc(hidden)]
pub use container::InMenu;
pub use container::{
    BandFill, Bracket, Collapse, ContainerProps, DragBand, EndRegion, FooterPlace, FooterRegion, Gap, HeaderEdge, HeaderParts, HeaderRegion,
    LeastWidth, Mount, MoveBandCursor, Notch, POPUP_PAD, Pad, PadEdges, PaneEdge, QuietHeader, Scroll, ScrollAnchor, ScrollAxes, ScrollIndicator,
    SectionRhythm, ShellPane, Sizing, ThemeScope, WrapWidth,
};
pub use craft_tracked::{PlainData, Revision, Tracked};
pub use craft_ui_macros::{CommandSet, Component, Pattern, PlainData, Props, register_contract};
#[doc(hidden)]
pub use cx::THUMB;
pub use cx::{
    Accessible, Activation, BodyCx, Cx, CxCore, Edit, Focus, ItemCx, ItemOpener, MeasureCx, PartBuilder, PartsCx, Pointer, Styled, WordSetCx,
};
pub use entry::{Entry, step_stop, step_stops};
#[doc(hidden)]
pub use footer_slot::FooterSlot;
pub use group::{GroupHandle, GroupKind, GroupState};
pub use held::Held;
pub use hosted::{Hosted, HostedKey, Hosting};
pub use items::item_node;
pub use items::lead_position;
pub use items::{ColumnCommands, SweepApply, SweepStep, item_part_open, request_rename, section_part};
#[doc(hidden)]
pub use items::{RowToggle, put_toggle_names};
pub use items::{measure_item, show_item, show_item_in_flow};
#[doc(hidden)]
pub use label::colon_of;
pub use label::{
    CaptionColumn, CaptionInk, CellWidth, FieldLabelRole, FieldLayout, GlyphColon, GlyphLabel, InsideLabel, LabelKind, LabelPress, LabelScheme,
    LinkLabelInk, LinkLabelUnderline, Scrub, SideLabel,
};
pub use lazy::Lazy;
#[doc(hidden)]
#[cfg(feature = "suite")]
pub use level::{Catalog, CatalogEntry, Level, StoryLayout, Surface, catalog, stale_table_rows, story_title};
pub use looks::Looks;
pub use memo::{MemoKey, MemoProps, Stateless};
pub use memo_view::{MemoView, Outbox};
pub use overlay::{Overlay, OverlayUi, Overlays, Popup, PopupHandle, SizeLimits, Trigger};
pub use pages::Pages;
pub use paint_ui::PaintUi;
pub use props::{Common, Props, Resolved, Tooltip, TooltipAt, TooltipSpec, TooltipWhen};
#[doc(hidden)]
pub use registry::{CollectionEntry, ContainerEntry, Exemption, LeafEntry, PatternKind, PatternProbe, Probe, Registration, ShowEntry};
pub use sampler::{CanvasGesture, Sampler};
pub use settle::SettleCx;
pub use text::EDIT_MENU;
pub use values::{Bind, BindCheck, BindMut, EditResponse, Hover, ItemOutput, NoValue, Outcome, Validity, ValueOf, clicked, pressed, response_of};

use crate::runtime::env::Env;
use crate::runtime::memory::sub;
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, Flags, StyleCx, StyleKey};
pub use craft_behavior::Input;
use craft_behavior::{CellSpace, ItemLayout, ItemSource, Proposal, ScrollPos, SelectionModel, Sense};
use egui::{Id, IdSalt, Pos2, Rect, Ui, Vec2};
use run::ShowAt;
use std::hash::Hash;
use std::ops::Deref;

pub use crate::runtime::drag::{DragPreview, DropMark, DropTarget, DropZone, OsFile, OsFiles, WindowDropTarget};
pub(crate) use body::MOVE_BAND;
pub use craft_behavior::ColumnKey;
pub use craft_behavior::DropOperation;
pub use craft_behavior::MiddleButton;
pub use craft_behavior::ReorderKeys;
pub(crate) use run::measure_under;
#[cfg(feature = "testing")]
pub(crate) use steps::queue_reveal;
pub use steps::scroll_step;
pub(crate) use steps::{child_reveal, peek_reveal, queue_child_reveal, take_reveal};
pub(crate) use steps::{draft, keeping_ids};

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
    /// Takes the caller's `Props::sizing`: a leaf keeps it in its `Common`; the derive writes a container's or a
    /// collection's into its `ContainerProps`, over the default its constructor set.
    #[doc(hidden)]
    fn set_sizing(&mut self, sizing: Sizing, _: Internal) {
        self.common_mut(Internal::new()).sizing = Some(sizing);
    }
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

/// How a component that is not enabled looks (`Component::DISABLED`). Either way it takes no input, its access node
/// is disabled and the children it shows inherit the disabled state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DisabledLook {
    /// The style's Disabled state: `text.disabled` ink, the theme's disabled fills, and the data colours its `Draw`
    /// and its content painter's data paths paint (a tone dot, a swatch icon, a gradient ramp, document text) at
    /// `Opacity::DISABLED`, each keeping its hue. Controls whose look is chrome (buttons, fields, sliders, a gradient
    /// bar's stops) take it.
    States,
    /// The component paints its enabled look, and step 13 lays one veil over it: a rect of the surface it sits on at
    /// `1 - Opacity::DISABLED`, over its surface, its paint, its faces, the children it shows from `paint` and
    /// `paint_over`, not its focus ring. Each colour under the veil shows mixed toward that surface by the disabled
    /// share, and its parts do not show through each other. A component that shows data (a picture, a colour, a plot)
    /// takes it, so the data dims with the rest. The children of its body are not under the veil; each shows its own
    /// disabled look.
    Faded,
}

/// What Escape does to a value component's running gesture (step 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum EscapeEdit {
    /// Escape ends the gesture with `Cancel { Escape }`: the value goes back to where the gesture began.
    #[default]
    Cancel,
    /// Escape ends the gesture with `Commit`: the value keeps what the gesture picked. A transient picker that the
    /// host drives (a HUD over the canvas) closes on Escape and keeps the pick.
    Commit,
}

/// The core trait: every UI type is a `Component`.
pub trait Component: Meta + Sized {
    /// The AccessKit role.
    const ROLE: Role;
    /// Roles `access` may pick per instance.
    const ALT_ROLES: &'static [Role] = &[];
    /// How input reaches the component.
    const INPUT: Input = Input::Pointer;
    /// What Escape does to the running gesture.
    const ESCAPE: EscapeEdit = EscapeEdit::Cancel;
    /// Whether the component hosts popups: `false` skips the overlay bookkeeping of steps 9, 10, 12, 15 and 16.
    const OVERLAYS: bool = true;
    /// Whether the component declares parts, a text part or an editor part: `false` skips the parts step (the parts
    /// hook, the measure view it reads and the memory of the last state) and leaves the sense step with no part
    /// regions. A shape that declares parts of its own (a body's scroll parts, an item layout) still runs its hook.
    const PARTS: bool = true;
    /// Whether the component or its parts can take key focus: `false` skips the stop registration of step 7, except
    /// for the roving group of a body the shape declares.
    const STOPS: bool = true;
    /// The focus scope a leaf runs in, as `ContainerProps::focus` gives a container's: the leaf's own stop and its
    /// parts lie inside it, and `FocusPolicy::NONE` (the default) opens none. A scope that restores
    /// (`FocusPolicy::TAB_LAYER`) makes the stop focus came from its opener, so focus goes back there when the leaf
    /// goes away with focus inside (a canvas text editor hands it back to the canvas). Containers and collections take
    /// their scope from their props, not from this.
    const FOCUS_SCOPE: crate::runtime::focus::FocusPolicy = crate::runtime::focus::FocusPolicy::NONE;
    /// The focus scope this instance runs in. The default is `FOCUS_SCOPE`; a component whose scope is a choice of its
    /// caller (an editor that hands focus back to where it came from, or not) overrides it.
    fn focus_scope(&self) -> crate::runtime::focus::FocusPolicy {
        Self::FOCUS_SCOPE
    }
    /// Keys the focused component keeps from egui and the shortcut dispatcher.
    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(Self::INPUT);
    /// The keys this instance keeps in its current state. The default is `KEYS`; a component whose key use depends
    /// on its state (a field that records chords while recording) overrides it, so one component serves every state.
    fn keys(&self) -> crate::runtime::focus::KeyUse {
        Self::KEYS
    }
    /// Whether the component paints its label as visible text. `false` for a face that shows only an icon or a
    /// glyph: with no tooltip set, its label is then its tooltip, since the label is the only visible name the
    /// pointer can reach. A component whose answer varies per instance overrides `shows_label`. A canvas control's
    /// label is never its tooltip, whatever this says: its rect is the canvas, and its parts name themselves.
    const SHOWS_LABEL: bool = true;
    /// Whether the control draws only its mark and its text is the label step's, placed after it: a check box, a
    /// radio button or a switch. `Show::show` then runs the label step for an instance that carries a label and sits
    /// in no label scope. The label of a leaf label step (its own, `labelled`) is part of the control's hit target:
    /// mark, gap and label hover, press and activate as one.
    const LABELLED_AFTER: bool = false;
    /// Whether the component edits one value that several selected things cannot disagree on (a colour picker's
    /// area, strip or ramp: it edits the colour it is given). Such a type has no mixed state and no mixed story.
    /// Two sources: a canvas control (`#[component(canvas_control)]`) is single-value by kind, since it edits one
    /// layer's geometry; any other component declares it here.
    const SINGLE_VALUE: bool = false;
    /// Whether the binding is the component's view of what it shows (a time area's scroll and zoom, a navigator's
    /// canvas view), not a value the user edits. The wheel moves a view without focus, as it scrolls a scroll
    /// region; a bound value control takes it only while focused or armed.
    const BINDS_VIEW: bool = false;
    /// How a disabled instance looks: token states (the default), or its enabled look faded.
    const DISABLED: DisabledLook = DisabledLook::States;
    /// Whether the component is a rule between groups of members (a divider). In a bar's line the bar draws it, as a
    /// divider line along the axis the rule fills, only when the member after it shows: a rule before a member the bar
    /// hides, or with no member after it, is not drawn.
    const RULE: bool = false;

    /// The style type.
    type Style: ComponentStyle;
    /// `Response`, `EditResponse<T>` or a richer type.
    type Output: Outcome;

    /// Whether this instance paints its label as visible text; `SHOWS_LABEL` unless an instance prop decides, as
    /// `icon_only` does. It reads props only, never the paint record.
    fn shows_label(&self) -> bool {
        Self::SHOWS_LABEL
    }

    /// The muted line a `LABELLED_AFTER` control's label shows after its text (`SideLabel::detail`), as a radio's
    /// `detail`; none by default. It reads props only.
    fn label_detail(&self) -> Option<crate::runtime::env::Label> {
        None
    }

    /// How far the component paints outside its rect by design, in points (a graph node's ports sit on its edge).
    /// Steps 13 and 14 clip to the rect grown by this much; the paint-bounds suite case reads it.
    fn paint_outset(_tokens: &crate::style::UiTokens) -> f32 {
        0.0
    }

    /// The variant an instance that asks for none takes in `env`; `None` takes the style's first. A variant the caller
    /// asks for always stands. Read only `#[env(style)]` values, which the style cache keys on.
    fn default_variant(_env: &crate::runtime::env::Env) -> Option<craft_theme::Variant> {
        None
    }

    /// Pure function of the resolved context and the key.
    fn style(cx: &StyleCx<'_>, key: Self::StyleKey) -> Self::Style;

    /// The insets from `rect` to the content rect where the component is shown: the style's padding. A component
    /// whose insets follow its rect computes them here (a sheet's row headers hold the digits of the last row its view
    /// shows). Measuring adds the style's padding.
    fn padding(&self, _rect: Rect, style: &Self::Style) -> craft_behavior::Insets {
        style.metrics().padding
    }

    /// Content size only; the pipeline adds padding and minimum size.
    fn measure(&mut self, _cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// The axis along which the component takes all the room the flow offers when that is more than it measured,
    /// as a container with `Sizing::Fill` does: the width when its caller set `Sizing::FillWidth` (a search field
    /// before a count) or `Sizing::Fill` (which takes the height as well). A component that fills by what it is
    /// overrides it: a vertical rule fills its row's height.
    fn fills(&self) -> Option<craft_behavior::Axis> {
        matches!(self.common(Internal::new()).sizing, Some(Sizing::FillWidth | Sizing::Fill)).then_some(craft_behavior::Axis::X)
    }

    /// Whether the content is made to be cut where it does not fit: an elided line or a path. In a row too short for
    /// its children such a component narrows to its least before any other gives way, so labels beside it stay whole.
    fn cut_first(&self) -> bool {
        false
    }

    /// The root's sense; the shape's default unless a canvas control.
    fn sense(&self) -> Sense {
        if Self::CANVAS_CONTROL { Sense::empty() } else { <Self::Shape as ShowShape>::DEFAULT_SENSE }
    }

    /// The distance one wheel line scrolls the component when it senses the wheel itself: the service converts the
    /// device's lines to points with it. A scroll region and a collection state theirs in their props.
    fn wheel_notch(&self) -> craft_behavior::WheelNotch {
        craft_behavior::WheelNotch::DEFAULT
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

    /// Lets a drag on the scrub's sources (the side label, the body, a part) change the value; the pipeline runs the
    /// gesture (16.3, B2).
    fn scrub(&self) -> Option<Scrub<ValueOf<Self>>> {
        None
    }

    /// The scroll offset's owner when the app drives scrolling; `None` keeps it in session memory. The app restores a
    /// scroll position, or keeps the content under the pointer fixed across a zoom, by writing it.
    /// A bound offset stays the app's plain offset: it does not follow content whose extents change before the view.
    fn scroll(&mut self) -> Option<BindMut<'_, ScrollPos>> {
        None
    }

    /// App values for content the component does not build: its body or its items. Runs at step 13, and in a measuring run of the body before it.
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

    /// Words `value` as the control shows it ("40.0", "20 to 60"); `None` for a component that words no value. The
    /// reset dot's tooltip names the default with it, and `value_text` of a slider or dial calls it with the shown
    /// value.
    fn word_value(&self, _value: &ValueOf<Self>, _cx: &Cx<'_, Self>) -> Option<String> {
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

    /// The room the container's own parts take at the edges of its header's line (a menu button, a drag band). The
    /// header's children take the rest; `parts` declares the parts in `PartsCx::header_slot` with
    /// `PartsCx::header_part`.
    fn header_parts(&self, _cx: &MeasureCx<'_, Self>) -> HeaderParts {
        HeaderParts::NONE
    }

    /// Fills the header's trailing region.
    fn controls(&mut self, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

    /// Fills the footer region.
    fn footer(&mut self, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

    /// Fills the region under the body and above the footer, whatever the footer's place.
    fn body_end(&mut self, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

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

    /// Whether the items are members of the collection's run instead of runs of their own. Each visible item is then
    /// sensed with its parts as a member of the collection's arena, painted at its row with the row's flags, and
    /// reported as an access sub-node of the collection, its parts' nodes under it; its `tooltip`, or its hovered part's,
    /// shows as the collection's while the pointer rests on it. The item's `style`, `measure`, `sense`, `flags`,
    /// `parts`, `update`, `paint`, `paint_over`, `access` and `finish` run, so its output gives the item event, open
    /// request and label rect as a run's does. A member has no stop and no focusable part, shows no popup, edits no
    /// value and takes no rich tooltip.
    const MEMBERS: bool = false;

    /// Plain data: selection mode, reorder, rename, columns.
    fn items(&self) -> &CollectionProps;

    /// Whether keyboard focus shows a ring on the lead row, or on a populated rows viewport with no visible lead.
    /// Lists and tables use this with independent item runs and an untransformed viewport.
    fn shows_row_focus(&self) -> bool {
        false
    }

    /// The selection binding; `None` keeps it in session memory.
    fn selection(&mut self) -> Option<BindMut<'_, Self::Selection>> {
        None
    }

    /// The column arrangement binding; `None` keeps it in the header's own memory. The header starts each pass from
    /// the bound layout and writes each change into it.
    fn column_layout(&mut self) -> Option<BindMut<'_, ColumnLayout>> {
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
    /// the layout anew. `item` is the largest ideal size of the items when the collection `measures_items`.
    fn item_layout_from(
        &self,
        _previous: Option<Self::ItemLayout>,
        source: &dyn ItemSource<Self::Key>,
        width: f32,
        _item: Option<Vec2>,
        cx: &StyleCx<'_>,
    ) -> Self::ItemLayout {
        self.item_layout(source, width, cx)
    }

    /// Whether the layout sizes its items from their own measure: the items step measures every item at its ideal
    /// size and hands the largest to `item_layout_from` and `ideal_width`. The measure is kept while the items' inputs
    /// hold (the rows stamp or the source's revision, the item count, the room, the env and `dress_inputs`).
    fn measures_items(&self) -> bool {
        false
    }

    /// What `dress` reads of the collection, hashed: a kept measure of the items serves only while it is the same.
    fn dress_inputs(&self) -> u64 {
        0
    }

    /// The width the collection asks for when it takes its width from its content, over its widest row: a grid of
    /// cards asks for its declared number of columns. 0 asks for nothing. `item` is as `item_layout_from` gets it;
    /// `rows` is the number of rows the source holds, for a collection whose rows stand side by side.
    fn ideal_width(&self, _item: Option<Vec2>, _rows: usize, _cx: &StyleCx<'_>) -> f32 {
        0.0
    }

    /// A view of `source` for this pass, such as the rows a filter keeps; `None` shows `source` as it is. `kept` is the
    /// item whose rename runs: the view keeps it whether or not it matches, so it is named in place.
    fn filtered<'s>(
        &self,
        _source: &'s dyn ItemSource<Self::Key>,
        _env: &crate::runtime::env::Env,
        _kept: Option<&Self::Key>,
    ) -> Option<Box<dyn ItemSource<Self::Key> + 's>> {
        None
    }

    /// Whether the focus ring goes around the lead item while the collection has keyboard-visible focus: the item step
    /// gives the ring the lead's rect (`Cx::focus_shape`), or the collection's own rect while it has no lead or its
    /// lead is not shown. Off by default.
    fn rings_lead(&self) -> bool {
        false
    }

    /// Adjusts the item the closure built for a row, from the row's state. An auto row's measure reads the item this
    /// makes, under the row-hook contract on `ItemCx`.
    fn dress<'r>(&self, item: Self::Item<'r>, _row: &RowState) -> Self::Item<'r> {
        item
    }

    /// The rename editor at `label`: the label rect the renaming item reports (`ItemOutput::label_rect`), else its row.
    /// `name` is the item's name before the rename; the editor is named after it.
    fn rename_editor(&mut self, _ui: &mut Ui, _draft: Bind<'_, String>, _label: Rect, _name: &str) -> Option<EditResponse<String>> {
        None
    }

    /// Whether the collection hosts a component in the header cell of `column`: the header then calls `header_cell`
    /// for it and paints no label.
    fn hosts_header(&self, _column: &ColumnKey) -> bool {
        false
    }

    /// Hosts a component in the header cell of `column`, in `rect`, measured and shown under `key` (the column's index
    /// among the declared columns, so a move or a hide keeps it). Called only for a column `hosts_header` answers
    /// `true` for.
    fn header_cell(&mut self, _ui: &mut PaintUi<'_>, _column: &ColumnKey, _key: HostedKey, _rect: Rect) {}

    /// The text the rename field of the item `key` opens with; `None` for the item's own text.
    fn rename_draft(&self, _key: &Self::Key) -> Option<String> {
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

    /// Whether the collection's own dragged items may land before `insert` (a position of the display order, `over` is
    /// the item the pointer is on, none over an empty collection). A refused spot shows no drop mark.
    fn accepts(&self, _drag: &ListDrag<Self::Key>, _over: Option<&Self::Key>, _insert: usize) -> bool {
        true
    }

    /// What the collection takes from drags that did not start in it, in order: the first kind that takes the drag's
    /// payload at the place under the pointer gets the drop, reported as `CollectionEvent::Dropped`. None by default.
    fn drops(&self) -> &[DropKind<'_, Self::Key>] {
        &[]
    }

    /// The press on the line between two adjacent rows the collection takes, reported as
    /// `CollectionEvent::EdgePressed`; none by default, and without one the rows take every press as they do.
    fn edge_press(&self) -> Option<&EdgePress<'_, Self::Key>> {
        None
    }

    /// Whether the collection takes the zoom share of the wheel (Ctrl or Cmd with the wheel): the items step reports
    /// it as `CollectionEvent::Zoom`. A collection that leaves the zoom to the app reports the event and the app sets
    /// its zoom and, through `Component::scroll`, the scroll position. A collection that applies the zoom itself takes
    /// the event and names the point to keep in place through [`Collection::zoom_anchor`], so the step keeps the row
    /// under it where it is.
    fn zooms(&self) -> bool {
        false
    }

    /// The point whose row stays in place across a zoom step this collection takes in the current pass, in screen
    /// points. `wheel` is the point of the `CollectionEvent::Zoom` the items step reported in this pass, `None` when
    /// it reported none; a collection that also takes zoom keys answers the pointer it read with them. Read by the
    /// items step in `content` of the same pass, against the layout the zoom changes from. `None` by default: the
    /// step keeps its usual anchor.
    fn zoom_anchor(&self, _wheel: Option<Pos2>) -> Option<Pos2> {
        None
    }

    /// The height of the empty state at `width`: a `Sizing::Content` collection with no rows is this tall. The
    /// default is the height of `CollectionProps::empty` laid out at `width`; an override replaces the empty text, so
    /// it replaces `empty` as well.
    fn empty_height(&self, width: f32, ui: &Ui, cx: &StyleCx<'_>) -> f32 {
        self.items().empty.as_ref().map_or(0.0, |text| items::empty_text::height(ui, cx.tokens, text, width))
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
        let place = Env::of(ui);
        run::show_leaf(&mut Some(self), ui, (ShowAt::Rect(rect), place), None)
    }

    /// Shows the component with rich tooltip content from the same call.
    fn show_with_tip(self, ui: &mut Ui, tip: impl FnOnce(&mut Ui)) -> Self::Output {
        let place = Env::of(ui);
        run::show_leaf(&mut Some(self), ui, (ShowAt::Flow, place), Some(Box::new(tip)))
    }

    /// Shows the component at `rect` with rich tooltip content.
    fn show_at_with_tip(self, ui: &mut Ui, rect: Rect, tip: impl FnOnce(&mut Ui)) -> Self::Output {
        let place = Env::of(ui);
        run::show_leaf(&mut Some(self), ui, (ShowAt::Rect(rect), place), Some(Box::new(tip)))
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
    fn show<R>(self, ui: &mut Ui, mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Inner<Self::Output, R> {
        let place = Env::of(ui);
        run::show_container(&mut Some(self), ui, (ShowAt::Flow, place), &mut body, None)
    }

    /// Shows the container at `rect`, with no allocation: a frame over a split or a layer the parent places.
    fn show_at<R>(self, ui: &mut Ui, rect: Rect, mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Inner<Self::Output, R> {
        let place = Env::of(ui);
        run::show_container(&mut Some(self), ui, (ShowAt::Rect(rect), place), &mut body, None)
    }

    /// Shows the container at `rect` and takes that rect's room in the flow of `ui`: what is shown after it lies
    /// past it, as after [`show`](Self::show).
    fn show_at_taking_room<R>(self, ui: &mut Ui, rect: Rect, mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Inner<Self::Output, R> {
        run::show_container_taking(&mut Some(self), ui, rect, &mut body)
    }

    /// Measures the container with `body` without allocating: steps 1 to 5 under the id `show` would use next, with
    /// the body, header, footer and end regions run once in an invisible sizing pass, laid out as `show` lays them
    /// out. The size comes from this pass's content alone; along an axis the body scrolls on it is at most the room.
    fn measure_in<R>(&mut self, ui: &Ui, proposal: Proposal, mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Vec2 {
        run::measure_container(self, ui, proposal, &mut body)
    }

    /// Shows the container with rich tooltip content.
    fn show_with_tip<R>(self, ui: &mut Ui, mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R, tip: impl FnOnce(&mut Ui)) -> Inner<Self::Output, R> {
        let place = Env::of(ui);
        run::show_container(&mut Some(self), ui, (ShowAt::Flow, place), &mut body, Some(Box::new(tip)))
    }

    /// Shows the container at `rect` with rich tooltip content.
    fn show_at_with_tip<R>(
        self,
        ui: &mut Ui,
        rect: Rect,
        mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R,
        tip: impl FnOnce(&mut Ui),
    ) -> Inner<Self::Output, R> {
        let place = Env::of(ui);
        run::show_container(&mut Some(self), ui, (ShowAt::Rect(rect), place), &mut body, Some(Box::new(tip)))
    }
}

/// Shows a collection's items.
pub trait ShowCollection: Collection + sealed::ShowCollection {
    /// Shows the visible items of `source`, one `item` call each.
    fn show_items<'r>(self, ui: &mut Ui, source: &dyn ItemSource<Self::Key>, item: &mut ItemFn<'r, Self>) -> Self::Output {
        run::show_items(&mut Some(self), ui, source, item)
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
