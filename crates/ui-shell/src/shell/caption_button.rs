//! `CaptionButton`: minimize, maximize or restore, and close, in the four desktop families. The title bar shows one
//! per caption and sends the window command of the click; the button only reports it.

use super::caption_icons::{CaptionIcon, CaptionIcons};
use super::words::ShellWords;
use super::{Caption, CaptionFamily, Captions, SlotSize};
use crate::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, PartsCx, Role, Styled as _, WordSetCx as _};
use crate::primitives::{Glyph, IconSpec, PaintShape, TextureCache};
use crate::runtime::access::Name;
use crate::runtime::env::Label;
use crate::runtime::windows::WindowHit;
use crate::style::{ComponentStyle, IconLook, StyleColor, StyleCx, UiTokens};
use craft_behavior::{Metrics, PartKey, Sense};
use craft_theme::{Stateful, States};
use egui::emath::GuiRounding as _;
use egui::{Color32, ColorImage, CornerRadius, Painter, Pos2, Rect, Response, Stroke, StrokeKind, Vec2, pos2, vec2};

#[cfg(feature = "testing")]
mod stories;

/// The button's one part: the whole slot.
const BUTTON: PartKey = PartKey::new("button");

/// The width of one Windows caption button, as the native ones.
const WINDOWS_BUTTON_WIDTH: f32 = 46.0;

/// libadwaita: 24 pt buttons 13 pt apart (6.5 pt of gap on each side, so the hit areas touch), 10 pt from the
/// window's edge.
const ADWAITA: CaptionMetrics = CaptionMetrics { side: 24.0, gap: 6.5, edge: 10.0 };
/// Breeze sizes buttons by the font (about 1.3 em, 18 pt at KDE's default 10 pt font), half a button apart, 8 pt from
/// the edge.
const BREEZE: CaptionMetrics = CaptionMetrics { side: 18.0, gap: 4.5, edge: 8.0 };
const WINDOWS: CaptionMetrics = CaptionMetrics { side: WINDOWS_BUTTON_WIDTH, gap: 0.0, edge: 0.0 };

/// The side of the Windows glyphs' box.
const WINDOWS_GLYPH: f32 = 10.0;
/// The side of the round families' glyph grid.
const ROUND_GLYPH: f32 = 16.0;

/// The glyph strokes, in the units of the box they are drawn in.
const WINDOWS_STROKE: f32 = 1.0;
const BREEZE_STROKE: f32 = 1.0;
const BREEZE_CLOSE_STROKE: f32 = 1.2;
const ADWAITA_FRAME_STROKE: f32 = 2.0;
const ADWAITA_CLOSE_STROKE: f32 = 1.5;

/// The Windows restore glyph's offset of the front square from the back one.
const RESTORE_OFFSET: f32 = 2.0;
/// Breeze: down chevron to minimize, up chevron to maximize, diamond to restore, cross to close.
const BREEZE_MINIMIZE: [(f32, f32); 3] = [(2.5, 5.5), (8.0, 11.0), (13.5, 5.5)];
const BREEZE_MAXIMIZE: [(f32, f32); 3] = [(2.5, 10.5), (8.0, 5.0), (13.5, 10.5)];
const BREEZE_RESTORE: [(f32, f32); 4] = [(8.0, 3.0), (13.0, 8.0), (8.0, 13.0), (3.0, 8.0)];
const BREEZE_CLOSE: [(f32, f32); 2] = [(3.6, 3.6), (12.4, 12.4)];
/// libadwaita: a bar low down, a square outline (smaller to restore), a cross.
const ADWAITA_MINIMIZE: ((f32, f32), (f32, f32)) = ((4.0, 10.0), (12.0, 12.0));
const ADWAITA_MAXIMIZE: (f32, f32) = (4.0, 12.0);
const ADWAITA_RESTORE: (f32, f32) = (5.0, 11.0);
const ADWAITA_CLOSE: [(f32, f32); 2] = [(4.5, 4.5), (11.5, 11.5)];

/// Which end of the window a button is outermost at. Caption layouts are the user's desktop setting and physical, so
/// this is a physical side, not a reading direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CaptionEnd {
    /// The left edge.
    Left,
    /// The right edge.
    Right,
}

