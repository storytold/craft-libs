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
mod group;

mod floating;
mod panels;
mod root;
mod spec;
mod splitter;
pub mod stack;
#[cfg(feature = "testing")]
mod stories;
pub mod targets;
pub mod tree;
mod window_menu;
mod words;

mod commands;
#[doc(hidden)]
pub mod cross_window;
pub mod native;

pub use config::{DockConfig, DockSides};
#[doc(hidden)]
pub use drop::{Arrangement, end_replaced_drags};
pub use drop::{DockDrag, DockDropTarget, DockDropped, DragUnit, float_drop, resolve_drop, start_group_drag};
pub use event::{DockEvent, FloatSpace, HidePanels};
pub use group::{DockGroup, DockGroupOutput, DockGroupStyle, GroupGrip, GroupGripOutput, GroupGripStyle, HeaderHook};

pub use floating::{FloatingGroup, FloatingGroupOutput, FloatingGroupStyle};
pub use panels::DockPanels;
pub use root::{DockRoot, DockRootOutput, DockRootStyle};
pub use spec::{DockHeights, PanelSpec};
pub use splitter::{Splitter, SplitterStyle};
pub use targets::{DockSizes, DockSpot, DockTargets, GroupDrag, ZoneRule};
pub use words::DockWords;

pub use commands::{
    ArrangeCommands, ArrangeSpace, Arranged, DockCommands, DockScene, DockTo, WorkspaceAsk, WorkspaceCommands, actions, arrange, arrange_states,
    dock_states, move_actions, run_workspace, workspace_states,
};

mod dock_column;
mod flyout;
mod iconic;

pub use dock_column::{DockColumn, DockColumnOutput, DockColumnStyle, GroupSize, GroupView, fit_widths, group_heights, stacked, width_bounds};
pub use flyout::{Flyout, GroupAt};
pub use iconic::{IconicColumn, IconicColumnOutput, IconicColumnStyle, IconicSizes};
pub use window_menu::{WindowMenuRows, WindowMenuRowsOutput};

use crate::contract::{
    Access, Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, Edit as _, EditResponse, Focus as _, Outcome, Pad, PaintUi,
    PartsCx, Props as _, Role, Sizing, WordSetCx as _,
};
use crate::runtime::env::Label;
use crate::runtime::focus::KeyUse;
use crate::runtime::memory::sub;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::dock::{DockEdge, Layout, Node, SplitSize};
use craft_behavior::{Axis, EditEvents, Flow, Metrics, PartKey, Sense};
use divider::{Divider, Drag};
use egui::{Rect, Response};
use std::fmt::Debug;
use std::hash::{DefaultHasher, Hash, Hasher};
use tree::{FrameRules, SplitHandle};

const HANDLE: PartKey = PartKey::new("handle");

/// The stable key of a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

/// A panel as its group's strip shows it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PanelTitle {
    /// The panel.
    pub key: PanelKey,
    /// The tab's text.
    pub title: Label,
    /// Whether the panel's tab shows a close button, where the group's tabs close.
    pub closable: bool,
}

impl PanelTitle {
    /// A panel `key` titled `title`, not closable from its tab.
    pub fn new(key: PanelKey, title: impl Into<Label>) -> Self {
        PanelTitle { key, title: title.into(), closable: false }
    }

    /// Whether the panel's tab shows a close button.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }
}

/// What a dragged tab carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PanelDrag {
    /// The dragged panel.
    pub panel: PanelKey,
    /// The group the tab was in, named by its anchor panel.
    pub from: PanelKey,
    /// The arrangement the drag started on, when it started in a dock area.
    pub(crate) started: Option<drop::Arrangement>,
}

/// The style of a split area: the docking sizes its frames, handles and keys read.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SplitAreaStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The docking sizes: the dividers' gap, reach and key steps, and the groups' least extent and strip.
    sizes: DockSizes,
}

