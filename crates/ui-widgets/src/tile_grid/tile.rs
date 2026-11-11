//! `Tile`: the item of a tile grid: a picture, a caption, corner badges, a badge bar with the rating, a flag and a
//! colour label, a check box, a file status and marks.
//!
//! Selection, the lead, hover, the focus ring, the drag ghost and the rename editor belong to the collection that
//! shows the tile; the tile draws the look the item step gives it and reports what its own parts do. The collection
//! sizes a tile as the picture it wants plus [`Tile::around_picture`]. The grid owns where the caption goes and
//! whether the card reserves a detail line, and applies both to its tiles.

use super::layout::Around;
use super::words::TileWords;
use super::{BadgeKind, BadgeSlot, BadgeVisibility, CaptionPlace, TileBadge, TileFlag, TileMark, TileStatus};
use crate::buttons::IconButton;
use crate::choice::{Checkbox, ChoiceWords, Rating};
use crate::contract::{
    Access, Bind, Common, Component, Cx, Focus as _, ItemOutput, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role,
    Styled as _, WordSetCx as _,
};
use crate::display::Badge;
use crate::list::{RowThumb, ThumbAspect, ThumbPaint, ToggleState};
use crate::primitives::{ContentPainterCx, IconSpec, PaintShape, StrokeWidth, TextRun, Wrap};
use crate::runtime::commands::CommandRef;
use crate::runtime::env::{Label, Units};
use crate::style::{ComponentStyle, Corners, DataColor, Flags, IconLook, StyleColor, StyleCx, SurfaceKind, SurfaceLook, TextLook, UiTokens};
use craft_behavior::{AccessAction, AccessData, Flow, IconId, Insets, Metrics, Mixed, NavShape, PartKey, PressPhase, Proposal, Roving, Sense};
use craft_keymap::Mods;
use craft_theme::variant::{Emphasis, Tone};
use craft_theme::{Rgba, Srgba, Stateful, States, TypeRole, Variant, over};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2};

/// Stars a rating has.
const STARS: u8 = 5;
/// The share of the scrim's opacity that veils a disabled tile.
const DISABLED_VEIL: f32 = 0.6;
/// The share of the scrim's opacity that veils a dimmed, rejected or hidden tile's picture.
const DIM_VEIL: f32 = 0.45;

/// What a tile's own parts report, besides what the collection reads from the pointer.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TilePartEvent {
    /// A click on the stars: the rating to set, 0 clears (a click on the star of the current rating).
    Rate(u8),
    /// A click on the check box, with the modifier keys held.
    #[non_exhaustive]
    Check {
        /// The modifiers held during the click.
        mods: Mods,
    },
    /// The pointer is over the picture: how far across it, from the reading start, 0 to 1.
    Scrub(f32),
    /// An activation of the corner action ([`Tile::corner_action`]).
    CornerAction,
}

/// What a tile returns.
#[derive(Debug)]
#[non_exhaustive]
pub struct TileOutput {
    /// The tile's response.
    pub response: Response,
    /// What the tile's parts reported this pass.
    pub event: Option<TilePartEvent>,
    /// Where the caption is drawn: the band under the picture, or the name's line in a start column; `None` for a
    /// tile that shows no caption.
    pub caption: Option<Rect>,
}

impl Outcome for TileOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl ItemOutput<TilePartEvent> for TileOutput {
    fn into_event(self) -> Option<TilePartEvent> {
        self.event
    }

    /// A tile without a caption has no label line: its rename field goes over the whole tile.
    fn label_rect(&self) -> Option<Rect> {
        self.caption
    }
}

const CHECK: PartKey = PartKey::new("check");
const RATING: PartKey = PartKey::new("rating");
const PILL: PartKey = PartKey::new("pill");
const MARK: PartKey = PartKey::new("mark");
const PICTURE: PartKey = PartKey::new("picture");
const CORNER: PartKey = PartKey::new("corner");

const CHECK_REASON: &str = "In a tile grid with checks, Space toggles the lead tile's check";
const RATING_REASON: &str = "In a tile grid with rating keys, the digits rate the selected tiles";

/// The sizes a tile takes from its control size and the tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sizes {
    card: bool,
    inset: f32,
    tight: f32,
    bar: f32,
    glyph: f32,
    band: f32,
    dot: f32,
    range_bar: f32,
    face: f32,
    name_line: f32,
    detail_line: f32,
    start: f32,
    /// The picture width of a tile measured on its own: least, and ideal (a grid's card thumbnail at zoom 1).
    thumbs: [f32; 2],
    around: AroundPicture,
}

/// What a tile adds around its picture for each caption arrangement, from the sizes: the grid sizes its cards with it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct AroundPicture {
    below: Around,
    below_detail: Around,
    start: Around,
    bare: Around,
}

impl AroundPicture {
    fn of(sizes: &Sizes) -> AroundPicture {
        let around = |place: CaptionPlace, detail: bool| {
            Around::new(sizes.inset + sizes.column(place), sizes.inset, sizes.inset, sizes.inset + sizes.caption(place, detail))
        };
        AroundPicture {
            below: around(CaptionPlace::Below, false),
            below_detail: around(CaptionPlace::Below, true),
            start: around(CaptionPlace::Start, false),
            bare: around(CaptionPlace::None, false),
        }
    }

    /// What a tile with its caption in `place` adds, with a detail line reserved or not.
    pub(super) fn get(&self, place: CaptionPlace, detail: bool) -> Around {
        match (place, detail) {
            (CaptionPlace::Below, false) => self.below,
            (CaptionPlace::Below, true) => self.below_detail,
            (CaptionPlace::Start, _) => self.start,
            _ => self.bare,
        }
    }
}

