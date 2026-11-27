//! `ContainerProps` and the types of its fields (16.2).

use super::label::{LabelScheme, SideLabel};
use super::overlay::{Overlay, Resize};
use crate::layout::BodyLayout;
use crate::runtime::access::{Politeness, SetPosition};
use crate::runtime::commands::CommandScope;
use crate::runtime::env::{EnvValue, Label};
use crate::runtime::focus::FocusPolicy;
use crate::runtime::memory::SaveKey;
use crate::runtime::windows::WindowSpec;
use crate::style::{GlyphRole, SurfaceKind};
use craft_behavior::{CanvasTransform, Roving};
use craft_theme::ControlSize;
use craft_theme::variant::Tone;
use egui::Rect;

pub use crate::runtime::env::{InBar, PaneEdge, ThemeScope};
pub use crate::style::BandFill;

/// Where a container lives.
#[derive(Clone, Debug, Default, PartialEq)]
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

/// One of `AppShell`'s panes.
///
/// A pane is placed one of three ways. With [`at`](Self::at) it fills the rect its caller laid out, takes no room from
/// its siblings (so a pane can overlay another) and ignores the extent. Otherwise an edge pane takes
/// [`extent`](Self::extent) along its edge from the room left, or, without an extent, the size it first took, which it
/// keeps in pane memory until the user resizes it. The central pane fills what the edge panes leave. A pane at a rect
/// that is as large as its content along its edge (a content-sized embedded shell's body, a slot sized by its content)
/// takes the rect's cross extent, its content's extent from the rect's outer edge, and that room in the flow of its
/// parent: a top or central pane its height from the rect's top, a bottom pane its height from the rect's bottom, a
/// start or end pane its width from that side.
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
    /// The pane at `at` is as large as its content along its edge.
    pub(crate) content_extent: bool,
}

impl ShellPane {
    /// The central pane named `name`.
    pub const fn new(name: &'static str) -> Self {
        ShellPane { name, edge: PaneEdge::Center, extent: None, at: None, content_extent: false }
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

    /// The pane in `rect`, as large as its content along its edge from the rect's outer edge, taking that room in its
    /// parent's flow. `rect` is the room the content may take.
    #[doc(hidden)]
    pub const fn at_content_extent(mut self, rect: Rect) -> Self {
        self.at = Some(rect);
        self.content_extent = true;
        self
    }
}

/// The padding tier of a body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, crate::contract::MemoProps)]
#[non_exhaustive]
pub enum Pad {
    /// No padding.
    #[default]
    None,
    /// `space.gap.tight`: a hairline frame around a compact group, as a small floating action bar's.
    Tight,
    /// `space.pad.control`.
    Control,
    /// `space.pad.regular`.
    Regular,
    /// `space.pad.dialog`: a dialog frame.
    Dialog,
    /// `space.pad.roomy`.
    Roomy,
    /// A full-window page's margin around its content: two and a half roomy pads (40 pt in the compact set), as a
    /// File page's body stands off the window and its side menu.
    Page,
}

/// The edges a container's padding tier applies to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PadEdges {
    /// Every edge.
    #[default]
    All,
    /// The top and bottom edges only: a column given its width whose children centre across it, as a tool rail does
    /// in its pane.
    Block,
    /// The start and end edges only: a row given its height that centres its line in it, as a bar does in its pane.
    /// The body picks it for such a row whatever the props name.
    Inline,
}

/// The padding of every popup surface around its content.
pub const POPUP_PAD: Pad = Pad::Control;

/// How a container's size follows its content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Sizing {
    /// The available rect.
    Fill,
    /// The available width and the height its content took last pass: a row of a column that gives no row the room
    /// left under it.
    FillWidth,
    /// The size its content took last pass. Shown in a `Stack` given its width, it passes that width on: a child that
    /// fills takes it, and the container with it. A content-sized mount, a row or a grid stops that.
    #[default]
    Content,
    /// The width its content took last pass and the available height: a side menu beside a page, as wide as its
    /// widest item and as tall as the row it stands in, with its footer at the bottom. Across the width it is
    /// `Content`, down the height `Fill`.
    FillHeight,
}