/// What a split area reports: the layout's edits.
#[derive(Debug)]
#[non_exhaustive]
pub struct SplitAreaOutput<P> {
    /// The resize edits of the layout and the area's response.
    pub edit: EditResponse<Layout<P>>,
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
/// frame (`tree::frames` with the same rules).
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, probe(SplitArea<'static, usize>))]
pub struct SplitArea<'a, P: Clone + Eq + Hash + Debug + Send + Sync + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    layout: Bind<'a, Layout<P>>,
    rules: FrameRules<'a, P>,
    /// The accessibility name of the area's dividers, for a split whose dividers are not "Resize panels": "Resize
    /// preview", "Resize timeline".
    #[prop(into)]
    divider_label: Option<Label>,
}

impl<'a, P: Clone + Eq + Hash + Debug + Send + Sync + 'static> SplitArea<'a, P> {
    /// A split area named `label` over `layout`, sized by `rules`. A mixed layout shows no handles.
    pub fn new(label: impl Into<Label>, layout: Bind<'a, Layout<P>>, rules: FrameRules<'a, P>) -> Self {
        let props = ContainerProps::new().arena(true).sizing(Sizing::Fill).pad(Pad::None);
        SplitArea { common: Common::new(), props, layout, rules, divider_label: None }.label(label)
    }

    /// A two-body area: `bodies.0` first along `axis` and `bodies.1` after it, with one divider between them. The first
    /// body takes `share` of the room left after the divider; the area's response reports the divider's drags and keys
    /// as [`SplitAreaOutput::share`], and the host hands the share back in the next pass. The body reads each body's
    /// rect with [`SplitFramesCx::group_frames`].
    pub fn pair(label: impl Into<Label>, axis: Axis, bodies: (P, P), share: f32, rules: FrameRules<'a, P>) -> Self {
        let root = Node::split(axis, SplitSize::Ratio(share), Node::tabs(vec![bodies.0], 0), Node::tabs(vec![bodies.1], 0));
        Self::new(label, Bind::given(Layout::new(Some(root), Vec::new())), rules)
    }
}

/// The groups of the layout with the frames the area gave them this pass, for the body.
#[derive(Clone)]
struct PassFrames<P>(Vec<(P, Rect)>);

impl<P> Default for PassFrames<P> {
    fn default() -> Self {
        PassFrames(Vec::new())
    }
}

/// The docks of the layout with the frames the area gave them this pass, for the body.
#[derive(Clone, Default)]
struct PassDocks(Vec<(DockEdge, Rect)>);

const FRAMES: &str = "split_frames";
const DOCKS: &str = "split_docks";

/// What a split area's body reads of the area's frames, on its `BodyCx`.
pub trait SplitFramesCx {
    /// The groups of the split area this body belongs to, each as its anchor panel and its rect, with this pass's
    /// resize edits applied: a panel placed at its group's rect sits against the handle in the same pass.
    fn group_frames<P: Clone + Send + Sync + 'static>(&self) -> Vec<(P, Rect)>;

    /// The docks of the split area this body belongs to, each as its window edge and its rect, with this pass's
    /// resize edits applied.
    fn dock_frames(&self) -> Vec<(DockEdge, Rect)>;
}

impl SplitFramesCx for BodyCx<'_> {
    fn group_frames<P: Clone + Send + Sync + 'static>(&self) -> Vec<(P, Rect)> {
        self.memory.recall::<PassFrames<P>>(sub(self.id, FRAMES)).0
    }

    fn dock_frames(&self) -> Vec<(DockEdge, Rect)> {
        self.memory.recall::<PassDocks>(sub(self.id, DOCKS)).0
    }
}

/// The dividers of the layout's docked tree in `rect`, each with the handle that names its split.
fn handles<P>(layout: &Layout<P>, rect: Rect, sizes: &DockSizes, rules: &FrameRules<'_, P>, rtl: bool) -> Vec<SplitHandle> {
    layout.root.as_ref().map_or_else(Vec::new, |root| tree::frames(root, rect, sizes, rules, rtl).handles)
}