impl Sizes {
    fn of(cx: &StyleCx<'_>, card: bool) -> Sizes {
        let (tokens, control) = (cx.tokens, cx.control());
        let line = tokens.typography.get(TypeRole::Caption).line_height;
        let name_line = if card { tokens.typography.get(TypeRole::BodyStrong).line_height } else { line };
        let sizes = Sizes {
            card,
            inset: if card { tokens.space.pad.control } else { tokens.space.gap.inline },
            tight: tokens.space.gap.tight,
            bar: control.height,
            glyph: control.icon,
            band: tokens.size.icon.md,
            dot: tokens.size.handle.md,
            range_bar: tokens.size.handle.sm,
            face: tokens.size.icon.lg,
            name_line,
            detail_line: line,
            start: control.height,
            thumbs: [tokens.size.thumb.sm, tokens.size.thumb.card],
            around: AroundPicture::default(),
        };
        Sizes { around: AroundPicture::of(&sizes), ..sizes }
    }

    /// The height of the caption band under the picture.
    fn caption(&self, place: CaptionPlace, detail: bool) -> f32 {
        match place {
            CaptionPlace::Below => {
                let gaps = if self.card { self.inset } else { 2.0 * self.tight };
                gaps + self.name_line + if detail { self.detail_line } else { 0.0 }
            }
            _ => 0.0,
        }
    }

    fn column(&self, place: CaptionPlace) -> f32 {
        if place == CaptionPlace::Start { self.start } else { 0.0 }
    }
}

/// The look of a tile.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TileStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    /// The name.
    #[style(text)]
    name: TextLook,
    /// The name of an offline tile.
    #[style(text)]
    name_failed: TextLook,
    /// The detail line.
    #[style(text)]
    detail: TextLook,
    /// The text of the offline band.
    #[style(text)]
    band_text: TextLook,
    /// The glyph of the offline band.
    #[style(icon)]
    band_icon: IconLook,
    /// A mark under a start caption.
    #[style(icon)]
    mark: IconLook,
    /// The stars of the badge bar, which the rating face reads as its host ink.
    #[style(icon)]
    stars: IconLook,
    /// The pick flag.
    #[style(icon)]
    picked: IconLook,
    /// The reject flag.
    #[style(icon)]
    rejected: IconLook,
    /// The glyph in the picture area of a folder tile or a tile with an icon face.
    #[style(icon)]
    face: IconLook,
    /// The glyph of a glyph badge.
    #[style(icon)]
    pill_icon: IconLook,
    placeholder: StyleColor,
    dim: StyleColor,
    veil: Stateful<StyleColor>,
    /// The fill under the badge bar and the check box, over the picture.
    plate: StyleColor,
    pill: StyleColor,
    band: StyleColor,
    range: StyleColor,
    scrub: StyleColor,
    dot_edge: StyleColor,
    pill_radius: CornerRadius,
    plate_radius: CornerRadius,
    sizes: Sizes,
}

/// A text badge's variant: a solid fill in its tone.
fn pill_variant(tone: Tone) -> Variant {
    Variant::look(Emphasis::Solid, tone)
}

/// A corner badge's width and the badge cell's height, from the measured faces.
#[derive(Clone, Copy, Debug)]
struct Faces {
    check: Vec2,
    stars: Vec2,
    pill: f32,
    corner: Vec2,
}

/// Where the parts of a tile lie in its content rect.
struct Placed {
    picture: Rect,
    caption: Rect,
    name: Rect,
    detail: Option<Rect>,
    marks: Vec<Rect>,
    band: Option<Rect>,
    bar: Option<Rect>,
    stars: Option<Rect>,
    flag: Option<Rect>,
    dot: Option<Rect>,
    check: Option<(Rect, Rect)>,
    pills: Vec<Rect>,
    corner: Option<Rect>,
}

/// One tile of a tile grid.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Tile<'a> {
    #[common]
    common: Common,
    /// Where the caption goes: the grid's, applied to its tiles.
    #[prop]
    caption: CaptionPlace,
    /// The card has a line for the detail, whether this tile has one or not: the grid's, applied to its tiles.
    #[prop]
    detail_line: bool,
    /// The tile is a framed card: the picture inset in a raised surface, the name strong and the detail under it, both
    /// at the reading start. The grid's, applied to its tiles.
    #[style_key]
    #[prop(flag)]
    card: bool,
    /// A second line under the name.
    #[prop(into)]
    detail: Option<Label>,
    /// What a screen reader reads as the tile's name, in place of the name with its details; the marks' tips still
    /// join it.
    #[prop(into)]
    accessible_name: Option<String>,
    /// The stars, 0 to 5; a tile without a rating has no stars.
    #[prop]
    rating: Option<u8>,
    /// A pick or reject flag.
    #[prop]
    flag: Option<TileFlag>,
    /// The colour label's dot.
    #[prop]
    label_color: Option<Srgba>,
    /// The app's name for the colour label, read by a screen reader.
    #[prop(into)]
    label_name: Option<Label>,
    /// Offline, unreadable or failed.
    #[prop]
    status: TileStatus,
    /// In and Out as fractions of the picture's width, drawn as a bar along its bottom.
    #[prop]
    range: Option<(f32, f32)>,
    /// The scrub line's fraction across the picture from the reading start.
    #[prop(quantity = "fraction")]
    scrub: Option<f32>,
    /// When the badge bar shows.
    #[prop]
    bar: BadgeVisibility,
    /// The size of the stack this tile stands for: a "×N" badge in the end-top corner.
    #[prop]
    stack: Option<usize>,
    /// The picture under a veil, still selectable.
    #[prop(flag)]
    dimmed: bool,
    /// Rejected: the picture under a veil.
    #[prop(flag)]
    rejected: bool,
    /// Hidden from the output: the picture under a veil and, with a start caption, a slash across the name.
    #[prop(flag)]
    hidden: bool,
    /// A folder: the placeholder with the folder glyph instead of a picture.
    #[prop(flag)]
    folder: bool,
    /// The rename editor replaces the name: the row's state, applied by the grid.
    #[prop]
    renaming: bool,
    /// The check box, in check mode; `None` outside it.
    #[prop]
    check: Option<ToggleState>,
    aspect: ThumbAspect,
    first: Option<ThumbPaint<'a>>,
    icon: Option<IconSpec<'static>>,
    badges: Vec<TileBadge>,
    marks: Vec<TileMark>,
    pills: Vec<TileBadge>,
    corner: Option<(IconId, CommandRef)>,
    placed: Option<Placed>,
    event: Option<TilePartEvent>,
    caption_rect: Option<Rect>,
}

