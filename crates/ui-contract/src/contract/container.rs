//! `ContainerProps` and the types of its fields (16.2).

use super::label::SideLabel;
use super::overlay::{Overlay, Resize};
use crate::layout::BodyLayout;
use crate::runtime::access::{Politeness, SetPosition};
use crate::runtime::commands::CommandScope;
use crate::runtime::env::{EnvValue, Label};
use crate::runtime::focus::FocusPolicy;
use crate::runtime::memory::SaveKey;
use crate::runtime::windows::WindowSpec;
use crate::style::SurfaceKind;
use craft_behavior::{CanvasTransform, Roving};
use craft_theme::ControlSize;
use egui::Rect;

pub use crate::runtime::env::ThemeScope;

/// Where a container lives.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub enum Mount {
    /// In the parent's layout.
    #[default]
    Inline,
    /// One of `AppShell`'s panes.
    Pane(ShellPane),
    /// On an egui layer through the overlay host.
    Layer(Overlay),
    /// Over the whole window.
    FullWindow,
    /// The whole content of the window the container is shown in: it fills the viewport's content rect, and when
    /// `resize` has grips they lie on the window's edges and corners. The body step moves and resizes the window
    /// through the OS only where the app draws the frame (the body step's `WindowParts` rule).
    #[non_exhaustive]
    Window {
        /// Whether grips on the window's edges and corners resize it.
        resize: Resize,
    },
    /// In an OS viewport; `fallback` is the layer it uses where viewports are unavailable.
    Viewport {
        /// The layer used in place of the viewport.
        fallback: Overlay,
        /// How the window opens; none opens a dialog window for a modal fallback and a tool window otherwise,
        /// with the OS's frame.
        spec: Option<WindowSpec>,
    },
}

impl Mount {
    /// The whole content of the window: its edges resize it where `resize` has grips and the app draws the frame.
    pub const fn window(resize: Resize) -> Mount {
        Mount::Window { resize }
    }

    /// An OS viewport that falls back to `fallback`, and opens as a dialog or tool window by the fallback's rank.
    pub const fn viewport(fallback: Overlay) -> Mount {
        Mount::Viewport { fallback, spec: None }
    }

    /// An OS viewport that falls back to `fallback` and opens as `spec` says.
    pub fn viewport_with(fallback: Overlay, spec: WindowSpec) -> Mount {
        Mount::Viewport { fallback, spec: Some(spec) }
    }
}

/// Where a shell pane sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PaneEdge {
    /// What the edge panes leave.
    #[default]
    Center,
    /// The start edge: left in reading order, right under right-to-left.
    Start,
    /// The end edge.
    End,
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
}

/// One of `AppShell`'s panes.
///
/// A pane is placed one of three ways. With [`at`](Self::at) it fills the rect its caller laid out, takes no room from
/// its siblings (so a pane can overlay another) and ignores the extent. Otherwise an edge pane takes
/// [`extent`](Self::extent) along its edge from the room left, or, without an extent, the size it first took, which it
/// keeps in pane memory until the user resizes it. The central pane fills what the edge panes leave.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ShellPane {
    /// The pane's name.
    pub name: &'static str,
    /// The edge the pane sits on.
    pub edge: PaneEdge,
    /// The pane's size along its edge, if fixed: the height of a top or bottom pane, the width of a start or end pane.
    pub extent: Option<f32>,
    /// The rect the caller laid the pane out in.
    pub at: Option<Rect>,
}

impl ShellPane {
    /// The central pane named `name`.
    pub const fn new(name: &'static str) -> Self {
        ShellPane { name, edge: PaneEdge::Center, extent: None, at: None }
    }

    /// The pane at `edge`.
    pub const fn edge(mut self, edge: PaneEdge) -> Self {
        self.edge = edge;
        self
    }

    /// The pane's size along its edge, exactly: the pane host shows it at this extent in every pass.
    pub const fn extent(mut self, extent: f32) -> Self {
        self.extent = Some(extent);
        self
    }