impl Sizing {
    /// Whether the width follows the content: `Content` and `FillHeight`.
    pub const fn content_width(self) -> bool {
        matches!(self, Sizing::Content | Sizing::FillHeight)
    }

    /// Whether the height is the available height: `Fill` and `FillHeight`.
    pub const fn fills_height(self) -> bool {
        matches!(self, Sizing::Fill | Sizing::FillHeight)
    }
}

/// A size role that sets the least width of a container's content or of a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LeastWidth {
    /// `size.popup_min`: popup surfaces.
    Popup,
    /// `size.dialog.least`: a dialog's content.
    Dialog,
    /// `size.dialog.button`: a dialog's footer button.
    DialogButton,
    /// `size.dialog.side_button`: a button in a dialog's column of buttons beside the body.
    DialogSideButton,
    /// `size.rail.width` per column: a tool rail of this many columns, as wide as the shell slot that holds it.
    Rail(std::num::NonZeroU8),
    /// `size.dialog.least` and `size.dialog.wrap` together: a wide dialog, as wide as a dialog's least content beside a
    /// paragraph's line (an About dialog whose tabs hold credit tables).
    WideDialog,
    /// Five `size.rail.width`: a page's side menu (a File page's list of pages), wide enough that its rows read as a
    /// band beside the page, not as buttons cut to their labels.
    SideMenu,
    /// `size.dock.column`: a fixed side pane is as wide as a dock column.
    DockColumn,
}

impl LeastWidth {
    /// The width the role names in `tokens`.
    pub fn points(self, tokens: &craft_theme::Tokens) -> f32 {
        match self {
            LeastWidth::Popup => tokens.size.popup_min,
            LeastWidth::Dialog => tokens.size.dialog.least,
            LeastWidth::DialogButton => tokens.size.dialog.button,
            LeastWidth::DialogSideButton => tokens.size.dialog.side_button,
            LeastWidth::Rail(columns) => tokens.size.rail.width * f32::from(columns.get()),
            LeastWidth::WideDialog => tokens.size.dialog.least + tokens.size.dialog.wrap,
            LeastWidth::SideMenu => tokens.size.rail.width * 5.0,
            LeastWidth::DockColumn => tokens.size.dock.column,
        }
    }
}

/// A size role that sets the line length of the paragraphs in a container whose width comes from its content: a
/// wrapping text breaks its lines there when its room is wider, and content wider than it still widens the container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WrapWidth {
    /// `size.dialog.wrap`: a dialog's paragraphs.
    Dialog,
    /// `size.dock.column`: a page's column of settings beside a preview (a Print page), as wide as a dock column.
    Column,
}

impl WrapWidth {
    /// The width the role names in `tokens`.
    pub fn points(self, tokens: &craft_theme::Tokens) -> f32 {
        match self {
            WrapWidth::Dialog => tokens.size.dialog.wrap,
            WrapWidth::Column => tokens.size.dock.column,
        }
    }
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
    /// Reorders the container among its siblings. The header's band takes the drag (the `REORDER_BAND` part), and the
    /// stacking keys (`keys::REORDER_KEYS`) anywhere in the container give `Intent::MoveItem`.
    Reorder,
}

/// The room a container's own parts take on its header's line (`Container::header_parts`): a width at the start edge
/// and one at the end edge. The header's children take the rest of the line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct HeaderParts {
    /// The width at the start edge.
    pub start: f32,
    /// The width at the end edge.
    pub end: f32,
    /// The least the end edge narrows to, when the line is too narrow for the header's children to keep `keep`
    /// beside the end parts' whole width. `None`: the end edge keeps its width.
    pub end_least: Option<f32>,
    /// The room the header's children keep before the end edge narrows below `end` (to `end_least`).
    pub keep: f32,
}

impl HeaderParts {
    /// No room for parts.
    pub const NONE: HeaderParts = HeaderParts { start: 0.0, end: 0.0, end_least: None, keep: 0.0 };

    /// `width` at the start edge.
    pub const fn start(mut self, width: f32) -> Self {
        self.start = width;
        self
    }

    /// `width` at the end edge.
    pub const fn end(mut self, width: f32) -> Self {
        self.end = width;
        self
    }

