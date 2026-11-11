//! `HeaderFooterZones`: the header and footer bands of the pages on screen, each in three sections (start, centre, end),
//! as Excel's Page Layout view and Word show them: the hovered section says "Add header" or "Add footer" when it is
//! empty, a double click (or Enter on the focused section) starts editing it, and the app's editor takes the section's
//! rectangle. What a section holds, its fields and the editor are the app's.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::kurbo::Point;
use craft_behavior::{HitLevel, HitPriority, Input, Intent, Metrics, PartKey, Sense};
use craft_ui::canvas::canvas_space;
use craft_ui::contract::{
    Access, Common, Component, Cx, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _, Props, Role, WordSetCx as _,
};
use craft_ui::fonts::FontRef;
use craft_ui::primitives::{Dashes, DocRun, DocText, HAlign, PaintShape, StrokeWidth, TextRun};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, DataColor, FocusLook, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

use crate::ui::view::PlacedPage;

/// The section `ZONE.at(n)`: `n` counts a page's header sections, then its footer sections, page by page.
const ZONE: PartKey = PartKey::new("zone");

/// The header and footer text size, page points (GridCraft's `hf_font` base).
pub const HEADER_TEXT: f32 = 10.0;

/// The screen sizes header and footer text is kept between at extreme zooms (GridCraft clamps 4 to 120): limits of
/// document text drawn at a zoom, not interface sizes.
pub const HEADER_TEXT_SCREEN: std::ops::RangeInclusive<f32> = 4.0..=120.0;

craft_ui::words! {
    /// The zones' words (GridCraft's strings).
    pub struct HeaderFooterZonesWords {
        add_header: "headerFooterZones.addHeader" = "Add header";
        add_footer: "headerFooterZones.addFooter" = "Add footer";
        header: "headerFooterZones.header" = "Header";
        footer: "headerFooterZones.footer" = "Footer";
        first_page: "headerFooterZones.firstPageHeader" = "First Page Header";
        even_page: "headerFooterZones.evenPageHeader" = "Even Page Header";
        odd_page: "headerFooterZones.oddPageHeader" = "Odd Page Header";
    }
}

/// One of the six sections of a page's header and footer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Zone {
    /// Whether the zone is in the footer band (true) or the header band (false).
    pub footer: bool,
    /// 0 start, 1 centre, 2 end of the band: left, centre and right in a left-to-right interface, mirrored in a
    /// right-to-left one.
    pub section: u8,
}

impl Zone {
    /// Section `section` (0 start, 1 centre, 2 end) of the header band, or of the footer band when `footer`.
    pub const fn new(footer: bool, section: u8) -> Self {
        Zone { footer, section }
    }
}

/// A section of one page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ZoneAt {
    /// Page index, 0-based.
    pub page: usize,
    /// The section on that page.
    pub zone: Zone,
}

impl ZoneAt {
    /// The section `zone` of page `page`.
    pub const fn new(page: usize, zone: Zone) -> Self {
        ZoneAt { page, zone }
    }

    fn part(self) -> u64 {
        (self.page as u64 * 2 + u64::from(self.zone.footer)) * 3 + u64::from(self.zone.section)
    }
}

/// Which header and footer a page uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HeaderFooterPage {
    /// The first page, with Different First Page.
    First,
    /// An odd page.
    Odd,
    /// An even page, with Different Odd & Even Pages.
    Even,
}

impl HeaderFooterPage {
    /// The kind for 1-based page `number` with Different First Page and Different Odd & Even.
    pub fn of(number: usize, different_first: bool, different_odd_even: bool) -> Self {
        if different_first && number == 1 {
            HeaderFooterPage::First
        } else if different_odd_even && number.is_multiple_of(2) {
            HeaderFooterPage::Even
        } else {
            HeaderFooterPage::Odd
        }
    }
}

/// Where the bands are on each page, page points: the header band from the top edge down to the top margin and the
/// footer band from the bottom margin to the bottom edge, inset left and right by `sides` (GridCraft's `hf_sides`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ZoneBands {
    /// Height of the header band, from the top edge to the top margin.
    pub top: f64,
    /// Height of the footer band, from the bottom margin to the bottom edge.
    pub bottom: f64,
    /// Left and right inset of the bands.
    pub sides: [f64; 2],
}

impl ZoneBands {
    /// Bands `top` and `bottom` high, inset by `sides`.
    pub const fn new(top: f64, bottom: f64, sides: [f64; 2]) -> Self {
        ZoneBands { top, bottom, sides }
    }

    /// The section `zone` of a page `size` points across and down: its rect in page points, before any inset. The
    /// band splits in thirds between the sides; section 0 is the third at the reading start.
    pub fn section(&self, size: Vec2, zone: Zone, rtl: bool) -> craft_behavior::kurbo::Rect {
        let (width, height) = (f64::from(size.x), f64::from(size.y));
        let (top, bottom) = if zone.footer { (height - self.bottom, height) } else { (0.0, self.top) };
        let third = (width - self.sides[0] - self.sides[1]).max(0.0) / 3.0;
        let column = if rtl { 2 - zone.section.min(2) } else { zone.section.min(2) };
        let left = self.sides[0] + third * f64::from(column);
        craft_behavior::kurbo::Rect::new(left, top, left + third, bottom)
    }
}