    /// The pane in `rect`, which the caller laid out for this pass. The pane takes no room from its siblings.
    pub const fn at(mut self, rect: Rect) -> Self {
        self.at = Some(rect);
        self
    }
}

/// The padding tier of a body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pad {
    /// No padding.
    #[default]
    None,
    /// `space.pad.control`.
    Control,
    /// `space.pad.regular`.
    Regular,
    /// `space.pad.roomy`.
    Roomy,
}

/// How a container's size follows its content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Sizing {
    /// The available rect.
    Fill,
    /// The available width and the height its content took last pass: a row of a column that gives no row the room
    /// left under it.
    FillWidth,
    /// The size its content took last pass.
    #[default]
    Content,
}

/// A size role that sets the least width of a container's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LeastWidth {
    /// `size.popup_min`: popup surfaces.
    Popup,
}

/// What dragging the header does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DragBand {
    /// Nothing.
    #[default]
    None,
    /// Moves the container.
    Move,
    /// Reorders the container among its siblings. The header's band takes the drag (the `REORDER_BAND` part), and
    /// Alt+Up and Alt+Down on the container give `Intent::MoveItem` instead of closing or opening it.
    Reorder,
}

/// A header region, filled by `Container::header`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HeaderRegion {
    /// What dragging the header does. A move band lies beneath the header's children, which keep their presses; an
    /// inline header's move band spans its whole line (header, body and end region), so every free point of the line
    /// moves the window or layer.
    pub band: DragBand,
    /// Whether a rule separates the header from the body. An inline header has no rule.
    pub rule: bool,
    /// Whether a rule runs across the container's width above the header, at its top edge.
    pub rule_above: bool,
    /// Whether the header shares a line with the body: the header at its measured width, the body in the rest.
    pub inline: bool,
    /// Whether the header has a trailing child region at its end edge, filled by `Container::controls`; the header
    /// itself fills the room it leaves.
    pub trailing: bool,
    /// The surface a band behind the header paints. The band spans the container's width and starts at its top edge
    /// whatever the body's padding, and has no rule.
    pub fill: Option<SurfaceKind>,
    /// The padding on the header's two sides: the header's own tier, independent of the body's.
    pub inset: Pad,
    /// The space between a stacked header and the body, none by default.
    pub gap: Option<Gap>,
}

impl HeaderRegion {
    /// A header with no drag band, no rule, on its own line, with no trailing region, no fill and no inset.
    pub const fn new() -> Self {
        HeaderRegion { band: DragBand::None, rule: false, rule_above: false, inline: false, trailing: false, fill: None, inset: Pad::None, gap: None }
    }

    /// The surface of a band behind the header.
    pub const fn fill(mut self, kind: SurfaceKind) -> Self {
        self.fill = Some(kind);
        self
    }

    /// The space between a stacked header and the body.
    pub const fn gap(mut self, gap: Gap) -> Self {
        self.gap = Some(gap);
        self
    }

    /// The padding tier between the container's edge and the header's content.
    pub const fn inset(mut self, pad: Pad) -> Self {
        self.inset = pad;
        self
    }

    /// What dragging the header does.
    pub const fn band(mut self, band: DragBand) -> Self {
        self.band = band;
        self
    }

    /// Whether a rule separates the header from the body.
    pub const fn rule(mut self, on: bool) -> Self {
        self.rule = on;
        self
    }

    /// Whether a rule runs above the header.
    pub const fn rule_above(mut self, on: bool) -> Self {
        self.rule_above = on;
        self
    }

    /// Whether the header shares a line with the body.
    pub const fn inline(mut self, on: bool) -> Self {
        self.inline = on;
        self
    }

    /// Whether the header has a trailing region for the container's controls.
    pub const fn trailing(mut self, on: bool) -> Self {
        self.trailing = on;
        self
    }
}

/// A footer region, filled by `Container::footer`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FooterRegion {
    /// Whether a rule separates the footer from the content beside it: the body, or the end region's content.
    pub rule: bool,
    /// Where the footer sits.
    pub place: FooterPlace,
    /// The space above the footer's content, inside the region; none by default.
    pub gap: Option<Gap>,
}