/// The divider of `handle` in reading order. With `ranged`, the range it can travel is the tree's; without, it is
/// where it stands, which is all its part and face need.
fn divider_of<P>(
    layout: &Layout<P>,
    handle: &SplitHandle,
    rect: Rect,
    sizes: &DockSizes,
    rules: &FrameRules<'_, P>,
    rtl: bool,
    ranged: bool,
) -> Divider {
    let flow = Flow::new(rect, rtl);
    let (centre, split) = (flow.to_physical(handle.divider).center(), flow.to_physical(handle.split));
    let at = if handle.axis == Axis::X { centre.x } else { centre.y };
    let range = match layout.root.as_ref() {
        Some(root) if ranged => tree::drag_range(root, rect, sizes, rules, &handle.path),
        _ => (0.0, 0.0),
    };
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
        SplitAreaStyle { metrics: Metrics::new(), sizes: DockSizes::style(cx) }
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
        for (at, handle) in handles(layout, rect, &sizes, &self.rules, rtl).iter().enumerate() {
            let divider = divider_of(layout, handle, rect, &sizes, &self.rules, rtl, false);
            divider::declare(cx, rect, HANDLE, at as u64, &divider);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(layout) = cx.value().cloned() else {
            cx.core.memory.shown::<PassFrames<P>>(sub(sub(cx.core.id, "body"), FRAMES)).0.clear();
            return;
        };
        let (rect, rtl, sizes, rules) = (cx.rect, cx.flow.is_rtl(), cx.style.sizes, self.rules);
        let handles = handles(&layout, rect, &sizes, &rules, rtl);
        for (at, handle) in handles.iter().enumerate() {
            let divider = divider_of(&layout, handle, rect, &sizes, &rules, rtl, false);
            match divider::pointer(cx, HANDLE, at as u64, &divider) {
                Some(Drag::Start) => cx.begin(),
                Some(Drag::To(coord)) => {
                    if let Some(next) = moved(&layout, rect, &sizes, &rules, handle, coord - divider.at) {
                        cx.propose(next);
                    }
                }
                Some(Drag::Release) => cx.commit(),
                None => {}
            }
        }
        let focused = cx.active_part().filter(|part| part.key == HANDLE).and_then(|part| part.index);
        if let Some(index) = focused
            && let Some(handle) = handles.get(index as usize)
        {
            let divider = divider_of(&layout, handle, rect, &sizes, &rules, rtl, true);
            if let Some((_, coord)) = divider::keys(cx, HANDLE, &[(index, divider)], sizes.step, sizes.jump)
                && let Some(next) = moved(&layout, rect, &sizes, &rules, handle, coord - divider.at)
            {
                cx.set(next);
            }
        }
        let frames = cx.value().and_then(|shown| shown.root.as_ref()).map(|root| tree::frames(root, rect, &sizes, &rules, rtl));
        let groups = frames.iter().flat_map(|frames| frames.groups.iter()).filter_map(|group| Some((group.anchor()?.clone(), group.rect))).collect();
        let docks = frames
            .iter()
            .flat_map(|frames| frames.groups.iter())
            .filter_map(|group| match group.node {
                Node::Dock { side, .. } => Some((*side, group.rect)),
                _ => None,
            })
            .collect();
        cx.core.memory.shown::<PassDocks>(sub(sub(cx.core.id, "body"), DOCKS)).0 = docks;
        cx.core.memory.shown::<PassFrames<P>>(sub(sub(cx.core.id, "body"), FRAMES)).0 = groups;
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(layout) = cx.value() else { return };
        let (sizes, rtl) = (cx.style.sizes, cx.flow.is_rtl());
        for (at, handle) in handles(layout, cx.rect, &sizes, &self.rules, rtl).iter().enumerate() {
            ui.face(HANDLE.at(at as u64), Splitter::new(handle.axis));
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(layout) = cx.value() else { return };
        let (rect, rtl, sizes) = (cx.rect, cx.flow.is_rtl(), cx.style.sizes);
        let words = cx.words::<DockWords>();
        let given = self.divider_label.as_ref().map(|label| cx.env.resolve(label));
        let name = given.as_ref().map_or_else(|| words.resize.as_str(), Label::as_str);
        for (at, handle) in handles(layout, rect, &sizes, &self.rules, rtl).iter().enumerate() {
            let divider = divider_of(layout, handle, rect, &sizes, &self.rules, rtl, true);
            divider::describe(access, HANDLE, at as u64, &divider, name, sizes.step);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> SplitAreaOutput<P> {
        SplitAreaOutput { edit: cx.edit_response() }
    }
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Container for SplitArea<'_, P> {}
