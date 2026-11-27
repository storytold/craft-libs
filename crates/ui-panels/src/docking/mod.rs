//! `SplitArea`, `Splitter` and `DockGroup`.
//!
//! `SplitArea` shows the docked tree of a `Layout` (craft-behavior's dock model): its body shows one `DockGroup`
//! per group at the rects `tree::frames` gives, and its handles resize the splits. A panel dragged from a group's
//! tab carries a `PanelDrag`; `SplitArea` is the drop target that says which group and side it lands on.

mod config;
pub mod divider;
mod drop;
mod event;
pub mod frame;
pub(crate) mod group;

pub(crate) mod floating;
mod panels;
mod placed;
pub(crate) mod root;
mod sources;
mod spec;
pub(crate) mod splitter;
pub mod stack;
#[cfg(feature = "suite")]
pub(crate) mod stories;
pub mod targets;
pub mod tree;
pub(crate) mod window_menu;
mod words;

mod commands;
#[doc(hidden)]
pub mod cross_window;
pub mod native;

pub use config::{DockConfig, DockSides};
#[doc(hidden)]
pub use drop::{Arrangement, end_replaced_drags};
pub use drop::{DockDrag, DockDropTarget, DockDropped, DragUnit, float_drop, resolve_drop, start_group_drag, start_icon_drag};
pub use event::{DockEvent, FloatSpace, HidePanels};
pub use group::{DockGroup, DockGroupOutput, DockGroupStyle, GroupGrip, GroupGripOutput, GroupGripStyle, HeaderHook, MenuRows};

pub use floating::{FloatingGroup, FloatingGroupOutput, FloatingGroupStyle, WindowColumn};
pub use panels::{DockPanels, PanelSpecs, SpecSource, request_actions};
#[doc(hidden)]
pub use panels::{close_requests, open_requests};
pub use placed::{Placing, placed_rect, with_area_rules};
pub use root::{DockRoot, DockRootOutput, DockRootStyle};
pub use sources::{DividerKind, DockSource, SourceKind};
pub use spec::{DockGrid, DockHeights, PanelDocument, PanelSpec};
pub use splitter::{Splitter, SplitterStyle};
pub use targets::{CardInset, ColumnCards, DockSizes, DockSpot, DockTargets, GroupDrag, ZoneRule};
pub use words::DockWords;

pub use commands::{
    ArrangeCommands, ArrangeSpace, Arranged, DockCommands, DockScene, DockTo, WorkspaceAsk, WorkspaceCommands, actions, arrange, arrange_states,
    dock_states, move_actions, run_workspace, workspace_states,
};

pub(crate) mod dock_column;
mod flyout;
pub(crate) mod iconic;

pub use dock_column::{
    ColumnArrow, ColumnBar, DockColumn, DockColumnOutput, DockColumnStyle, GroupSize, GroupView, column_cells, fit_widths, group_heights,
    group_rects, scale_widths, stacked, width_bounds,
};
pub use flyout::{Flyout, FlyoutOutput, FlyoutShown, FlyoutStyle, GroupAt};
pub use iconic::{FlyoutResize, IconGroupSeparator, IconicColumn, IconicColumnOutput, IconicColumnStyle, IconicMode, IconicSizes};
pub use window_menu::{WindowMenuRows, WindowMenuRowsOutput};

use crate::contract::{
    Access, Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, Edit as _, EditResponse, Focus as _, MemoKey, MemoProps as _,
    Outcome, Pad, PaintUi, PartsCx, Pointer as _, Props as _, Role, Sizing, WordSetCx as _,
};
use crate::runtime::commands::CommandRef;
use crate::runtime::derived::derived;
use crate::runtime::env::{AppValues, EnvValue, Label};
use crate::runtime::focus::KeyUse;
use crate::style::{ComponentStyle, FocusLook, StyleCx};
use craft_behavior::dock::{DockEdge, Layout, Node, SplitSize};
use craft_behavior::{Axis, EditEvents, Flow, Metrics, PartKey, Sense};
use divider::{Divider, Drag};
use egui::{Rect, Response, Vec2};
use std::fmt::Debug;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use tree::{FrameRules, SplitHandle};

const HANDLE: PartKey = PartKey::new("handle");