impl FooterRegion {
    /// A footer across the container under the line, with no rule.
    pub const fn new() -> Self {
        FooterRegion { rule: false, place: FooterPlace::Across, gap: None }
    }

    /// Whether a rule separates the footer from the content beside it.
    pub const fn rule(mut self, on: bool) -> Self {
        self.rule = on;
        self
    }

    /// Where the footer sits.
    pub const fn place(mut self, place: FooterPlace) -> Self {
        self.place = place;
        self
    }

    /// The space above the footer's content, inside the region.
    pub const fn gap(mut self, gap: Gap) -> Self {
        self.gap = Some(gap);
        self
    }
}

/// Where a footer region sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FooterPlace {
    /// Under the line, across the container's width (under the body only beside a full-height end region).
    #[default]
    Across,
    /// At the foot of the end region, as wide as it: the end region then takes the full height under the header. A
    /// container without an end region shows the footer across.
    EndBottom,
    /// At the top of the end region, above its content; otherwise as `EndBottom`.
    EndTop,
}

impl FooterPlace {
    /// Whether the footer sits in the end region.
    pub const fn in_end(self) -> bool {
        !matches!(self, FooterPlace::Across)
    }
}

/// An end region, filled by `Container::end`: content in reading order at the line's end edge, beside the body and
/// outside its scroll region, as wide as it measured. Its stops join the body's roving group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EndRegion {
    /// Whether the region takes the full height under the header, down to the container's bottom, beside the body and
    /// the footer, instead of the line's height. Its content then starts at the top, else it is centred on the line.
    pub full_height: bool,
}

impl EndRegion {
    /// An end region as high as the line.
    pub const fn new() -> Self {
        EndRegion { full_height: false }
    }

    /// Whether the region takes the full height under the header.
    pub const fn full_height(mut self, on: bool) -> Self {
        self.full_height = on;
        self
    }
}

/// Which end of the content a scroll region stays at when the content grows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScrollAnchor {
    /// The start.
    #[default]
    Start,
    /// The end (a timeline pane that follows its newest entry).
    End,
}

/// The distance of one wheel notch.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Notch {
    /// This many text lines.
    Lines(f32),
    /// This share of the view.
    ViewShare(f32),
}

impl Default for Notch {
    fn default() -> Self {
        Notch::Lines(3.0)
    }
}

/// The axes a body scrolls on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScrollAxes {
    /// Along y.
    Vertical,
    /// Along x.
    Horizontal,
    /// Both.
    Both,
}

/// A scrolling body.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Scroll {
    /// The axes that scroll.
    pub axes: ScrollAxes,
    /// The end the region stays at when the content grows.
    pub anchor: ScrollAnchor,
    /// The distance of one wheel notch.
    pub notch: Notch,
}

impl Scroll {
    const fn on(axes: ScrollAxes) -> Scroll {
        Scroll { axes, anchor: ScrollAnchor::Start, notch: Notch::Lines(3.0) }
    }

    /// Scrolls along y.
    pub const fn vertical() -> Scroll {
        Scroll::on(ScrollAxes::Vertical)
    }

    /// Scrolls along x.
    pub const fn horizontal() -> Scroll {
        Scroll::on(ScrollAxes::Horizontal)
    }

    /// Scrolls along both axes.
    pub const fn both() -> Scroll {
        Scroll::on(ScrollAxes::Both)
    }

    /// Stays at the end when the content grows.
    pub const fn anchored_end(mut self) -> Scroll {
        self.anchor = ScrollAnchor::End;
        self
    }

    /// The distance of one wheel notch.
    pub const fn notch(mut self, notch: Notch) -> Scroll {
        self.notch = notch;
        self
    }

    /// Whether the x axis scrolls.
    pub const fn horizontal_on(&self) -> bool {
        matches!(self.axes, ScrollAxes::Horizontal | ScrollAxes::Both)
    }