/// The sizes one family lays its buttons out with, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CaptionMetrics {
    /// The button's face: the disc's diameter, or the width of a Windows button.
    side: f32,
    /// The gap on each side of a button's face, split between neighbours so the hit areas touch.
    gap: f32,
    /// The gap between the outermost button's face and the window's edge, part of that button's hit area.
    edge: f32,
}

impl CaptionMetrics {
    /// The metrics of `family`.
    fn of(family: CaptionFamily) -> Self {
        match Face::of(family) {
            Face::Windows => WINDOWS,
            Face::Adwaita | Face::Generic => ADWAITA,
            Face::Breeze => BREEZE,
        }
    }
}

/// Where one button lies in a bar.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CaptionSlot {
    /// The button.
    pub caption: Caption,
    /// Its hit area, the full bar height.
    pub rect: Rect,
    /// Set for the button nearest a window edge, whose hit area runs to that edge.
    pub end: Option<CaptionEnd>,
}

/// The caption buttons of a window at both ends of its bar.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CaptionPlan {
    family: CaptionFamily,
    left: Vec<Caption>,
    right: Vec<Caption>,
}

impl CaptionPlan {
    /// The buttons of `captions` at the (left, right) ends, left to right. A right-to-left UI mirrors the Windows
    /// captions; the Linux desktops' layouts are the user's and stay as set.
    pub fn new(captions: &Captions, rtl: bool) -> Self {
        let (mut left, mut right) = (captions.layout.left.clone(), captions.layout.right.clone());
        if rtl && captions.family == CaptionFamily::Windows {
            std::mem::swap(&mut left, &mut right);
            left.reverse();
            right.reverse();
        }
        CaptionPlan { family: captions.family, left, right }
    }

    /// The buttons at the left end, left to right.
    pub fn left(&self) -> &[Caption] {
        &self.left
    }

    /// The buttons at the right end, left to right.
    pub fn right(&self) -> &[Caption] {
        &self.right
    }

    /// The width the buttons of the left end take, edge gap included.
    pub fn left_width(&self) -> f32 {
        Self::side_width(self.family, &self.left)
    }

    /// The width the buttons of the right end take, edge gap included.
    pub fn right_width(&self) -> f32 {
        Self::side_width(self.family, &self.right)
    }

    /// Every button's slot in `bar` (the title bar's outer rect), full bar height, between the window's `left` and
    /// `right` edges: flush with them even where the bar's contents run wider, so a maximized window's corner hits
    /// the outermost button.
    pub fn place(&self, bar: Rect, left: f32, right: f32) -> Vec<CaptionSlot> {
        let slot = |(caption, (from, to), end): (Caption, (f32, f32), Option<CaptionEnd>)| CaptionSlot {
            caption,
            rect: Rect::from_min_max(pos2(from, bar.top()), pos2(to, bar.bottom())),
            end,
        };
        let lefts = Self::slots(self.family, &self.left, left, Some(CaptionEnd::Left));
        let rights = Self::slots(self.family, &self.right, right - self.right_width(), Some(CaptionEnd::Right));
        lefts.into_iter().chain(rights).map(slot).collect()
    }

    /// Each button's horizontal extent on a side starting at `from`, in order. `outer` is the end the side's edge gap
    /// belongs to: before its first button at the left end, after its last at the right.
    fn slots(family: CaptionFamily, side: &[Caption], from: f32, outer: Option<CaptionEnd>) -> Vec<(Caption, (f32, f32), Option<CaptionEnd>)> {
        let CaptionMetrics { side: size, gap, edge } = CaptionMetrics::of(family);
        let mut x = from;
        let count = side.len();
        side.iter()
            .enumerate()
            .map(|(i, &caption)| {
                let at_left = outer == Some(CaptionEnd::Left) && i == 0;
                let at_right = outer == Some(CaptionEnd::Right) && i + 1 == count;
                let (before, after) = (if at_left { edge } else { gap }, if at_right { edge } else { gap });
                let extent = (x, x + before + size + after);
                x = extent.1;
                (caption, extent, outer.filter(|_| at_left || at_right))
            })
            .collect()
    }

    fn side_width(family: CaptionFamily, side: &[Caption]) -> f32 {
        Self::slots(family, side, 0.0, Some(CaptionEnd::Left)).last().map_or(0.0, |(_, (_, right), _)| *right)
    }
}