/// The stable key of a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, crate::contract::MemoProps, crate::contract::PlainData)]
#[non_exhaustive]
pub struct PanelKey(pub u64);

impl PanelKey {
    /// The panel keyed `key`.
    pub const fn new(key: u64) -> Self {
        PanelKey(key)
    }

    /// The key of the panel `panel`, from its hash. Equal panels have equal keys.
    pub fn of<P: Hash + ?Sized>(panel: &P) -> Self {
        let mut hasher = DefaultHasher::new();
        panel.hash(&mut hasher);
        PanelKey(hasher.finish())
    }
}

/// A key names a panel: it has no place, so a replay at another place keeps it.
impl crate::runtime::Shift for PanelKey {
    fn shifted(&self, _: &egui::RecordShift) -> Self {
        *self
    }

    fn snapped(&self, _: &Self, _: f32) -> Self {
        *self
    }
}

/// A panel as its group's strip shows it.
#[derive(Clone, Debug, PartialEq, crate::contract::MemoProps)]
#[non_exhaustive]
pub struct PanelTitle {
    /// The panel.
    pub key: PanelKey,
    /// The tab's text.
    pub title: Label,
    /// Whether the panel's tab shows a close button, where the group's tabs close.
    pub closable: bool,
    /// The panel's toggle command: the tab resolves it for its marker and its hint.
    pub command: Option<CommandRef>,
    /// The number the tab shows after the title ([`PanelSpec::count`]).
    pub count: Option<usize>,
    /// The line under the header row while the panel is shown ([`PanelSpec::caption`]).
    pub caption: Option<Label>,
    /// The data colour before the tab's text ([`PanelSpec::swatch`]).
    pub swatch: Option<Option<craft_theme::Srgba>>,
    /// The panel's documents, as tabs after the group's panel tabs ([`PanelSpec::documents`]).
    pub documents: Vec<PanelDocument>,
}

/// The tab titles of one group with their content key, taken once when they are made: a memoized view that holds them
/// feeds that key, not the titles. A host keeps a group's titles in a derived value so the key is taken only when they
/// are rebuilt.
#[derive(Clone, Debug)]
pub struct Titles {
    list: Arc<Vec<PanelTitle>>,
    key: u128,
}

impl Titles {
    /// The titles `list`, keyed by their content: this call walks them.
    pub fn from_content(list: Vec<PanelTitle>) -> Self {
        Titles { key: crate::contract::MemoKey::of(&list), list: Arc::new(list) }
    }

    /// The titles in tab order.
    pub fn as_slice(&self) -> &[PanelTitle] {
        &self.list
    }

    /// The panel and the document whose tab `key` is ([`PanelTitle::document_key`]); `None` for a panel's tab.
    pub fn document_of(&self, key: PanelKey) -> Option<(PanelKey, u64)> {
        self.list
            .iter()
            .find_map(|panel| panel.documents.iter().find(|document| panel.document_key(document.id) == key).map(|document| (panel.key, document.id)))
    }
}

/// Feeds the content key taken when the titles were made, not the titles.
impl Hash for Titles {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl PanelTitle {
    /// A panel `key` titled `title`, not closable from its tab.
    pub fn new(key: PanelKey, title: impl Into<Label>) -> Self {
        PanelTitle { key, title: title.into(), closable: false, command: None, count: None, caption: None, swatch: None, documents: Vec::new() }
    }

    /// The key of the tab of the panel's document `id`.
    pub fn document_key(&self, id: u64) -> PanelKey {
        PanelKey::of(&(self.key, "document", id))
    }

    /// The data colour before the tab's text; `Some(None)` is the theme's empty swatch.
    #[must_use]
    pub fn swatch(mut self, swatch: Option<Option<craft_theme::Srgba>>) -> Self {
        self.swatch = swatch;
        self
    }

    /// The panel's documents.
    #[must_use]
    pub fn documents(mut self, documents: Vec<PanelDocument>) -> Self {
        self.documents = documents;
        self
    }

    /// The number the tab shows after the title.
    #[must_use]
    pub fn count(mut self, count: Option<usize>) -> Self {
        self.count = count;
        self
    }