    /// Whether the y axis scrolls.
    pub const fn vertical_on(&self) -> bool {
        matches!(self.axes, ScrollAxes::Vertical | ScrollAxes::Both)
    }
}

/// Collapse configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Collapse {
    /// Whether the body starts open.
    pub initially_open: bool,
    /// Whether opening and closing animate.
    pub animated: bool,
}

impl Collapse {
    /// A collapsible body that starts open, animated.
    pub const fn new() -> Self {
        Collapse { initially_open: true, animated: true }
    }

    /// Whether the body starts open.
    pub const fn initially_open(mut self, on: bool) -> Self {
        self.initially_open = on;
        self
    }

    /// Whether opening and closing animate.
    pub const fn animated(mut self, on: bool) -> Self {
        self.animated = on;
        self
    }
}

impl Default for Collapse {
    fn default() -> Self {
        Collapse::new()
    }
}

/// The spacing between the children of a body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Gap {
    /// No gap: rows touch, as in native menus.
    None,
    /// `space.gap.row`.
    #[default]
    Row,
    /// `space.gap.tight`.
    Tight,
    /// `space.gap.stack`: between stacked rows.
    Stack,
    /// `space.gap.item`: between neighbouring controls in a bar, and between a bar caption and its control.
    Item,
    /// `space.gap.section`.
    Section,
    /// `space.gap.card_column`.
    CardColumn,
    /// `space.gap.card_row`.
    CardRow,
}

/// Set by a bar for its body: its fields put `space.gap.item` between a caption and the control, and a vertical
/// divider takes a box wider than its line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[doc(hidden)]
pub struct InBar(pub bool);

impl EnvValue for InBar {
    fn root() -> Self {
        InBar(false)
    }
}

/// Set by a menu for its body: a horizontal divider takes `space.gap.inline` above and below its line, since the rows
/// of a menu touch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[doc(hidden)]
pub struct InMenu(pub bool);

impl EnvValue for InMenu {
    fn root() -> Self {
        InMenu(false)
    }
}

/// What a container or collection adds to `Common`. A builder added here is added to the forwarder list in
/// `crates/ui-macros/src/props.rs` too.
///
/// A container body may run more than once per pass: a measuring run first, then the real run. It builds or borrows
/// what it shows on every run and consumes nothing (no `Option::take`, no "has run" flag); it changes state and
/// reports only through `measure::effect`. A pattern around a container never guards against a second run.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ContainerProps {
    /// Where the container lives.
    pub mount: Mount,
    /// The surface the container paints; `None` for sections, stacks and rows.
    pub surface: Option<SurfaceKind>,
    /// The padding tier.
    pub pad: Pad,
    /// How the size follows the content.
    pub sizing: Sizing,
    /// The height the body offers its children in flow, in place of the flow's (unbounded in a scrolling body): a
    /// child that fills the height is given it, any other child takes its content height. A host that shows content
    /// of either kind at a definite room (a story in a gallery card) sets it.
    pub room_height: Option<f32>,
    /// How the body lays out its children.
    pub layout: BodyLayout,
    /// A header region.
    pub header: Option<HeaderRegion>,
    /// A scroll region.
    pub scroll: Option<Scroll>,
    /// A footer region.
    pub footer: Option<FooterRegion>,
    /// An end region at the line's end edge.
    pub end: Option<EndRegion>,
    /// The help line the footer shows under the control column of a label-scoped container; its text joins the
    /// control's description.
    pub help: Option<Label>,
    /// The container's place in its set, reported on its node.
    pub position: Option<SetPosition>,
    /// The focus scope of an `Inline`, `Pane` or `FullWindow` mount.
    pub focus: FocusPolicy,
    /// Collapse configuration; the initial state seeds the open slot.
    pub collapse: Option<Collapse>,
    /// The spacing between the children of the body.
    pub body_gap: Gap,
    /// The least width of the content, from a size role; the content may be wider.
    pub least_width: Option<LeastWidth>,
    /// A scope size for the body.
    pub child_size: Option<ControlSize>,
    /// The container is an arena root.
    pub arena: bool,
    /// The container's value is a gate that disables its content while off.
    #[doc(hidden)]
    pub gates: bool,
    /// The body is shown through this transform on its own egui layer.
    pub transform: Option<CanvasTransform>,
    /// Token-changing values for the body.
    pub theme: Option<ThemeScope>,
    /// The container is an instance of this command scope.
    pub command_scope: Option<CommandScope>,
    /// A live region.
    pub live: Option<Politeness>,
    /// A field label.
    pub label: Option<SideLabel>,
    /// The body's members are one roving stop.
    pub roving: Option<Roving>,
    /// Placement, size and open state saved across runs.
    pub save: Option<SaveKey>,
    /// One outer gesture over the body's fields (`runtime::frame::OuterEdit`), from open to close. The dialog shells
    /// read it: the body's header opens the gesture, and the dialog's answer ends it.
    pub outer_edit: bool,
    /// The body holds one collection that owns the scroll: a layer's own body scroll is off.
    pub collection_body: bool,
    /// The labelled rows of the body share the widest label as one column.
    pub label_column: bool,
}