impl<'a> Tile<'a> {
    /// A tile named `label`, which is its caption and its accessible name.
    pub fn new(label: impl Into<Label>) -> Self {
        Tile {
            common: Common::new(),
            caption: CaptionPlace::Below,
            detail_line: false,
            card: false,
            detail: None,
            accessible_name: None,
            rating: None,
            flag: None,
            label_color: None,
            label_name: None,
            status: TileStatus::Normal,
            range: None,
            scrub: None,
            bar: BadgeVisibility::Auto,
            stack: None,
            dimmed: false,
            rejected: false,
            hidden: false,
            folder: false,
            icon: None,
            renaming: false,
            check: None,
            aspect: ThumbAspect::SQUARE,
            first: None,
            badges: Vec::new(),
            marks: Vec::new(),
            pills: Vec::new(),
            corner: None,
            placed: None,
            event: None,
            caption_rect: None,
        }
        .label(label)
    }

    /// A picture `aspect` times as wide as high; `paint` draws into the picture's rect through the content painter,
    /// which is clipped to it. Without one the tile shows the placeholder.
    #[must_use]
    pub fn thumb(mut self, aspect: ThumbAspect, paint: impl FnOnce(&ContentPainterCx, Rect) + 'a) -> Self {
        self.aspect = aspect;
        self.first = Some(Box::new(paint));
        self.icon = None;
        self
    }

    /// The picture area shows `icon` centred on the placeholder, in the style's face size and ink, in place of a
    /// picture. The last of `thumb` and `icon` called wins.
    #[must_use]
    pub fn icon(mut self, icon: impl Into<IconSpec<'static>>) -> Self {
        self.icon = Some(icon.into());
        self.first = None;
        self
    }

    /// A corner badge.
    #[must_use]
    pub fn badge(mut self, badge: TileBadge) -> Self {
        self.badges.push(badge);
        self
    }

    /// A glyph under a start caption.
    #[must_use]
    pub fn mark(mut self, mark: TileMark) -> Self {
        self.marks.push(mark);
        self
    }

    /// A button in the picture's end-top corner that shows while the pointer is over the tile or the tile has the
    /// keyboard focus, and is reached by the keys as a part of the tile. Its activation is
    /// [`TilePartEvent::CornerAction`]; `command` gives its label, tooltip and enabled state, and the tile does not
    /// fire it.
    #[must_use]
    pub fn corner_action(mut self, icon: IconId, command: impl Into<CommandRef>) -> Self {
        self.corner = Some((icon, command.into()));
        self
    }

    /// What a tile adds around its picture: its padding, the caption band under the picture or the column at its
    /// reading start. A tile grid sizes a tile as the picture plus this.
    pub fn around_picture(cx: &StyleCx<'_>, place: CaptionPlace, detail: bool) -> Around {
        Self::around(cx, false).get(place, detail)
    }

    /// What a tile adds around its picture for every caption arrangement.
    pub(super) fn around(cx: &StyleCx<'_>, card: bool) -> AroundPicture {
        Sizes::of(cx, card).around
    }

    /// Whether the caption band holds a detail line.
    fn has_detail_line(&self) -> bool {
        self.detail_line || self.detail.is_some()
    }

    fn has_bar_content(&self) -> bool {
        self.rating.is_some() || self.flag.is_some() || self.label_color.is_some()
    }

    fn has_bar_values(&self) -> bool {
        self.rating.is_some_and(|rating| rating > 0) || self.flag.is_some() || self.label_color.is_some()
    }

    /// The corner badges: the app's, the stack badge and the file status badge.
    fn corner_badges(&self, words: &TileWords, units: &Units) -> Vec<TileBadge> {
        let mut all = self.badges.clone();
        if let Some(n) = self.stack {
            all.push(TileBadge::new(BadgeSlot::EndTop, BadgeKind::Text(Label::from(units.times(n as f64))), Tone::Neutral, None));
        }
        let failure = |text: &Label| TileBadge::new(BadgeSlot::StartBottom, BadgeKind::Text(text.clone()), Tone::Bad, None);
        match &self.status {
            TileStatus::Unreadable => all.push(failure(&words.unreadable)),
            TileStatus::Failed => all.push(failure(&words.failed)),
            _ => {}
        }
        all
    }