/// The text of each section as the app prints it.
pub struct ZoneText<'a>(Box<dyn Fn(ZoneAt) -> String + 'a>);

impl<'a> ZoneText<'a> {
    /// The text `text` gives for a section.
    pub fn new(text: impl Fn(ZoneAt) -> String + 'a) -> Self {
        ZoneText(Box::new(text))
    }

    fn of(&self, at: ZoneAt) -> String {
        (self.0)(at)
    }
}

/// The style of the zones.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct HeaderFooterZonesStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The space between a section and the band's top and bottom.
    inset: f32,
    hint: StyleColor,
    editing: StyleColor,
    neighbour: StyleColor,
    faint: StyleColor,
    dash: f32,
    gap: f32,
}

/// What the zones report.
#[derive(Debug)]
#[non_exhaustive]
pub struct HeaderFooterZonesOutput {
    /// The zones' response.
    pub response: Response,
    /// A section double clicked (or activated with the keyboard): start, or move, editing there.
    pub begin: Option<ZoneAt>,
    /// A press on a page outside the sections, or Escape on a focused section, while one was being edited: end it.
    pub end: bool,
    /// The screen rect of the section being edited, inset as the zones draw it, for the app's editor.
    pub editor: Option<Rect>,
}

impl Outcome for HeaderFooterZonesOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The zones over the pages on screen.
///
/// - A page's bands split their inner width in thirds; section 0 is the third at the reading start, so a right-to-left
///   interface mirrors them.
/// - A hovered or focused empty section shows the dashed hint outline and "Add header" or "Add footer"; the section
///   being edited ([`editing`](Self::editing)) is filled with the paper colour with the editing outline, its
///   neighbours in the band outlined (Excel).
/// - Text the app gives is drawn start, centre and end aligned in its section at the document's header size
///   ([`HEADER_TEXT`] page points) times the page scale, kept between [`HEADER_TEXT_SCREEN`] screen points.
/// - Keys: each section is a Tab stop named by what it holds, or "Add header"; Enter or Space starts editing it. Escape
///   on a focused section ends the editing. Show the zones after the page view's other canvas controls so they are
///   above the pages.
#[derive(Component, Props)]
#[component(canvas_control)]
pub struct HeaderFooterZones<'a> {
    #[common]
    common: Common,
    placed: &'a [PlacedPage],
    bands: &'a dyn Fn(usize) -> ZoneBands,
    /// Each section's text as it prints (fields expanded by the app).
    #[prop(func = "Fn(ZoneAt) -> String")]
    text: Option<ZoneText<'a>>,
    /// The section being edited, if any; it paints as an editor instead of a label.
    #[prop]
    editing: Option<ZoneAt>,
    /// The paper of the pages (document data).
    paper: DataColor,
    /// The ink of the pages' text (document data).
    ink: DataColor,
    begin: Option<ZoneAt>,
    end: bool,
}

impl<'a> HeaderFooterZones<'a> {
    /// The zones named `label` over `placed`, whose bands `bands` gives per page, on pages of the document's `paper`
    /// with text in its `ink`.
    pub fn new(label: impl Into<Label>, placed: &'a [PlacedPage], bands: &'a dyn Fn(usize) -> ZoneBands, paper: DataColor, ink: DataColor) -> Self {
        HeaderFooterZones { common: Common::new(), placed, bands, text: None, editing: None, paper, ink, begin: None, end: false }.label(label)
    }

    /// The whole section on `placed`, without the style's inset.
    fn section_rect(&self, placed: &PlacedPage, zone: Zone, rtl: bool) -> Rect {
        let size = Vec2::new(placed.rect.width() / placed.scale as f32, placed.rect.height() / placed.scale as f32);
        let section = (self.bands)(placed.page).section(size, zone, rtl);
        let corner = |x: f64, y: f64| placed.to_screen(Point::new(x, y));
        Rect::from_min_max(corner(section.x0, section.y0), corner(section.x1, section.y1))
    }

    /// Every section of the pages on screen, with its rect inset as the zones draw it.
    fn sections(&self, rtl: bool, inset: f32, visible: Rect) -> Vec<(ZoneAt, &'a PlacedPage, Rect)> {
        let mut out = Vec::new();
        for placed in self.placed.iter().filter(|placed| placed.rect.intersects(visible)) {
            for footer in [false, true] {
                for section in 0..3 {
                    let at = ZoneAt::new(placed.page, Zone::new(footer, section));
                    out.push((at, placed, self.section_rect(placed, at.zone, rtl).shrink2(Vec2::new(0.0, inset))));
                }
            }
        }
        out
    }