/// The family as a style key: the foreign type has no `Hash`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Face {
    Windows,
    Adwaita,
    Breeze,
    Generic,
}

impl Face {
    fn of(family: CaptionFamily) -> Self {
        match family {
            CaptionFamily::Windows => Face::Windows,
            CaptionFamily::Adwaita => Face::Adwaita,
            CaptionFamily::Breeze => Face::Breeze,
            CaptionFamily::Generic => Face::Generic,
        }
    }

    fn round(self) -> bool {
        self != Face::Windows
    }
}

/// The style of a caption button.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CaptionButtonStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The face: the whole slot for Windows, a disc for the round families.
    fill: Stateful<StyleColor>,
    /// The glyph: its box and its colour per state.
    #[style(icon)]
    glyph: IconLook,
    /// Whether the face is a disc of `side` points; otherwise it fills the slot.
    round: bool,
    /// The disc's diameter.
    side: f32,
    /// The gap on each side of the face, and between the outermost face and the window's edge.
    gap: f32,
    edge: f32,
}

/// One caption button of a title bar. A click is reported in the response; the title bar sends the window command.
/// The button is not a Tab stop, as the operating system's own are not, and screen readers press it through AccessKit.
/// Its window role is exclusive: the Windows hit test answers a press on it as that caption, which is what snap
/// layouts need.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct CaptionButton<'a> {
    #[common]
    common: Common,
    caption: Caption,
    #[style_key]
    face: Face,
    /// Whether the caption is the close button, which the families colour apart.
    #[style_key]
    close: bool,
    /// Whether the window is maximized: Maximize then shows the restore glyph and the restore name.
    #[prop]
    maximized: bool,
    /// The edge the button is nearest to, which its hit area runs to; the face then sits inside the area.
    #[prop]
    outermost: Option<CaptionEnd>,
    /// Glyphs from the desktop's icon theme for the round families.
    #[prop]
    icons: Option<&'a CaptionIcons>,
}

impl CaptionButton<'_> {
    /// A button for `caption` in the look of `family`.
    pub fn new(caption: Caption, family: CaptionFamily) -> Self {
        CaptionButton {
            common: Common::new(),
            caption,
            face: Face::of(family),
            close: matches!(caption, Caption::Close),
            maximized: false,
            outermost: None,
            icons: None,
        }
    }

    fn word<'w>(&self, labels: &'w ShellWords) -> Option<&'w Label> {
        match self.caption {
            Caption::Minimize => Some(&labels.minimize),
            Caption::Maximize if self.maximized => Some(&labels.restore),
            Caption::Maximize => Some(&labels.maximize),
            Caption::Close => Some(&labels.close),
            _ => None,
        }
    }

    fn icon(&self) -> Option<CaptionIcon> {
        match self.caption {
            Caption::Minimize => Some(CaptionIcon::Minimize),
            Caption::Maximize if self.maximized => Some(CaptionIcon::Restore),
            Caption::Maximize => Some(CaptionIcon::Maximize),
            Caption::Close => Some(CaptionIcon::Close),
            _ => None,
        }
    }
}