    fn badge_face(badge: &TileBadge) -> Option<Badge> {
        match &badge.kind {
            BadgeKind::Text(text) => Some(Badge::new(text.clone()).variant(pill_variant(badge.tone))),
            BadgeKind::Glyph(_) => None,
        }
    }

    fn place(&self, sizes: &Sizes, area: Rect, rtl: bool, bar_shown: bool, faces: &Faces, pill_widths: &[f32]) -> Placed {
        let flow = Flow::new(area, rtl);
        let column = sizes.column(self.caption);
        let band_height = sizes.caption(self.caption, self.has_detail_line());
        let (start, rest) = flow.take_start(column);
        let picture = Rect::from_min_max(rest.rect().min, rest.rect().max - Vec2::new(0.0, band_height));
        let band = Rect::from_min_max(egui::pos2(rest.rect().min.x, picture.bottom()), rest.rect().max);
        let (caption, name, detail, marks) = match self.caption {
            CaptionPlace::Below => {
                let above = if sizes.card { sizes.inset } else { sizes.tight };
                let name = Rect::from_min_size(egui::pos2(band.left(), band.top() + above), Vec2::new(band.width(), sizes.name_line));
                let detail = self.detail.as_ref().map(|_| Rect::from_min_size(name.left_bottom(), Vec2::new(band.width(), sizes.detail_line)));
                (band, name, detail, Vec::new())
            }
            CaptionPlace::Start => {
                let name = Rect::from_min_size(egui::pos2(start.left(), start.top() + sizes.tight), Vec2::new(start.width(), sizes.name_line));
                let first = name.bottom() + sizes.tight;
                let marks = (0..self.marks.len())
                    .map(|at| {
                        let top = first + at as f32 * (sizes.glyph + sizes.tight);
                        Rect::from_center_size(egui::pos2(start.center().x, top + sizes.glyph / 2.0), Vec2::splat(sizes.glyph))
                    })
                    .collect();
                (start, name, None, marks)
            }
            CaptionPlace::None => (Rect::NOTHING, Rect::NOTHING, None, Vec::new()),
        };
        let pictures = Flow::new(picture, rtl);
        let mut floor = picture.bottom();
        let band_rect = matches!(self.status, TileStatus::Offline(_)).then(|| {
            let rect = Rect::from_min_max(egui::pos2(picture.left(), floor - sizes.band), picture.right_bottom());
            floor = rect.top();
            rect
        });
        let (mut bar, mut stars, mut flag, mut dot) = (None, None, None, None);
        if bar_shown {
            let rect = Rect::from_min_max(egui::pos2(picture.left(), floor - sizes.bar), egui::pos2(picture.right(), floor));
            // The stars and the flag run from the start, the dot sits at the end, each with an inset after it. A part
            // that does not fit in what the earlier ones left is left out, and so is every part after it.
            let mut room = Flow::new(rect, rtl).take_start(sizes.inset).1;
            let mut fitting = true;
            let mut take = |width: f32, at_end: bool| {
                fitting &= width + sizes.inset <= room.rect().width();
                fitting.then(|| {
                    if at_end {
                        let (taken, rest) = room.take_end(width + sizes.inset);
                        room = rest;
                        Rect::from_center_size(taken.center(), Vec2::new(width, taken.height()))
                    } else {
                        let (taken, rest) = room.take_start(width);
                        room = rest.take_start(sizes.inset).1;
                        taken
                    }
                })
            };
            stars = self.rating.and_then(|_| take(faces.stars.x, false));
            flag = self.flag.and_then(|_| take(sizes.glyph, false)).map(|cell| Rect::from_center_size(cell.center(), Vec2::splat(sizes.glyph)));
            dot = self.label_color.and_then(|_| take(sizes.dot, true)).map(|cell| Rect::from_center_size(cell.center(), Vec2::splat(sizes.dot)));
            floor = rect.top();
            bar = Some(rect);
        }
        let margin = sizes.inset + sizes.tight;
        let check = self.check.map(|_| {
            let across = pictures.span(margin, faces.check.x);
            let at = Rect::from_min_size(egui::pos2(across.min.x, picture.top() + margin), faces.check);
            (at, at.expand(sizes.tight))
        });
        let corner = self.corner.as_ref().map(|_| {
            let across = Flow::new(picture, rtl).span((picture.width() - margin - faces.corner.x).max(0.0), faces.corner.x);
            Rect::from_min_size(egui::pos2(across.min.x, picture.top() + margin), faces.corner)
        });
        let corners = Rect::from_min_max(picture.min, egui::pos2(picture.right(), floor));
        let mut pills = vec![Rect::NOTHING; self.pills.len()];
        for slot in [BadgeSlot::StartTop, BadgeSlot::EndTop, BadgeSlot::StartBottom, BadgeSlot::EndBottom] {
            let (is_start, top) = match slot {
                BadgeSlot::StartTop => (true, true),
                BadgeSlot::EndTop => (false, true),
                BadgeSlot::StartBottom => (true, false),
                _ => (false, false),
            };
            let members: Vec<usize> = (0..self.pills.len()).filter(|&at| self.pills[at].slot == slot).collect();
            let widths: Vec<f32> = members.iter().map(|&at| pill_widths[at]).collect();
            let plate = if is_start { check.map(|(_, plate)| plate) } else { corner.map(|at| at.expand(sizes.tight)) };
            let rects = badge_rects(Flow::new(corners, rtl), &widths, faces.pill, sizes, plate, (is_start, top));
            for (at, rect) in members.into_iter().zip(rects) {
                pills[at] = rect;
            }
        }
        Placed { picture, caption, name, detail, marks, band: band_rect, bar, stars, flag, dot, check, pills, corner }
    }
}