    /// The end edge narrows, down to `least`, so the header's children keep `keep` of the line: on a line too narrow
    /// for both, the children's least room comes before the end parts' whole width.
    pub const fn end_narrows(mut self, least: f32, keep: f32) -> Self {
        self.end_least = Some(least);
        self.keep = keep;
        self
    }
}

/// An edge of a header's line, where a container's own header parts sit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeaderEdge {
    /// The start edge: the parts there come before the header's children, in Tab order too.
    Start,
    /// The end edge: the parts there come after the header's children, in Tab order too.
    End,
}

/// The room around the rule above a header ([`HeaderRegion::rule_above`]), as a `Section` has. An app sets it once at its
/// root (`RootEnv::values`); the default is `Tight`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::runtime::env::EnvValue, crate::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum SectionRhythm {
    /// The rule takes the gap its flow leaves before the container, and the header follows it at once.
    #[default]
    Tight,
    /// The rule stands `space.gap.section` below what precedes it in its `Stack`, in place of the stack's gap (no room
    /// at its flow's start), and the header `space.gap.row` below it: production VectorCraft's panel sections.
    Spaced,
}

/// Whether a titled header that draws no rule under it (a quiet title, [`HeaderRegion::heads`]) still heads its body as
/// a rule would. An app sets it once at its root (`RootEnv::values`); the default is `Ruled`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::runtime::env::EnvValue, crate::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum QuietHeader {
    /// The body's first section draws its own rule above, as under no header.
    #[default]
    Ruled,
    /// The header heads the body: its flow starts as after a rule, so the first section under the title draws no rule
    /// above it and sections after it keep theirs. Production VectorCraft's kind line over its inspector's sections.
    Heads,
}

/// The cursor over a header's move band ([`DragBand::Move`]). An app sets it once at its root (`RootEnv::values`); the
/// default is `Hand`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::runtime::env::EnvValue, crate::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum MoveBandCursor {
    /// The open hand, and the closed hand while the band is dragged.
    #[default]
    Hand,
    /// The arrow, as an OS title bar shows, also over the click-only controls on the band (menu titles, buttons):
    /// production VectorCraft's title bar and panel headers.
    Arrow,
}

/// A header region, filled by `Container::header`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HeaderRegion {
    /// What dragging the header does. A move band lies beneath the header's children, which keep their presses; an
    /// inline header's move band spans its whole line (header, body and end region), so every free point of the line
    /// moves the window or layer.
    pub band: DragBand,
    /// Whether a rule separates the header from the body, in a band of its own between them. An inline header has no
    /// rule.
    pub rule: bool,
    /// Whether the header is a title that heads the body as a rule would where the app asks for it
    /// ([`QuietHeader::Heads`]), though it draws none.
    pub heads: bool,
    /// Whether a rule runs across the container's width above the header, in a band of its own at the top edge.
    pub rule_above: bool,
    /// Whether the header shares a line with the body: the header at its measured width, the body in the rest.
    pub inline: bool,
    /// Whether the header has a trailing child region at its end edge, filled by `Container::controls`; the header
    /// itself fills the room it leaves.
    pub trailing: bool,
    /// What a band behind the header paints. The band spans the container's width and starts at its top edge
    /// whatever the body's padding, and has no rule.
    pub fill: Option<BandFill>,
    /// The padding on the header's two sides: the header's own tier, independent of the body's.
    pub inset: Pad,
    /// The space between a stacked header and the body, none by default.
    pub gap: Option<Gap>,
}

impl HeaderRegion {
    /// A header with no drag band, no rule, on its own line, with no trailing region, no fill and no inset.
    pub const fn new() -> Self {
        HeaderRegion {
            band: DragBand::None,
            rule: false,
            heads: false,
            rule_above: false,
            inline: false,
            trailing: false,
            fill: None,
            inset: Pad::None,
            gap: None,
        }
    }

    /// The surface of a band behind the header.
    pub const fn fill(mut self, kind: SurfaceKind) -> Self {
        self.fill = Some(BandFill::Surface(kind));
        self
    }