    /// The line under the header row while the panel is shown.
    #[must_use]
    pub fn caption(mut self, caption: Option<Label>) -> Self {
        self.caption = caption;
        self
    }

    /// The panel's toggle command.
    #[must_use]
    pub fn command(mut self, command: Option<CommandRef>) -> Self {
        self.command = command;
        self
    }

    /// Whether the panel's tab shows a close button.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }
}

/// What a dragged tab carries.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PanelDrag {
    /// The dragged panel.
    pub panel: PanelKey,
    /// The group the tab was in, named by its anchor panel.
    pub from: PanelKey,
    /// The size of that group where the drag started: a float of the panel takes it.
    pub source: Option<Vec2>,
    /// The arrangement the drag started on, when it started in a dock area.
    pub(crate) started: Option<drop::Arrangement>,
}

/// The style of a split area: the docking sizes its frames, handles and keys read.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SplitAreaStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The ring around the handle that has keyboard focus.
    #[style(focus)]
    focus: FocusLook,
    /// The docking sizes: the dividers' gap, reach and key steps, and the groups' least extent and strip.
    sizes: DockSizes,
}

/// What a split area reports: the layout's edits.
#[derive(Debug)]
#[non_exhaustive]
pub struct SplitAreaOutput<P> {
    /// The resize edits of the layout and the area's response.
    pub edit: EditResponse<Layout<P>>,
    /// The dividers drawn this pass: each split's path from the root (`false` for the first child), the rect of its
    /// handle and the id of the handle's accessibility node.
    pub dividers: Vec<(Vec<bool>, Rect, egui::Id)>,
}

/// The first body's share of a pair's room when its layout holds none: an even split.
const EVEN_SHARE: f32 = 0.5;

impl<P: Clone + PartialEq + Send + Sync + 'static> SplitAreaOutput<P> {
    /// The edits of a [`SplitArea::pair`]'s share: the first body's fraction of the room left after the divider, as the
    /// gesture the divider's drag or keys made.
    pub fn share(&self) -> EditEvents<f32> {
        let share = |layout: Layout<P>| match layout.root {
            Some(Node::Split { size: SplitSize::Ratio(share), .. }) => share,
            _ => EVEN_SHARE,
        };
        self.edit.events.clone().map(share)
    }
}

impl<P: Clone + PartialEq + Send + Sync + 'static> Outcome for SplitAreaOutput<P> {
    type Value = Layout<P>;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// The docked tree of a layout, with a handle between neighbouring groups. Its body shows the groups, each at its
/// frame (`tree::frames` with the same rules). The frames are built again only when the caller's revision of the layout
/// and rules, the room, the sizes or the direction change, and in a pass whose divider drag or keys edit the layout.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, probe(SplitArea<'static, usize>))]
pub struct SplitArea<'a, P: Clone + Eq + Hash + Debug + Send + Sync + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    layout: Bind<'a, Layout<P>>,
    rules: FrameRules<'a, P>,
    revision: u128,
    /// The accessibility name of the area's dividers, for a split whose dividers are not "Resize panels": "Resize
    /// preview", "Resize timeline".
    #[prop(into)]
    divider_label: Option<Label>,
    /// The paths of the splits whose handles `parts` declared this pass, by handle index.
    declared: Vec<Vec<bool>>,
    /// The frames of this run: the ones its revision keys, or those of the layout its own edit set.
    frames: Option<Arc<SplitFrames<P>>>,
}