    fn name(&self, at: ZoneAt, words: &HeaderFooterZonesWords) -> String {
        let text = self.text.as_ref().map(|text| text.of(at)).unwrap_or_default();
        if !text.is_empty() {
            text
        } else if at.zone.footer {
            words.add_footer.as_str().to_owned()
        } else {
            words.add_header.as_str().to_owned()
        }
    }
}

impl Component for HeaderFooterZones<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS.with_chrome_key(false);
    const INPUT: Input = Input::Pointer;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = HeaderFooterZonesStyle;
    type Output = HeaderFooterZonesOutput;

    fn style(cx: &StyleCx<'_>, _: HeaderFooterZonesStyleKey) -> HeaderFooterZonesStyle {
        let (colors, stroke) = (&cx.tokens.color, &cx.tokens.stroke);
        HeaderFooterZonesStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            inset: cx.tokens.space.gap.tight,
            hint: cx.color(colors.line.control_edge),
            editing: cx.color(colors.accent.solid),
            neighbour: cx.color(colors.line.control_edge),
            faint: cx.color(colors.text.faint),
            dash: stroke.dash,
            gap: stroke.dash_gap,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// One focusable click part per section of the pages on screen.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let rtl = cx.flow.is_rtl();
        for (at, _, section) in self.sections(rtl, cx.style.inset, rect) {
            if self.editing == Some(at) {
                continue;
            }
            cx.part(ZONE.at(at.part()), section).sense(Sense::CLICK | Sense::TAKE_FOCUS).priority(HitPriority::new(HitLevel::Edge, 0)).focusable();
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let rtl = cx.flow.is_rtl();
        let sections = self.sections(rtl, cx.style.inset, cx.rect);
        for (at, _, _) in &sections {
            let Some(result) = cx.part_if_declared(ZONE.at(at.part())) else { continue };
            if result.press.double_pressed() || cx.part_activated(ZONE.at(at.part())).is_some() {
                self.begin = Some(*at);
            }
        }
        let focused = cx.active_part().is_some_and(|part| part.key == ZONE);
        let escaped = focused && cx.intents().take(Intent::Cancel);
        let elsewhere = canvas_space(&cx.env).is_some_and(|space| space.background().pressed);
        self.end = self.editing.is_some() && (escaped || elsewhere);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (rtl, style) = (cx.flow.is_rtl(), &cx.style);
        let draw = ui.draw();
        let words = cx.words::<HeaderFooterZonesWords>();
        let focused = cx.focus_visible().then(|| cx.active_part()).flatten().filter(|part| part.key == ZONE).and_then(|part| part.index);
        for (at, placed, rect) in self.sections(rtl, style.inset, cx.rect) {
            let outline = PaintShape::Rect(rect, CornerRadius::ZERO);
            if self.editing == Some(at) {
                draw.data_fill(&outline, self.paper);
                draw.stroke(&outline, StrokeWidth::Thin, style.editing);
                continue;
            }
            if self.editing.is_some_and(|editing| editing.page == at.page && editing.zone.footer == at.zone.footer) {
                draw.stroke(&outline, StrokeWidth::Thin, style.neighbour);
            }
            let text = self.text.as_ref().map(|text| text.of(at)).unwrap_or_default();
            if text.is_empty() {
                let shown = cx.part_if_declared(ZONE.at(at.part())).is_some_and(|part| part.hovered) || focused == Some(at.part());
                if shown {
                    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
                    draw.dashed(&corners, Dashes::new(style.dash, style.gap), StrokeWidth::Thin, style.hint);
                    let hint = if at.zone.footer { &words.add_footer } else { &words.add_header };
                    let run = TextRun::new(hint.as_str().to_owned()).color(style.faint).align(HAlign::Center);
                    draw.text(rect, &run);
                }
                continue;
            }
            let size = (HEADER_TEXT * placed.scale as f32).clamp(*HEADER_TEXT_SCREEN.start(), *HEADER_TEXT_SCREEN.end());
            let runs = [DocRun::new(text, FontRef::proportional(), size, self.ink)];
            let mut doc = DocText::new(&runs);
            doc.align = match at.zone.section {
                0 => HAlign::Start,
                1 => HAlign::Center,
                _ => HAlign::End,
            };
            let height = size * 1.2;
            let band = if at.zone.footer { Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - height), rect.max) } else { rect };
            draw.doc(band, &doc);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<HeaderFooterZonesWords>();
        for (at, _, _) in self.sections(cx.flow.is_rtl(), cx.style.inset, cx.rect) {
            if self.editing != Some(at) {
                access.node(ZONE.at(at.part()), Role::BUTTON).name = Name::Text(self.name(at, &words));
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> HeaderFooterZonesOutput {
        let rtl = cx.flow.is_rtl();
        let editor = self.editing.and_then(|at| {
            let placed = self.placed.iter().find(|placed| placed.page == at.page)?;
            Some(self.section_rect(placed, at.zone, rtl).shrink2(Vec2::new(0.0, cx.style.inset)))
        });
        HeaderFooterZonesOutput { response: cx.response.clone(), begin: self.begin, end: self.end, editor }
    }
}