    /// A band behind the header in `tone`'s tinted fill (a slicer's caption band); the header's title row takes the
    /// tone's ink for that fill (`TitleRow::tone`).
    pub const fn tint(mut self, tone: Tone) -> Self {
        self.fill = Some(BandFill::Tone(tone));
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

    /// Whether the header is a title that heads the body as a rule would where the app asks for it
    /// ([`QuietHeader::Heads`]), though it draws none.
    pub const fn heads(mut self, on: bool) -> Self {
        self.heads = on;
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
    /// Whether a rule separates the footer from the content beside it (the body, or the end region's content), in a
    /// band of its own.
    pub rule: bool,
    /// Where the footer sits.
    pub place: FooterPlace,
    /// The space above the footer's content, inside the region; none by default.
    pub gap: Option<Gap>,
    /// The padding between the container's edges and the footer's content, at its two sides and under it: the
    /// footer's own tier, for a container whose body has no padding.
    pub inset: Pad,
    /// The footer shows the validity message of the control the container labels as its status line
    /// (`BodyCx::field_message`); a control under a container without one shows its message itself.
    pub status: bool,
}

impl FooterRegion {
    /// A footer across the container under the line, with no rule and no inset.
    pub const fn new() -> Self {
        FooterRegion { rule: false, place: FooterPlace::Across, gap: None, inset: Pad::None, status: false }
    }

    /// The padding tier between the container's edges and the footer's content, at its sides and under it.
    pub const fn inset(mut self, pad: Pad) -> Self {
        self.inset = pad;
        self
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

    /// The footer shows the validity message of the control the container labels as its status line.
    pub const fn status(mut self) -> Self {
        self.status = true;
        self
    }
}

/// Where a footer region sits. In every place, `Container::body_end` fills the body's column under its content, so
/// a notice it shows lies under the body and above the footer, or beside a footer in the end region.
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
    /// In the body, at the cell where the body shows a [`FooterSlot`](crate::contract::FooterSlot): the footer takes no
    /// band of its own, and the slot is as large as the footer measured. The container still runs the footer, so its
    /// controls stay the container's (a dialog's buttons, their keys and their order); the body only chooses where
    /// they stand. The body shows exactly one slot.
    Body,
}

impl FooterPlace {
    /// Whether the footer sits in the end region.
    pub const fn in_end(self) -> bool {
        matches!(self, FooterPlace::EndBottom | FooterPlace::EndTop)
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
    /// The gap between the body and the region.
    pub gap: Gap,
    /// Whether a rule separates the end region from the body, in a band of its own along the region's edge that
    /// faces it.
    pub rule: bool,
    /// Whether the region joins the body's fit: when the body's units ask for more than the line the region leaves,
    /// the region's units that carry a `fit_rank` leave first, lowest rank first, until the body's units fit at their
    /// ideal widths, before any body unit narrows or leaves.
    pub fit: bool,
}

impl EndRegion {
    /// An end region as high as the line, `space.gap.tight` from the body, with no rule.
    pub const fn new() -> Self {
        EndRegion { full_height: false, gap: Gap::Tight, rule: false, fit: false }
    }

    /// Whether the region joins the body's fit.
    pub const fn fit(mut self, on: bool) -> Self {
        self.fit = on;
        self
    }

    /// Whether a rule separates the end region from the body.
    pub const fn rule(mut self, on: bool) -> Self {
        self.rule = on;
        self
    }

    /// Whether the region takes the full height under the header.
    pub const fn full_height(mut self, on: bool) -> Self {
        self.full_height = on;
        self
    }

    /// Sets the gap between the body and the region.
    pub const fn gap(mut self, gap: Gap) -> Self {
        self.gap = gap;
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

impl Notch {
    /// The wheel distance of the notch, with a text line `line` points high.
    pub fn wheel(self, line: f32) -> craft_behavior::WheelNotch {
        match self {
            Notch::Lines(lines) => craft_behavior::WheelNotch::Points(lines * line),
            Notch::ViewShare(share) => craft_behavior::WheelNotch::ViewShare(share),
        }
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

/// How a scrolling body shows that its content overflows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScrollIndicator {
    /// A scroll bar over the view's end edge, and fades at the edges past which content is hidden.
    Bar,
    /// A strip with an arrow above the view and one below it while the content overflows along y, as a menu shows:
    /// resting the pointer on a strip scrolls toward it. The strips take their height from the view, never width.
    Strips,
    /// A bar that takes no room from the content, as a tool rail or an options bar shows its overflow: it lies in the
    /// container's end padding (its bottom padding for x), over the content where the container has none, and shows
    /// only while the pointer is over the container or presses the bar, or while the content overflows when the scroll
    /// shows it at rest ([`Scroll::at_rest`]).
    Overlay,
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
    /// How the overflow shows.
    pub indicator: ScrollIndicator,
    /// Whether an overlay bar also shows while the pointer is away from the container.
    pub at_rest: bool,
}

impl Scroll {
    const fn on(axes: ScrollAxes) -> Scroll {
        Scroll { axes, anchor: ScrollAnchor::Start, notch: Notch::Lines(3.0), indicator: ScrollIndicator::Bar, at_rest: false }
    }

    /// Shows the overflow along y as scroll strips instead of a bar.
    pub const fn strips(mut self) -> Scroll {
        self.indicator = ScrollIndicator::Strips;
        self
    }

    /// Whether the overflow along y shows as scroll strips.
    pub const fn has_strips(&self) -> bool {
        matches!(self.indicator, ScrollIndicator::Strips) && self.vertical_on()
    }

    /// Shows the overflow as a bar in the container's padding that takes no room from the content and shows only
    /// under the pointer ([`ScrollIndicator::Overlay`]).
    pub const fn overlay(mut self) -> Scroll {
        self.indicator = ScrollIndicator::Overlay;
        self
    }

    /// Whether the overflow shows as an overlay bar.
    pub const fn has_overlay(&self) -> bool {
        matches!(self.indicator, ScrollIndicator::Overlay)
    }

    /// Shows an overlay bar whenever the content overflows, also while the pointer is away from the container. A bar of
    /// another indicator already shows at rest.
    pub const fn at_rest(mut self) -> Scroll {
        self.at_rest = true;
        self
    }

    /// Whether the overlay bar shows only while the pointer is over the container or presses the bar.
    pub const fn overlay_hides_at_rest(&self) -> bool {
        self.has_overlay() && !self.at_rest
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
    /// `space.gap.inline`: between groups of tools in a rail.
    Inline,
    /// `size.page.gap.title`: between a page's title and its subtitle.
    PageTitle,
    /// `size.page.gap.lead`: between a page's title and subtitle and the actions under them.
    PageLead,
    /// `size.page.gap.actions`: between a page's New and Open.
    PageActions,
    /// `size.page.gap.section`: between the sections of a page's column.
    PageSection,
    /// `size.page.gap.heading`: between a page section's heading and its body.
    PageHeading,
    /// `size.page.gap.link`: between a page's links.
    PageLink,
}

/// Set by a menu for its body: a horizontal divider takes `space.gap.inline` above and below its line, since the rows
/// of a menu touch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, crate::contract::MemoProps)]
#[doc(hidden)]
pub struct InMenu(pub bool);

impl EnvValue for InMenu {
    fn root() -> Self {
        InMenu(false)
    }
}

/// The bracket a `Row` draws from the labelled rows of its first child to the child after them (Image Size's width and
/// height to their chain): a tick from each of the first and the last row's controls to the middle child's centre
/// line, joined along that line. The middle child sits halfway between the two controls. The value is the link's
/// state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Bracket {
    /// The rows are linked: the bracket is drawn.
    Linked,
    /// The rows are not linked: the middle child keeps its place, with no bracket.
    Unlinked,
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
    /// The edges the padding tier applies to; a collection pads every edge.
    pub pad_edges: PadEdges,
    /// How the size follows the content.
    pub sizing: Sizing,
    /// Among the members of a bar (`InBar`) the container takes its content width, whatever `sizing` says: a field
    /// fills the width of a form and fits its content in a bar. Setting `sizing` clears it.
    pub content_in_bar: bool,
    /// The height the body offers its children in flow, in place of the flow's (unbounded in a scrolling body): the
    /// most a child takes. Every child takes its content height, a child that fills or scrolls at most this height,
    /// as a scrolling list is as tall as its rows until they outgrow the room. A host that shows content of either
    /// kind at a definite room (a story in a gallery card) sets it.
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
    /// The accessible name of each resize grip of a window or layer mount; its grips report no node without one.
    pub resize_name: Option<Label>,
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
    /// The least height of an unwrapped `Row` body's line, in points: the line is at least this high, as if a child of
    /// more than one line this high stood in it, so the children that fill the line's height take at least it. Other
    /// layouts ignore it.
    pub least_height: Option<f32>,
    /// The line length of the body's paragraphs while the container takes its width from its content, from a size role.
    pub wrap_width: Option<WrapWidth>,
    /// A divider along the inner edge of a container mounted as an edge pane (`Mount::Pane` at an edge): the edge that
    /// faces the window's centre. Any other container draws nothing.
    pub edge_rule: bool,
    /// A scope size for the regions: header, body, end region and footer.
    pub child_size: Option<ControlSize>,
    /// A scope glyph role for the regions: how the icon-only buttons in them show their glyph.
    pub child_glyphs: Option<GlyphRole>,
    /// The container is an arena root.
    pub arena: bool,
    /// The container is its body's scroll owner without a scroll region, as a canvas with a view of its own is: the
    /// page and end moves and the reveals its children leave go to it (`Focus::take_scroll_moves`,
    /// `Focus::take_reveal`). A container with a `scroll` region is one already. The body is clipped to its body
    /// region, the view, so what it shows never paints over the container's padding (a canvas's rulers).
    pub scroll_owner: bool,
    /// A collection that receives moves the focused control in this region leaves unused.
    pub fallback_collection: Option<egui::Id>,
    /// The container's value is a gate that disables its content while off.
    #[doc(hidden)]
    pub gates: bool,
    /// The body is shown through this transform on its own egui layer.
    pub transform: Option<CanvasTransform>,
    /// Another theme definition, mode, contrast or density for the container itself (its surface, header, rules,
    /// scroll bars, pads and focus ring), everything it shows, and every layer opened from inside it.
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
    /// The rank by which a body unit of a shrinking row leaves whole; unranked units never leave by rank.
    pub fit_rank: Option<u8>,
    /// How the labelled rows under the container show their labels; `None` keeps the scheme of the scope.
    pub labels: Option<LabelScheme>,
    /// The bracket an unwrapped `Row` placed from its start draws from its first child's labelled rows to the child
    /// after them; any other body draws none.
    pub bracket: Option<Bracket>,
    /// The component whose memory holds the container's shown memory and its layer's bookkeeping, when it is another:
    /// the body of a popup an `overlay` hook shows belongs to the component that shows it.
    pub(crate) owner: Option<egui::Id>,
}

impl Default for ContainerProps {
    fn default() -> Self {
        ContainerProps::new()
    }
}

impl ContainerProps {
    /// Whether a body measured under `other` measures the same under these props. What it leaves out (focus, arena,
    /// gates, commands, live region, roving, save key, set position) does not change a size.
    pub(crate) fn measures_like(&self, other: &ContainerProps) -> bool {
        self.mount == other.mount
            && self.surface == other.surface
            && self.pad == other.pad
            && self.pad_edges == other.pad_edges
            && self.sizing == other.sizing
            && self.content_in_bar == other.content_in_bar
            && self.room_height == other.room_height
            && self.fit_rank == other.fit_rank
            && self.layout == other.layout
            && self.header == other.header
            && self.scroll == other.scroll
            && self.footer == other.footer
            && self.end == other.end
            && self.help == other.help
            && self.collapse == other.collapse
            && self.body_gap == other.body_gap
            && self.least_width == other.least_width
            && self.least_height == other.least_height
            && self.wrap_width == other.wrap_width
            && self.child_size == other.child_size
            && self.child_glyphs == other.child_glyphs
            && self.transform == other.transform
            && self.theme == other.theme
            && self.label == other.label
            && self.outer_edit == other.outer_edit
            && self.collection_body == other.collection_body
            && self.labels == other.labels
            && self.bracket == other.bracket
    }

    /// Props with every field at its default.
    pub const fn new() -> Self {
        ContainerProps {
            mount: Mount::Inline,
            surface: None,
            pad: Pad::None,
            pad_edges: PadEdges::All,
            sizing: Sizing::Content,
            content_in_bar: false,
            room_height: None,
            layout: BodyLayout::Stack,
            header: None,
            scroll: None,
            footer: None,
            end: None,
            help: None,
            resize_name: None,
            position: None,
            focus: FocusPolicy::NONE,
            collapse: None,
            body_gap: Gap::Row,
            least_width: None,
            least_height: None,
            wrap_width: None,
            edge_rule: false,
            child_size: None,
            child_glyphs: None,
            arena: false,
            scroll_owner: false,
            fallback_collection: None,
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
            fit_rank: None,
            labels: None,
            bracket: None,
            owner: None,
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

    /// The edges the padding tier applies to.
    pub const fn pad_edges(mut self, edges: PadEdges) -> Self {
        self.pad_edges = edges;
        self
    }

    /// How the size follows the content.
    pub const fn sizing(mut self, sizing: Sizing) -> Self {
        self.sizing = sizing;
        self.content_in_bar = false;
        self
    }

    /// Takes the content width among the members of a bar ([`content_in_bar`](Self::content_in_bar)).
    pub const fn content_in_bar(mut self) -> Self {
        self.content_in_bar = true;
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

    /// The accessible name of every resize grip of the container's window or layer.
    pub fn resize_name(mut self, name: impl Into<Label>) -> Self {
        self.resize_name = Some(name.into());
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

    /// The least height of an unwrapped `Row` body's line ([`least_height`](Self::least_height)).
    pub const fn least_height(mut self, points: f32) -> Self {
        self.least_height = Some(points);
        self
    }

    /// The line length of the body's paragraphs while the container takes its width from its content, from a size role.
    pub const fn wrap_width(mut self, wrap: WrapWidth) -> Self {
        self.wrap_width = Some(wrap);
        self
    }

    /// Draws a divider along the inner edge of a container mounted as an edge pane.
    pub const fn edge_rule(mut self, on: bool) -> Self {
        self.edge_rule = on;
        self
    }

    /// A scope size for the regions: header, body, end region and footer.
    pub const fn child_size(mut self, size: ControlSize) -> Self {
        self.child_size = Some(size);
        self
    }

    /// A scope glyph role for the regions: a rail's buttons show its icons at the rail's side and ink.
    pub const fn child_glyphs(mut self, role: GlyphRole) -> Self {
        self.child_glyphs = Some(role);
        self
    }

    /// Makes the container's `bool` value a gate on its content.
    #[doc(hidden)]
    pub const fn gates(mut self) -> Self {
        self.gates = true;
        self
    }

    /// Sets the rank by which the container leaves a shrinking row whole.
    pub const fn fit_rank(mut self, rank: u8) -> Self {
        self.fit_rank = Some(rank);
        self
    }

    /// Makes the container an arena root.
    pub const fn arena(mut self, on: bool) -> Self {
        self.arena = on;
        self
    }

    /// Makes the container its body's scroll owner with a view of its own (`ContainerProps::scroll_owner`).
    pub const fn scroll_owner(mut self, on: bool) -> Self {
        self.scroll_owner = on;
        self
    }

    /// Sends unused moves from this region's focused control to the named collection.
    pub fn fallback_collection(mut self, collection: egui::Id) -> Self {
        self.fallback_collection = Some(collection);
        self
    }

    /// Shows the body through `transform` on its own egui layer.
    pub const fn transform(mut self, transform: CanvasTransform) -> Self {
        self.transform = Some(transform);
        self
    }

    /// Resolves the container, everything it shows and every layer opened from inside it through `theme`.
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

    /// The bracket of a `Row` from its first child's labelled rows to the child after them ([`bracket`](Self::bracket)).
    pub const fn bracket(mut self, bracket: Bracket) -> Self {
        self.bracket = Some(bracket);
        self
    }

    /// The labelled rows under the container show their labels in `scheme`: its regions, its body and every container
    /// inside, until one sets a scheme of its own.
    pub const fn labels(mut self, scheme: LabelScheme) -> Self {
        self.labels = Some(scheme);
        self
    }
}