impl<'a, P: Clone + Eq + Hash + Debug + Send + Sync + 'static> SplitArea<'a, P> {
    /// A split area named `label` over `layout`, sized by `rules`. `revision` is the caller's revision of the layout
    /// and of everything the rules read: it moves whenever either changes. A mixed layout shows no handles.
    pub fn new(label: impl Into<Label>, layout: Bind<'a, Layout<P>>, rules: FrameRules<'a, P>, revision: u128) -> Self {
        let props = ContainerProps::new().arena(true).sizing(Sizing::Fill).pad(Pad::None);
        SplitArea { common: Common::new(), props, layout, rules, revision, divider_label: None, declared: Vec::new(), frames: None }.label(label)
    }

    /// A two-body area: `bodies.0` first along `axis` and `bodies.1` after it, with one divider between them. The first
    /// body takes `share` of the room left after the divider; the area's response reports the divider's drags and keys
    /// as [`SplitAreaOutput::share`], and the host hands the share back in the next pass. The body reads each body's
    /// rect with [`SplitFramesCx::group_frames`]. `revision` is the caller's revision of everything the rules read.
    pub fn pair(label: impl Into<Label>, axis: Axis, bodies: (P, P), share: f32, rules: FrameRules<'a, P>, revision: u128) -> Self {
        let mut key = MemoKey::new();
        revision.key(&mut key);
        (axis == Axis::X).key(&mut key);
        share.key(&mut key);
        key.feed_hash(&bodies);
        let root = Node::split(axis, SplitSize::Ratio(share), Node::tabs(vec![bodies.0], 0), Node::tabs(vec![bodies.1], 0));
        Self::new(label, Bind::given(Layout::new(Some(root), Vec::new())), rules, key.finish())
    }

    /// The frames of this run (`SplitArea::frames`), else the ones derived for the caller's revision of `layout` in
    /// `rect`.
    fn frames_of(&self, ctx: &egui::Context, id: egui::Id, layout: &Layout<P>, rect: Rect, sizes: &DockSizes, rtl: bool) -> Arc<SplitFrames<P>> {
        if let Some(frames) = &self.frames {
            return Arc::clone(frames);
        }
        let key = FramesKey { revision: self.revision, rect, sizes: *sizes, rtl };
        derived(ctx, id.with("frames"), &key, || SplitFrames::of(layout, rect, sizes, &self.rules, rtl))
    }
}

/// What a split area's frames are built from: the caller's revision of the layout and rules, the room, the sizes and
/// the direction.
#[derive(crate::contract::MemoProps)]
struct FramesKey {
    revision: u128,
    rect: Rect,
    sizes: DockSizes,
    rtl: bool,
}

/// A split area's layout laid out in its rect: each group as its anchor panel and rect, each dock as its window edge
/// and rect, and each handle with the range its divider can travel.
struct SplitFrames<P: Send + Sync> {
    groups: SplitGroups<P>,
    docks: SplitDocks,
    handles: Vec<(SplitHandle, (f32, f32))>,
}

impl<P: Clone + Send + Sync> SplitFrames<P> {
    fn of(layout: &Layout<P>, rect: Rect, sizes: &DockSizes, rules: &FrameRules<'_, P>, rtl: bool) -> Self {
        let Some(root) = layout.root.as_ref() else { return Self::none() };
        let frames = tree::frames(root, rect, sizes, rules, rtl);
        let (anchors, rects): (Vec<P>, Vec<Rect>) = frames.groups.iter().filter_map(|group| Some((group.anchor()?.clone(), group.rect))).unzip();
        let (edges, dock_rects): (Vec<DockEdge>, Vec<Rect>) = frames
            .groups
            .iter()
            .filter_map(|group| match group.node {
                Node::Dock { side, .. } => Some((*side, group.rect)),
                _ => None,
            })
            .unzip();
        let handles = frames
            .handles
            .into_iter()
            .map(|handle| {
                let range = tree::drag_range(root, rect, sizes, rules, &handle.path);
                (handle, range)
            })
            .collect();
        SplitFrames {
            groups: SplitGroups { anchors: Arc::from(anchors), rects: Arc::from(rects) },
            docks: SplitDocks { edges: Arc::from(edges), rects: Arc::from(dock_rects) },
            handles,
        }
    }

    fn none() -> Self {
        SplitFrames { groups: SplitGroups::empty(), docks: SplitDocks::root(), handles: Vec::new() }
    }
}

/// The groups of the innermost split area, each anchor panel with its rect, as the area lays them out in the run its
/// body runs in: at the measuring run's rect, or at the real run's after its divider edits.
#[derive(Clone, PartialEq, crate::contract::MemoProps)]
struct SplitGroups<P: Send + Sync> {
    #[memo_props(hash = "a split area's anchors are plain panel values it keys by their hash")]
    anchors: Arc<[P]>,
    rects: Arc<[Rect]>,
}

impl<P: Send + Sync> SplitGroups<P> {
    fn empty() -> Self {
        SplitGroups { anchors: Arc::from(Vec::new()), rects: Arc::from(Vec::new()) }
    }
}