impl Component for CaptionButton<'_> {
    const ROLE: Role = Role::BUTTON;
    const STOPS: bool = false;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = CaptionButtonStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, key: CaptionButtonStyleKey) -> CaptionButtonStyle {
        let colors = &cx.tokens.color;
        let (normal, transparent) = (cx.color(colors.text.normal), cx.color(craft_theme::Rgba::TRANSPARENT));
        let states = |rest, hover, pressed| Stateful::new(rest).off(States::new(rest, hover, pressed, rest));
        let metrics = CaptionMetrics::of(match key.face {
            Face::Windows => CaptionFamily::Windows,
            Face::Adwaita => CaptionFamily::Adwaita,
            Face::Breeze => CaptionFamily::Breeze,
            Face::Generic => CaptionFamily::Generic,
        });
        // A window that is not active tints the glyphs until the pointer lights the button.
        let resting = if cx.active { normal } else { cx.color(colors.caption.inactive) };
        let (hover, pressed) = (cx.color(colors.state.hover), cx.color(colors.state.pressed));
        let close = cx.color(colors.caption.close_hover);
        let close_pressed = cx.color(colors.caption.close_pressed);
        let close_text = cx.color(colors.caption.close_text);
        let (fill, lit) = match (key.face, key.close) {
            (Face::Windows, true) => (states(transparent, close, close_pressed), close_text),
            (Face::Windows, false) | (Face::Generic, _) => (states(transparent, hover, pressed), normal),
            (Face::Adwaita, _) => (states(cx.color(colors.caption.disc), hover, pressed), normal),
            // KWin inverts a lit button: the disc takes the glyph colour (Close its red) and the glyph the bar's.
            (Face::Breeze, true) => (states(transparent, close, close_pressed), close_text),
            (Face::Breeze, false) => (states(transparent, normal, cx.color(colors.caption.pressed)), cx.color(colors.surface.chrome)),
        };
        let glyph = IconLook::new(
            if key.face.round() { ROUND_GLYPH } else { WINDOWS_GLYPH },
            Stateful::new(resting).off(States::new(resting, lit, lit, resting)),
        )
        .on(fill.clone());
        let slot = if key.face.round() {
            Vec2::new(metrics.gap + metrics.side + metrics.gap, metrics.side)
        } else {
            Vec2::new(metrics.side, SlotSize::TitleBar.px(cx))
        };
        CaptionButtonStyle {
            metrics: Metrics::new().min_size(slot),
            fill,
            glyph,
            round: key.face.round(),
            side: metrics.side,
            gap: metrics.gap,
            edge: metrics.edge,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let words = cx.words::<ShellWords>();
        let mut part = cx.part(BUTTON, rect);
        part = part.sense(Sense::CLICK).pointer_only("the operating system's window commands minimize, maximize and close the window");
        part = part.exclusive(WindowHit::Caption(self.caption));
        if let Some(word) = self.word(&words) {
            part.tooltip(word.clone());
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if let Some(word) = self.word(&cx.words::<ShellWords>()) {
            access.name(Name::Text(word.as_str().to_owned()));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let inward = match self.outermost {
            Some(CaptionEnd::Left) => 1.0,
            Some(CaptionEnd::Right) => -1.0,
            None => 0.0,
        };
        let center = cx.rect.center() + vec2(inward * (style.edge - style.gap) / 2.0, 0.0);
        let draw = ui.draw();
        let fill = cx.resolve(&style.fill);
        if style.round {
            draw.fill(&PaintShape::Circle(center, style.side / 2.0), fill);
        } else {
            draw.fill(&PaintShape::Rect(cx.rect, CornerRadius::ZERO), fill);
        }
        let mark = Mark { face: self.face, caption: self.caption, maximized: self.maximized, icon: self.icon(), icons: self.icons };
        let glyph = Rect::from_center_size(center, Vec2::splat(style.glyph.side));
        draw.icon(glyph, &IconSpec::glyph(&mark).color(cx.resolve(&style.glyph.color)));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A caption glyph drawn by hand, or from the icon theme where the family allows and the theme has it.
struct Mark<'a> {
    face: Face,
    caption: Caption,
    maximized: bool,
    icon: Option<CaptionIcon>,
    icons: Option<&'a CaptionIcons>,
}

impl Glyph for Mark<'_> {
    fn paint(&self, painter: &Painter, _: &UiTokens, rect: Rect, color: Color32, _: bool) {
        if self.face == Face::Windows {
            self.windows(painter, rect, color);
            return;
        }
        // Whole pixels, so the glyphs' edges stay sharp.
        let ppp = painter.pixels_per_point();
        let rect = Rect::from_min_size(rect.min.round_to_pixels(ppp), rect.size());
        if !self.themed(painter, rect, color) {
            self.round(painter, rect, color);
        }
    }
}

impl Mark<'_> {
    /// The 10 pt line glyphs of the Windows buttons.
    fn windows(&self, painter: &Painter, rect: Rect, color: Color32) {
        let scale = rect.width() / WINDOWS_GLYPH;
        let stroke = Stroke::new(WINDOWS_STROKE * scale, color);
        let offset = RESTORE_OFFSET * scale;
        match self.caption {
            Caption::Minimize => {
                painter.line_segment([rect.left_center(), rect.right_center()], stroke);
            }
            // Restore: a front square with the corner of the one behind it.
            Caption::Maximize if self.maximized => {
                let front = Rect::from_min_max(rect.min + vec2(0.0, offset), rect.max - vec2(offset, 0.0));
                painter.rect_stroke(front, 0.0, stroke, StrokeKind::Middle);
                let back = vec![
                    rect.min + vec2(offset, offset),
                    rect.min + vec2(offset, 0.0),
                    rect.right_top(),
                    rect.max - vec2(0.0, offset),
                    rect.max - vec2(offset, offset),
                ];
                painter.line(back, stroke);
            }
            Caption::Maximize => {
                painter.rect_stroke(rect, 0.0, stroke, StrokeKind::Middle);
            }
            Caption::Close => {
                painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
                painter.line_segment([rect.right_top(), rect.left_bottom()], stroke);
            }
            _ => {}
        }
    }

    /// The theme's glyph as a mask tinted with `color`; `false` when there is none to show.
    fn themed(&self, painter: &Painter, rect: Rect, color: Color32) -> bool {
        let (Some(icons), Some(icon)) = (self.icons, self.icon) else { return false };
        let ppp = painter.pixels_per_point();
        let px = (rect.width() * ppp).round() as u32;
        let Some(mask) = icons.mask(icon, px) else { return false };
        let side = px as usize;
        if mask.len() != side * side {
            return false;
        }
        let key = (icons.theme().to_owned(), icons.generation(), icon, px);
        let texture = TextureCache::texture(painter.ctx(), key, || {
            // White at the mask's coverage, tinted with the ink when drawn.
            let pixels = mask.iter().map(|coverage| Color32::from_white_alpha(*coverage)).collect();
            ColorImage { size: [side, side], source_size: vec2(px as f32, px as f32), pixels }
        });
        let shown = Rect::from_min_size(rect.center().round_to_pixels(ppp) - Vec2::splat(px as f32 / ppp / 2.0), Vec2::splat(px as f32 / ppp));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        crate::runtime::egui::textures::paint(painter, texture, shown, uv, color, egui::CornerRadius::ZERO);
        true
    }

    /// A round family's glyph on a 16 pt grid.
    fn round(&self, painter: &Painter, rect: Rect, color: Color32) {
        let scale = rect.width() / ROUND_GLYPH;
        let at = |(x, y): (f32, f32)| rect.min + vec2(x, y) * scale;
        if self.face == Face::Breeze {
            let line = Stroke::new(BREEZE_STROKE * scale, color);
            match self.caption {
                Caption::Minimize => {
                    painter.line(BREEZE_MINIMIZE.map(at).to_vec(), line);
                }
                Caption::Maximize if self.maximized => {
                    painter.add(egui::Shape::closed_line(BREEZE_RESTORE.map(at).to_vec(), line));
                }
                Caption::Maximize => {
                    painter.line(BREEZE_MAXIMIZE.map(at).to_vec(), line);
                }
                Caption::Close => self.cross(painter, BREEZE_CLOSE, at, Stroke::new(BREEZE_CLOSE_STROKE * scale, color)),
                _ => {}
            }
            return;
        }
        match self.caption {
            Caption::Minimize => {
                let (from, to) = ADWAITA_MINIMIZE;
                painter.rect_filled(Rect::from_min_max(at(from), at(to)), 0.0, color);
            }
            Caption::Maximize => {
                let (from, to) = if self.maximized { ADWAITA_RESTORE } else { ADWAITA_MAXIMIZE };
                let square = Rect::from_min_max(at((from, from)), at((to, to)));
                painter.rect_stroke(square, 0.0, Stroke::new(ADWAITA_FRAME_STROKE * scale, color), StrokeKind::Inside);
            }
            Caption::Close => self.cross(painter, ADWAITA_CLOSE, at, Stroke::new(ADWAITA_CLOSE_STROKE * scale, color)),
            _ => {}
        }
    }

    /// The cross whose diagonal runs between the two points of `corners`.
    fn cross(&self, painter: &Painter, corners: [(f32, f32); 2], at: impl Fn((f32, f32)) -> Pos2, stroke: Stroke) {
        let [(x0, y0), (x1, y1)] = corners;
        painter.line_segment([at((x0, y0)), at((x1, y1))], stroke);
        painter.line_segment([at((x1, y0)), at((x0, y1))], stroke);
    }
}