/// The rects of the badges of one corner, inward from it. Badges stack toward the middle of the picture `tight`
/// apart; one that would touch the plate of the check box or the corner button moves past it, away from the corner.
fn badge_rects(flow: Flow, widths: &[f32], height: f32, sizes: &Sizes, plate: Option<Rect>, corner: (bool, bool)) -> Vec<Rect> {
    let (is_start, top) = corner;
    let area = flow.rect();
    let mut y = if top { area.top() + sizes.inset } else { area.bottom() - sizes.inset - height };
    widths
        .iter()
        .map(|&width| {
            let across = if is_start { flow.span(sizes.inset, width) } else { flow.span((area.width() - sizes.inset - width).max(0.0), width) };
            let mut rect = Rect::from_min_size(Pos2::new(across.min.x, y), Vec2::new(width, height));
            if let Some(plate) = plate.filter(|plate| rect.intersects(*plate)) {
                let moved = if top { plate.bottom() + sizes.tight } else { plate.top() - sizes.tight - height };
                rect = rect.translate(Vec2::new(0.0, moved - rect.top()));
            }
            y = if top { rect.bottom() + sizes.tight } else { rect.top() - sizes.tight - height };
            rect
        })
        .collect()
}

impl Component for Tile<'_> {
    const ROLE: Role = Role::LIST_ITEM;
    const OVERLAYS: bool = false;
    type Style = TileStyle;
    type Output = TileOutput;

    fn style(cx: &StyleCx<'_>, key: TileStyleKey) -> TileStyle {
        let tokens = cx.tokens;
        let colors = &tokens.color;
        let (state, text) = (&colors.state, &colors.text);
        let clear = Rgba::TRANSPARENT;
        let c = |rgba: Rgba| cx.color(rgba);
        let sizes = Sizes::of(cx, key.card);
        let (rest, hover, pressed) = if key.card {
            let raised = colors.surface.raised;
            (c(raised), c(over(state.hover, raised)), c(over(state.pressed, raised)))
        } else {
            (c(clear), c(state.hover), c(state.pressed))
        };
        let fill = Stateful::new(rest).off(States::new(rest, hover, pressed, rest)).on(States::new(
            c(state.selected),
            c(state.selected_hover),
            c(state.selected_hover),
            c(state.selected),
        ));
        let line = c(clear);
        let edge = Stateful::new(line)
            .off(States::new(line, c(text.muted), c(text.muted), line))
            .on(States::new(c(colors.accent.solid), c(colors.accent.solid), c(colors.accent.solid), c(colors.accent.solid)))
            .when(Flags::CURRENT, c(colors.accent.solid));
        let radius = UiTokens::corner(tokens.shape.radius.control);
        let surface = SurfaceLook::new(fill).radius(radius).border(edge, tokens.stroke.thick);
        let surface = if key.card { surface.kind(SurfaceKind::Raised) } else { surface };
        let on_selected = c(text.on_selected);
        let ink = |base: StyleColor| {
            Stateful::new(base).off(States::new(base, base, base, c(text.disabled))).on(States::new(
                on_selected,
                on_selected,
                on_selected,
                on_selected,
            ))
        };
        let fixed = |color: Rgba| Stateful::new(c(color));
        let pill = cx.with_variant(pill_variant(Tone::Neutral)).variant_look();
        let plate = c(colors.surface.scrim);
        let on_bar = |ink: StyleColor, side: f32| IconLook::new(side, Stateful::new(ink)).on(Stateful::new(plate));
        let veil = Stateful::new(c(clear)).off(States::new(c(clear), c(clear), c(clear), c(colors.surface.scrim.faded(DISABLED_VEIL))));
        TileStyle {
            surface,
            metrics: Metrics::new().padding(Insets::new(sizes.inset, sizes.inset, sizes.inset, sizes.inset)).gap(sizes.tight),
            name: if key.card {
                TextLook::new(TypeRole::BodyStrong, ink(c(text.normal)))
            } else {
                TextLook::new(TypeRole::Caption, ink(c(text.muted)))
            },
            name_failed: TextLook::new(if key.card { TypeRole::BodyStrong } else { TypeRole::Caption }, ink(cx.tone(Tone::Bad))),
            detail: TextLook::new(TypeRole::Caption, ink(c(if key.card { text.muted } else { text.faint }))),
            band_text: TextLook::new(TypeRole::Caption, fixed(text.on_status)).on(fixed(colors.status.bad.solid)),
            band_icon: IconLook::new(sizes.glyph, fixed(text.on_status)).on(fixed(colors.status.bad.solid)),
            mark: IconLook::new(sizes.glyph, ink(c(text.faint))),
            stars: on_bar(c(text.on_strong), tokens.size.icon.glyph_sm),
            picked: on_bar(cx.tone(Tone::Good), sizes.glyph),
            rejected: on_bar(cx.tone(Tone::Bad), sizes.glyph),
            face: IconLook::new(
                sizes.face,
                Stateful::new(c(text.muted)).off(States::new(c(text.muted), c(text.muted), c(text.muted), c(text.disabled))),
            )
            .on(fixed(colors.surface.sunken)),
            pill_icon: IconLook::new(sizes.glyph, pill.text.clone()).on(pill.fill.clone()),
            placeholder: c(colors.surface.sunken),
            dim: c(colors.surface.scrim.faded(DIM_VEIL)),
            veil,
            plate,
            pill: pill.fill.off.rest,
            band: c(colors.status.bad.solid),
            range: c(colors.accent.solid),
            scrub: c(colors.accent.solid),
            dot_edge: c(colors.line.mark),
            pill_radius: UiTokens::corner(tokens.shape.radius.pill),
            plate_radius: UiTokens::corner(tokens.shape.radius.sm),
            sizes,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let sizes = &cx.style.sizes;
        let ratio = self.aspect.of(1.0);
        let caption = sizes.caption(self.caption, self.has_detail_line());
        let column = sizes.column(self.caption);
        // The picture is a card of a grid at zoom 1: as wide as the thumbnail side, with what the density adds around
        // it. A narrower room shrinks the picture to fit.
        let side = if cx.proposal == Proposal::Min { sizes.thumbs[0] } else { sizes.thumbs[1] };
        let width = cx.room.map_or(side + column, |room| room.min(side + column));
        Vec2::new(width, ((width - column).max(0.0) / ratio) + caption)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn parts(&mut self, _rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let sizes = cx.style.sizes;
        let rtl = cx.flow.is_rtl();
        let words = cx.words::<TileWords>();
        self.pills = self.corner_badges(&words, &cx.env.units());
        let faces = Faces {
            check: cx.face(Checkbox::new(Bind::given(Mixed::Same(false)))),
            stars: cx.face(Rating::new(Bind::given(0u8), STARS).compact(true)),
            pill: cx.face(Badge::new("0")).y,
            corner: self.corner.as_ref().map_or(Vec2::ZERO, |(icon, command)| cx.face(IconButton::command(icon.clone(), command.clone()))),
        };
        let pill_widths: Vec<f32> = self.pills.iter().map(|badge| Self::badge_face(badge).map_or(faces.pill, |face| cx.face(face).x)).collect();
        let bar_shown = self.has_bar_content()
            && match self.bar {
                BadgeVisibility::Always => true,
                BadgeVisibility::Never => false,
                _ => cx.hovered() || self.has_bar_values(),
            };
        let mut placed = self.place(&sizes, cx.content_rect, rtl, bar_shown, &faces, &pill_widths);
        let _ = cx.part(PICTURE, placed.picture).sense(Sense::HOVER);
        if let (Some(state), Some((at, _))) = (self.check, &placed.check) {
            let mut flags = Flags::empty();
            flags.set(Flags::CHECKED, state == ToggleState::On);
            flags.set(Flags::MIXED, state == ToggleState::Mixed);
            cx.part(CHECK, *at).sense(Sense::CLICK).flags(flags).pointer_only(CHECK_REASON);
        }
        if let Some(stars) = placed.stars {
            // A click rates by where it lands; AccessKit rates with Increment, Decrement and SetValue instead.
            cx.part(RATING, stars).sense(Sense::CLICK).access_click(false).pointer_only(RATING_REASON);
        }
        for (at, (badge, rect)) in self.pills.iter().zip(&placed.pills).enumerate() {
            let part = cx.part(PILL.at(at as u64), *rect);
            if let Some(tip) = &badge.tip {
                let _ = part.tooltip(tip.clone());
            }
        }
        for (at, (mark, rect)) in self.marks.iter().zip(&placed.marks).enumerate() {
            let _ = cx.part(MARK.at(at as u64), *rect).tooltip(mark.tip.clone());
        }
        if let Some(at) = placed.corner.filter(|_| cx.hovered() || cx.focus_visible()) {
            cx.part(CORNER, at).sense(Sense::CLICK | Sense::FOCUS).focusable();
            cx.roving(Roving::new(NavShape::Flat));
        } else {
            placed.corner = None;
        }
        self.placed = Some(placed);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let rtl = cx.flow.is_rtl();
        if let (Some(pos), Some(result)) = (cx.hover_pos(PICTURE), cx.part_if_declared(PICTURE)) {
            let rect = result.rect;
            let along = if rtl { rect.right() - pos.x } else { pos.x - rect.left() };
            self.event = Some(TilePartEvent::Scrub((along / rect.width().max(f32::EPSILON)).clamp(0.0, 1.0)));
        }
        if let Some(result) = cx.part_if_declared(RATING)
            && let PressPhase::Release { pos, click: true, .. } = result.press.phase
        {
            let rect = result.rect;
            let along = if rtl { rect.right() - pos.x } else { pos.x - rect.left() };
            let star = (along / (rect.width() / f32::from(STARS))).floor().clamp(0.0, f32::from(STARS - 1)) as u8 + 1;
            let current = self.rating.unwrap_or(0);
            self.event = Some(TilePartEvent::Rate(if star == current { 0 } else { star }));
        }
        if cx.part_activated(CHECK).is_some() {
            self.event = Some(TilePartEvent::Check { mods: cx.part(CHECK).press.mods });
        }
        if cx.part_activated(CORNER).is_some() {
            self.event = Some(TilePartEvent::CornerAction);
        }
        let current = self.rating.unwrap_or(0);
        while let Some(request) = cx.intents().take_access() {
            if request.target != Some(RATING.into()) {
                continue;
            }
            let rated = match (request.action, request.data) {
                (AccessAction::Increment, _) => current.saturating_add(1),
                (AccessAction::Decrement, _) => current.saturating_sub(1),
                (AccessAction::SetValue, Some(AccessData::NumericValue(value))) => value.round().max(0.0) as u8,
                _ => continue,
            };
            self.event = Some(TilePartEvent::Rate(rated.min(STARS)));
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<TileWords>();
        let label = cx.resolved.label;
        access.name(Name::Text(self.full_name(cx, &words)));
        if self.check.is_some() {
            access.node(CHECK, Role::CHECK_BOX).name = Name::Text(words.select.fill(&[("name", label)]));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(placed) = self.placed.take() else { return };
        let style = cx.style;
        let picture = placed.picture;
        let flat = CornerRadius::ZERO;
        if style.sizes.card {
            ui.draw().surface(picture, SurfaceKind::Sunken, Corners::All);
        } else {
            ui.draw().fill(&PaintShape::Rect(picture, flat), style.placeholder);
        }
        if let Some(paint) = self.first.take() {
            let _ = ui.child("picture", picture, RowThumb::new(paint));
        }
        {
            let draw = ui.draw();
            let face = self.icon.take().or_else(|| self.folder.then(|| IconSpec::svg(IconId::new("folder"))));
            if let Some(face) = face {
                let icon = Rect::from_center_size(picture.center(), Vec2::splat(style.face.side));
                draw.icon(icon, &face.color(cx.resolve(&style.face.color)));
            }
            if self.dimmed || self.rejected || self.hidden {
                draw.fill(&PaintShape::Rect(picture, flat), style.dim);
            }
            draw.fill(&PaintShape::Rect(picture, flat), cx.resolve(&style.veil));
            self.paint_range_and_scrub(&draw, cx, picture);
        }
        self.paint_band(ui, cx, &placed);
        self.paint_bar(ui, cx, &placed);
        self.paint_pills(ui, cx, &placed);
        if let (Some((icon, command)), Some(_)) = (&self.corner, placed.corner) {
            ui.face(CORNER, IconButton::command(icon.clone(), command.clone()));
        }
        if let (Some(state), Some((_, plate))) = (self.check, placed.check) {
            ui.draw().fill(&PaintShape::Rect(plate, style.plate_radius), style.plate);
            let value = match state {
                ToggleState::Mixed => Mixed::Mixed,
                state => Mixed::Same(state == ToggleState::On),
            };
            ui.face(CHECK, Checkbox::new(Bind::given(value)));
        }
        self.paint_caption(ui, cx, &placed);
        self.caption_rect = match self.caption {
            CaptionPlace::Below => Some(placed.caption),
            CaptionPlace::Start => Some(placed.name),
            CaptionPlace::None => None,
        };
    }

    fn finish(self, cx: Cx<'_, Self>) -> TileOutput {
        TileOutput { response: cx.response.clone(), event: self.event, caption: self.caption_rect }
    }
}

impl Tile<'_> {
    fn full_name(&self, cx: &Cx<'_, Self>, words: &TileWords) -> String {
        let marks = || self.marks.iter().map(|mark| mark.tip.as_str().to_owned()).filter(|tip| !tip.is_empty());
        let joined = |name: &str, details: &[String]| {
            if details.is_empty() { name.to_owned() } else { words.tile_name.fill(&[("name", name), ("details", &details.join(", "))]) }
        };
        if let Some(name) = &self.accessible_name {
            return joined(name, &marks().collect::<Vec<_>>());
        }
        let mut details: Vec<String> = Vec::new();
        details.extend(self.detail.iter().map(|detail| detail.as_str().to_owned()));
        if let Some(n) = self.rating.filter(|n| *n > 0) {
            let stars = cx.words::<ChoiceWords>();
            details.push(stars.stars.text(u64::from(n), &cx.format().integer(u64::from(n))));
        }
        match self.flag {
            Some(TileFlag::Picked) => details.push(words.picked.as_str().to_owned()),
            Some(TileFlag::Rejected) => details.push(words.rejected.as_str().to_owned()),
            None if self.rejected => details.push(words.rejected.as_str().to_owned()),
            None => {}
        }
        if let (Some(name), Some(_)) = (&self.label_name, self.label_color) {
            details.push(words.label.fill(&[("label", name.as_str())]));
        }
        match &self.status {
            TileStatus::Offline(Some(text)) => {
                details.push(words.offline_with.fill(&[("offline", words.offline.as_str()), ("message", text.as_str())]))
            }
            TileStatus::Offline(None) => details.push(words.offline.as_str().to_owned()),
            TileStatus::Unreadable => details.push(words.unreadable.as_str().to_owned()),
            TileStatus::Failed => details.push(words.failed.as_str().to_owned()),
            _ => {}
        }
        if let Some(n) = self.stack {
            details.push(cx.env.units().times(n as f64));
        }
        details.extend(self.badges.iter().filter_map(|badge| badge.tip.as_ref()).map(|tip| tip.as_str().to_owned()).filter(|tip| !tip.is_empty()));
        details.extend(marks());
        joined(cx.resolved.label, &details)
    }

    /// The in and out bar along the picture's bottom and the scrub line.
    fn paint_range_and_scrub(&self, draw: &crate::primitives::Draw<'_>, cx: &Cx<'_, Self>, picture: Rect) {
        let flow = Flow::new(picture, cx.flow.is_rtl());
        if let Some((from, to)) = self.range {
            let (low, high) = (from.clamp(0.0, 1.0).min(to.clamp(0.0, 1.0)), from.clamp(0.0, 1.0).max(to.clamp(0.0, 1.0)));
            let span = flow.span(low * picture.width(), (high - low) * picture.width());
            let bar = Rect::from_x_y_ranges(span.x_range(), (picture.bottom() - cx.style.sizes.range_bar)..=picture.bottom());
            draw.fill(&PaintShape::Rect(bar, CornerRadius::ZERO), cx.style.range);
        }
        if let Some(fraction) = self.scrub {
            let x = flow.span(fraction.clamp(0.0, 1.0) * picture.width(), 0.0).min.x;
            draw.stroke(&PaintShape::Path(vec![Pos2::new(x, picture.top()), Pos2::new(x, picture.bottom())]), StrokeWidth::Thin, cx.style.scrub);
        }
    }

    /// The offline band: the glyph and the text, over a failure fill.
    fn paint_band(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        let (Some(band), TileStatus::Offline(text)) = (placed.band, &self.status) else { return };
        let (style, words) = (cx.style, cx.words::<TileWords>());
        let draw = ui.draw();
        draw.fill(&PaintShape::Rect(band, CornerRadius::ZERO), style.band);
        let (glyph, rest) = Flow::new(band, cx.flow.is_rtl()).take_start(style.sizes.inset + style.band_icon.side);
        let icon = Rect::from_center_size(glyph.center(), Vec2::splat(style.band_icon.side));
        draw.icon(icon, &IconSpec::svg(IconId::new("unlink")).color(cx.resolve(&style.band_icon.color)));
        let message = text.as_ref().unwrap_or(&words.offline);
        let run = TextRun::new(message.as_str()).role(style.band_text.role).color(cx.resolve(&style.band_text.color)).elide();
        draw.text(rest.take_start(style.sizes.inset).1.rect(), &run);
    }

    /// The badge bar: the stars, the flag and the colour dot over a plate laid on the picture's bottom.
    fn paint_bar(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        let Some(bar) = placed.bar else { return };
        let style = cx.style;
        ui.draw().fill(&PaintShape::Rect(bar, CornerRadius::ZERO), style.plate);
        if let (Some(rating), Some(_)) = (self.rating, placed.stars) {
            let words = cx.words::<TileWords>();
            let face = Rating::new(Bind::given(rating), STARS).compact(true).label(words.rating.fill(&[("name", cx.resolved.label)]));
            ui.face_in(RATING, face, &style.stars.color);
        }
        let draw = ui.draw();
        if let (Some(flag), Some(rect)) = (self.flag, placed.flag) {
            let (id, look) = match flag {
                TileFlag::Picked => ("flag", &style.picked),
                TileFlag::Rejected => ("ban", &style.rejected),
            };
            draw.icon(rect, &IconSpec::svg(IconId::new(id)).color(cx.resolve(&look.color)));
        }
        if let (Some(color), Some(rect)) = (self.label_color, placed.dot) {
            let shape = PaintShape::Circle(rect.center(), rect.width() / 2.0);
            draw.data_fill(&shape, DataColor::new(Color32::from(color)));
            draw.stroke(&shape, StrokeWidth::Thin, style.dot_edge);
        }
    }

    fn paint_pills(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        let style = cx.style;
        for (at, (badge, rect)) in self.pills.iter().zip(&placed.pills).enumerate() {
            match &badge.kind {
                BadgeKind::Text(_) => {
                    if let Some(face) = Self::badge_face(badge) {
                        ui.face(PILL.at(at as u64), face);
                    }
                }
                BadgeKind::Glyph(id) => {
                    let draw = ui.draw();
                    draw.fill(&PaintShape::Rect(*rect, style.pill_radius), style.pill);
                    let icon = Rect::from_center_size(rect.center(), Vec2::splat(style.pill_icon.side));
                    draw.icon(icon, &IconSpec::svg(id.clone()).color(cx.resolve(&style.pill_icon.color)));
                }
            }
        }
    }

    /// Where a caption line of `width` starts in `rect`: centred, or at the reading start in a card.
    fn text_x(&self, rect: Rect, width: f32, cx: &Cx<'_, Self>) -> f32 {
        match (cx.style.sizes.card, cx.flow.is_rtl()) {
            (false, _) => rect.center().x - width / 2.0,
            (true, false) => rect.left(),
            (true, true) => rect.right() - width,
        }
    }

    /// The name and detail in the caption band or column, the slash of a hidden tile and the marks.
    fn paint_caption(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        if self.caption == CaptionPlace::None {
            return;
        }
        let style = cx.style;
        let draw = ui.draw();
        let failed = matches!(self.status, TileStatus::Offline(_));
        let look = if failed { &style.name_failed } else { &style.name };
        let ink = cx.resolve(&look.color);
        if !self.renaming {
            let run = TextRun::new(cx.resolved.label).role(look.role).color(ink).wrap(Wrap::ElidePath);
            let layout = draw.layout(&run, placed.name.width());
            let at = Pos2::new(self.text_x(placed.name, layout.size().x, cx), placed.name.top());
            draw.galley(at, &layout);
            if self.hidden && self.caption == CaptionPlace::Start {
                let name = Rect::from_min_size(at, layout.size());
                draw.stroke(&PaintShape::Path(vec![name.left_bottom(), name.right_top()]), StrokeWidth::Medium, ink);
            }
        }
        if let (Some(detail), Some(rect)) = (&self.detail, placed.detail) {
            let run = TextRun::new(detail.as_str()).role(style.detail.role).color(cx.resolve(&style.detail.color)).wrap(Wrap::ElidePath);
            let layout = draw.layout(&run, rect.width());
            draw.galley(Pos2::new(self.text_x(rect, layout.size().x, cx), rect.top()), &layout);
        }
        for (mark, rect) in self.marks.iter().zip(&placed.marks) {
            draw.icon(*rect, &IconSpec::svg(mark.icon.clone()).color(cx.resolve(&style.mark.color)));
        }
    }
}