impl<P: Clone + PartialEq + Hash + Send + Sync + 'static> EnvValue for SplitGroups<P> {
    fn root() -> Self {
        Self::empty()
    }
}

/// The docks of the innermost split area, each window edge with its rect, laid out as [`SplitGroups`] are.
#[derive(Clone, PartialEq, crate::contract::MemoProps)]
struct SplitDocks {
    #[memo_props(hash = "a dock edge is a plain enum")]
    edges: Arc<[DockEdge]>,
    rects: Arc<[Rect]>,
}

impl EnvValue for SplitDocks {
    fn root() -> Self {
        SplitDocks { edges: Arc::from(Vec::new()), rects: Arc::from(Vec::new()) }
    }
}

/// What a split area's body reads of the area's frames, on its `BodyCx`.
pub trait SplitFramesCx {
    /// The groups of the split area this body belongs to, each as its anchor panel and its rect, with this pass's
    /// resize edits applied: a panel placed at its group's rect sits against the handle in the same pass.
    fn group_frames<P: Clone + PartialEq + Hash + Send + Sync + 'static>(&self) -> Vec<(P, Rect)>;

    /// The docks of the split area this body belongs to, each as its window edge and its rect, with this pass's
    /// resize edits applied.
    fn dock_frames(&self) -> Vec<(DockEdge, Rect)>;
}

impl SplitFramesCx for BodyCx<'_> {
    fn group_frames<P: Clone + PartialEq + Hash + Send + Sync + 'static>(&self) -> Vec<(P, Rect)> {
        let groups = self.env.get::<SplitGroups<P>>();
        groups.anchors.iter().cloned().zip(groups.rects.iter().copied()).collect()
    }

    fn dock_frames(&self) -> Vec<(DockEdge, Rect)> {
        let docks = self.env.get::<SplitDocks>();
        docks.edges.iter().copied().zip(docks.rects.iter().copied()).collect()
    }
}

/// The divider of `handle` in reading order, which can travel `range` from where it stands; `(0.0, 0.0)` where only
/// its place matters, as for its part and face.
fn divider_of(handle: &SplitHandle, range: (f32, f32), rect: Rect, sizes: &DockSizes, rtl: bool) -> Divider {
    let flow = Flow::new(rect, rtl);
    let (centre, split) = (flow.to_physical(handle.divider).center(), flow.to_physical(handle.split));
    let at = if handle.axis == Axis::X { centre.x } else { centre.y };
    Divider::new(handle.axis, at, split, (at + range.0, at + range.1), sizes.reach)
}