impl Default for ContainerProps {
    fn default() -> Self {
        ContainerProps::new()
    }
}

impl ContainerProps {
    /// A fingerprint of the props a body's measure reads: equal props give equal fingerprints. What it leaves out
    /// (focus, arena, gates, commands, live region, roving, save key, set position) does not change a size. Glyphs
    /// show as `Glyph(..)`: an icon is a component of the content, which the content check asks.
    pub(crate) fn measure_fingerprint(&self) -> u64 {
        use std::fmt::Write as _;
        /// FNV-1a over the `Debug` text, without building it: `Debug` writes every `f32` so that it reads back
        /// exactly.
        struct Fnv(u64);
        impl std::fmt::Write for Fnv {
            fn write_str(&mut self, text: &str) -> std::fmt::Result {
                for byte in text.bytes() {
                    self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
                }
                Ok(())
            }
        }
        let mut hash = Fnv(0xcbf2_9ce4_8422_2325);
        let _ = write!(
            hash,
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            self.mount,
            self.surface,
            self.pad,
            self.sizing,
            self.room_height,
            self.layout,
            self.header,
            self.scroll,
            self.footer,
            self.end,
            self.help,
            self.collapse,
            self.body_gap,
            self.least_width,
            self.child_size,
            self.transform,
            self.theme,
            self.label,
            self.outer_edit,
            self.collection_body,
        );
        hash.0
    }
    /// Props with every field at its default.
    pub const fn new() -> Self {
        ContainerProps {
            mount: Mount::Inline,
            surface: None,
            pad: Pad::None,
            sizing: Sizing::Content,
            room_height: None,
            layout: BodyLayout::Stack,
            header: None,
            scroll: None,
            footer: None,
            end: None,
            help: None,
            position: None,
            focus: FocusPolicy::NONE,
            collapse: None,
            body_gap: Gap::Row,
            least_width: None,
            child_size: None,
            arena: false,
            gates: false,
            transform: None,
            theme: None,
            command_scope: None,
            live: None,
            label: None,
            roving: None,
            save: None,
            outer_edit: false,
            collection_body: false,
            label_column: false,
        }
    }

    /// Where the container lives.
    pub fn mount(mut self, mount: Mount) -> Self {
        self.mount = mount;
        self
    }

    /// The surface the container paints.
    pub const fn surface(mut self, surface: SurfaceKind) -> Self {
        self.surface = Some(surface);
        self
    }

    /// The padding tier.
    pub const fn pad(mut self, pad: Pad) -> Self {
        self.pad = pad;
        self
    }

    /// How the size follows the content.
    pub const fn sizing(mut self, sizing: Sizing) -> Self {
        self.sizing = sizing;
        self
    }

    /// The height the body offers its children in flow ([`room_height`](Self::room_height)).
    pub const fn room_height(mut self, height: f32) -> Self {
        self.room_height = Some(height);
        self
    }