/// The layout with the boundary of `handle` moved by `delta` points in reading order; `None` when nothing moves.
fn moved<P: Clone>(
    layout: &Layout<P>,
    rect: Rect,
    sizes: &DockSizes,
    rules: &FrameRules<'_, P>,
    handle: &SplitHandle,
    delta: f32,
) -> Option<Layout<P>> {
    let root = layout.root.as_ref()?;
    let edits = tree::drag_sizes(root, rect, sizes, rules, &handle.path, delta);
    if edits.is_empty() {
        return None;
    }
    let mut next = layout.clone();
    tree::apply_sizes(next.root.as_mut()?, &edits);
    Some(next)
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Component for SplitArea<'_, P> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::SPLITTER;
    type Style = SplitAreaStyle;
    type Output = SplitAreaOutput<P>;

    fn style(cx: &StyleCx<'_>, _: SplitAreaStyleKey<P>) -> SplitAreaStyle {
        SplitAreaStyle { metrics: Metrics::new(), focus: cx.focus_look(), sizes: DockSizes::style(cx) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn bind(&mut self) -> Option<BindMut<'_, Layout<P>>> {
        Some(self.layout.reborrow())
    }

    /// The handles: edge parts above the groups, each its own Tab stop that the arrow keys move.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(layout) = cx.value() else { return };
        let (sizes, rtl) = (cx.style.sizes, cx.flow.is_rtl());
        let frames = self.frames_of(cx.ui.ctx(), cx.id, layout, rect, &sizes, rtl);
        for (at, (handle, _)) in frames.handles.iter().enumerate() {
            let divider = divider_of(handle, (0.0, 0.0), rect, &sizes, rtl);
            divider::declare(cx, rect, HANDLE, at as u64, &divider);
        }
        self.declared = frames.handles.iter().map(|(handle, _)| handle.path.clone()).collect();
        self.frames = Some(frames);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(layout) = cx.value().cloned() else { return };
        let (rect, rtl, sizes, rules) = (cx.rect, cx.flow.is_rtl(), cx.style.sizes, self.rules);
        let frames = self.frames_of(&cx.core.ctx, cx.core.id, &layout, rect, &sizes, rtl);
        let mut edited = false;
        for (at, (handle, _)) in frames.handles.iter().enumerate() {
            let divider = divider_of(handle, (0.0, 0.0), rect, &sizes, rtl);
            match divider::pointer(cx, HANDLE, at as u64, &divider) {
                Some(Drag::Start) => cx.begin(),
                Some(Drag::To(coord)) => {
                    if let Some(next) = moved(&layout, rect, &sizes, &rules, handle, coord - divider.at) {
                        cx.propose(next);
                        edited = true;
                    }
                }
                Some(Drag::Release) => cx.commit(),
                None => {}
            }
        }
        let focused = cx.active_part().filter(|part| part.key == HANDLE).and_then(|part| part.index);
        if let Some(index) = focused
            && let Some((handle, range)) = frames.handles.get(index as usize)
        {
            let divider = divider_of(handle, *range, rect, &sizes, rtl);
            if let Some((_, coord)) = divider::keys(cx, HANDLE, &[(index, divider)], sizes.step, sizes.jump)
                && let Some(next) = moved(&layout, rect, &sizes, &rules, handle, coord - divider.at)
            {
                cx.set(next);
                edited = true;
            }
        }
        // The layout this pass edited lays out from its own frames, in this pass.
        let frames = match cx.value() {
            Some(shown) if edited => Arc::new(SplitFrames::of(shown, rect, &sizes, &rules, rtl)),
            _ => frames,
        };
        self.frames = Some(frames);
    }

    /// The body lays the groups and docks out at the frames of this run: in a measuring run at its rect, in the real
    /// run after `update`'s divider edits.
    fn content_env(&self, cx: &Cx<'_, Self>, values: &mut AppValues) {
        let frames = match cx.value() {
            Some(layout) => self.frames_of(&cx.core.ctx, cx.core.id, layout, cx.rect, &cx.style.sizes, cx.flow.is_rtl()),
            None => Arc::new(SplitFrames::none()),
        };
        values.set(frames.groups.clone());
        values.set(frames.docks.clone());
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(layout) = cx.value() else { return };
        let (sizes, rtl) = (cx.style.sizes, cx.flow.is_rtl());
        let frames = self.frames_of(&cx.core.ctx, cx.core.id, layout, cx.rect, &sizes, rtl);
        for (at, (handle, _)) in frames.handles.iter().enumerate() {
            ui.face(HANDLE.at(at as u64), Splitter::new(handle.axis));
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(layout) = cx.value() else { return };
        let (rect, rtl, sizes) = (cx.rect, cx.flow.is_rtl(), cx.style.sizes);
        let words = cx.words::<DockWords>();
        let given = self.divider_label.as_ref().map(|label| cx.env.resolve(label));
        let name = given.as_ref().map_or_else(|| words.resize.as_str(), Label::as_str);
        let frames = self.frames_of(&cx.core.ctx, cx.core.id, layout, rect, &sizes, rtl);
        for (at, (handle, range)) in frames.handles.iter().enumerate() {
            let divider = divider_of(handle, *range, rect, &sizes, rtl);
            divider::describe(access, HANDLE, at as u64, &divider, name, sizes.step);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> SplitAreaOutput<P> {
        let dividers = self
            .declared
            .into_iter()
            .enumerate()
            .filter_map(|(at, path)| {
                let part = HANDLE.at(at as u64);
                let rect = cx.part_if_declared(part)?.rect;
                Some((path, rect, crate::runtime::access::sub_node_id(cx.core.id, part)))
            })
            .collect();
        SplitAreaOutput { edit: cx.edit_response(), dividers }
    }
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Container for SplitArea<'_, P> {}