    /// How the body lays out its children.
    pub fn layout(mut self, layout: BodyLayout) -> Self {
        self.layout = layout;
        self
    }

    /// A header region.
    pub const fn header(mut self, header: HeaderRegion) -> Self {
        self.header = Some(header);
        self
    }

    /// A scroll region.
    pub const fn scroll(mut self, scroll: Scroll) -> Self {
        self.scroll = Some(scroll);
        self
    }

    /// The body holds one collection that owns the scroll (a popup list): a layer's own body scroll is off, and the
    /// collection scrolls inside the layer's capped room.
    pub const fn collection_body(mut self) -> Self {
        self.collection_body = true;
        self
    }

    /// A footer region.
    pub const fn footer(mut self, footer: FooterRegion) -> Self {
        self.footer = Some(footer);
        self
    }

    /// An end region at the line's end edge, outside the body's scroll region. Its stops join the body's roving
    /// group.
    pub const fn end(mut self, end: EndRegion) -> Self {
        self.end = Some(end);
        self
    }

    /// The help line the footer shows under the control column. Its text joins the description of the control the
    /// label scope names, as a validity message does; the footer draws the visible line.
    pub fn help(mut self, help: impl Into<Label>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// The container's place in its set (`n of m`), reported on its node.
    pub const fn position(mut self, position: SetPosition) -> Self {
        self.position = Some(position);
        self
    }

    /// The focus scope.
    pub const fn focus(mut self, policy: FocusPolicy) -> Self {
        self.focus = policy;
        self
    }

    /// Collapse configuration.
    pub const fn collapse(mut self, collapse: Collapse) -> Self {
        self.collapse = Some(collapse);
        self
    }

    /// The spacing between the children of the body.
    pub const fn body_gap(mut self, gap: Gap) -> Self {
        self.body_gap = gap;
        self
    }

    /// The least width of the content, from a size role.
    pub const fn least_width(mut self, least: LeastWidth) -> Self {
        self.least_width = Some(least);
        self
    }

    /// A scope size for the body.
    pub const fn child_size(mut self, size: ControlSize) -> Self {
        self.child_size = Some(size);
        self
    }

    /// Makes the container's `bool` value a gate on its content.
    #[doc(hidden)]
    pub const fn gates(mut self) -> Self {
        self.gates = true;
        self
    }

    /// Makes the container an arena root.
    pub const fn arena(mut self, on: bool) -> Self {
        self.arena = on;
        self
    }

    /// Shows the body through `transform` on its own egui layer.
    pub const fn transform(mut self, transform: CanvasTransform) -> Self {
        self.transform = Some(transform);
        self
    }

    /// Token-changing values for the body.
    pub fn theme(mut self, theme: ThemeScope) -> Self {
        self.theme = Some(theme);
        self
    }

    /// Makes the container an instance of `scope`.
    pub const fn command_scope(mut self, scope: CommandScope) -> Self {
        self.command_scope = Some(scope);
        self
    }

    /// A live region.
    pub const fn live(mut self, politeness: Politeness) -> Self {
        self.live = Some(politeness);
        self
    }

    /// A field label.
    pub fn label(mut self, label: SideLabel) -> Self {
        self.label = Some(label);
        self
    }

    /// Makes the body's members one roving stop.
    pub const fn roving(mut self, roving: Roving) -> Self {
        self.roving = Some(roving);
        self
    }

    /// Saves placement, size and open state under `key`.
    pub fn save(mut self, key: SaveKey) -> Self {
        self.save = Some(key);
        self
    }

    /// One outer gesture over the body's fields, from open to close.
    pub const fn outer_edit(mut self, on: bool) -> Self {
        self.outer_edit = on;
        self
    }

    /// The labelled rows of the body share the widest label as one column. A nested container with its own
    /// label column opens its own group; a row reads the innermost. A menu body is such a group: its rows share
    /// their mark and icon columns through it.
    pub const fn label_column(mut self, on: bool) -> Self {
        self.label_column = on;
        self
    }
}
