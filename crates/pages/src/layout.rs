//! Page layout and view (N3.06): where each page of a document sits in the scroll content of a
//! page view, at a zoom, and the zoom a fit mode gives.
//!
//! Two apps lay pages out today and they differ (fact sheet `S6b-pages-facts.md`): PdfCraft
//! shows one page across or a two-page spread (with Acrobat's cover-page rule), continuous or one
//! row at a time, with a side gutter; WordCraft flows as many pages across as fit (Word's Print Layout zoomed
//! out), with a markup column beside each page and Hide White Space. [`PageDisplay`] holds both,
//! and [`LayoutOptions`] carries each app's switches ([`LayoutOptions::ACROBAT`],
//! [`LayoutOptions::WORD`]). The screen spacing ([`PageSpacing`]: gap, margin, side gutter, Hide
//! White Space's pad and seam) is not a constant here: the page view reads it from the `pages.*`
//! size tokens (`PageView::style`), which an app overrides in its `sizes.json` (WordCraft's wider
//! gap and no side gutter).
//!
//! ```ignore
//! let sizes: Vec<Size> = doc.pages.iter().map(|p| p.size()).collect();
//! let spacing = PageView::style(&tokens).spacing;
//! let scale = fit_scale(&sizes, viewport, view.fit, &options, &spacing, view.current).unwrap_or(view.zoom * SCREEN_POINTS_PER_POINT);
//! let laid = place_pages(&sizes, scale, viewport, &options, &spacing);
//! let shown = if options.continuous { 0..sizes.len() } else { shown_row(sizes.len(), view.current, &options) };
//! ```

use std::ops::{Range, RangeInclusive};

use kurbo::{Point, Rect, Size};

/// Screen points per page point at 100 %: 96 dpi, as browsers, Acrobat and Word show a page
/// (PdfCraft's `PT`, WordCraft's `PX_PER_PT`).
pub const SCREEN_POINTS_PER_POINT: f64 = 96.0 / 72.0;

/// The zooms a page view shows, 8 % to 6400 % (Acrobat's range; PdfCraft clamps to it).
pub const ZOOM_RANGE: RangeInclusive<f64> = 0.08..=64.0;

/// The page size a non-finite or tiny page counts as, page points.
const FALLBACK_PAGE: f64 = 72.0;

/// The narrowest width a fit works with.
const MIN_FIT_WIDTH: f64 = 100.0;

fn sane_dim(v: f64) -> f64 {
    if v.is_finite() && v >= 1.0 { v } else { FALLBACK_PAGE }
}

fn sane_scale(scale: f64) -> f64 {
    if scale.is_finite() {
        scale.clamp(ZOOM_RANGE.start() * SCREEN_POINTS_PER_POINT, ZOOM_RANGE.end() * SCREEN_POINTS_PER_POINT)
    } else {
        SCREEN_POINTS_PER_POINT
    }
}

fn sane_extent(v: f64) -> f64 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

/// How pages are placed across the view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageDisplay {
    /// One page across (Acrobat's Single Page, Word's One Page and Page Width).
    #[default]
    Single,
    /// Two-page spreads (Acrobat's Two Page); with [`LayoutOptions::cover`] the first page sits
    /// alone on the right.
    TwoUp,
    /// As many pages across as fit, at least [`LayoutOptions::min_columns`] (Word's Print Layout
    /// zoomed out, and Multiple Pages with a minimum of two).
    Columns,
}

impl PageDisplay {
    /// The order the View menu, a rail button and Preferences list them.
    pub const ORDER: [PageDisplay; 3] = [PageDisplay::Single, PageDisplay::TwoUp, PageDisplay::Columns];
}

/// The fit modes. A fit recomputes the zoom whenever the viewport or the pages change; a zoom the
/// user sets is [`Fit::None`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fit {
    /// Fit Width (Word's Page Width): the widest page, or the widest spread, fills the width.
    #[default]
    Width,
    /// Fit Page (Word's One Page; Multiple Pages with [`PageDisplay::Columns`]): a whole page or
    /// spread shows.
    Page,
    /// Fit Height.
    Height,
    /// A fixed zoom.
    None,
}

/// The constants and switches of a layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutOptions {
    #[allow(missing_docs)]
    pub display: PageDisplay,
    /// [`PageDisplay::TwoUp`]: the first page alone, as a cover.
    pub cover: bool,
    /// Every page scrolls past; off shows one row (a page or a spread) at a time ([`shown_row`]).
    pub continuous: bool,
    /// Pages keep [`PageSpacing::gap`] between them; off joins page edges (PdfCraft's Show page
    /// gaps off: pages are then sized in whole device pixels so no hairline seams show).
    pub page_gaps: bool,
    /// Page points beside each page for comment and revision balloons (WordCraft's markup area,
    /// 216 pt when shown), or 0.
    pub markup: f64,
    /// [`PageDisplay::Columns`]: at least this many pages across.
    pub min_columns: usize,
}

impl LayoutOptions {
    /// PdfCraft's: single page, continuous, page gaps.
    pub const ACROBAT: LayoutOptions =
        LayoutOptions { display: PageDisplay::Single, cover: false, continuous: true, page_gaps: true, markup: 0.0, min_columns: 1 };
    /// WordCraft's: as many across as fit, continuous, page gaps.
    pub const WORD: LayoutOptions =
        LayoutOptions { display: PageDisplay::Columns, cover: false, continuous: true, page_gaps: true, markup: 0.0, min_columns: 1 };
}

/// The spacing of a page layout: screen points, except [`white_space_pad`](Self::white_space_pad),
/// which is in points at 100 % zoom and is multiplied by the zoom. The page view takes it from the `pages.*` size tokens
/// (`craft_pages::ui::PageView::style`); this crate's core has no tokens, so the layout functions
/// take it as a value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSpacing {
    /// Between pages, and between rows (the page view's gap); unused with
    /// [`LayoutOptions::page_gaps`] off.
    pub gap: f64,
    /// Above the first and below the last row (the page view's margin).
    pub margin: f64,
    /// Kept clear at each side (PdfCraft's gutter for its floating
    /// quick-action bar).
    pub side: f64,
    /// What Hide White Space leaves above and below each page's text area, in points at 100 % zoom:
    /// it is multiplied by the zoom, as WordCraft's `WHITE_SPACE_PAD` is (`canvas/geometry.rs:17,40`).
    pub white_space_pad: f64,
    /// The seam Hide White Space draws between pages, in screen points at any zoom.
    pub seam: f64,
}

/// Pages placed in the scroll content of a view, origin at the content's top-left.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PagedLayout {
    /// Each page's rectangle in screen points.
    pub rects: Vec<Rect>,
    /// The content's size: at least the viewport's.
    pub size: Size,
    /// Pages per row.
    pub columns: usize,
    /// Per page, the part of its height that shows (screen points from its top): the whole page,
    /// except with Hide White Space ([`PagedLayout::hide_white_space`]).
    pub bands: Vec<(f64, f64)>,
}

/// Place pages of `sizes` (page points) at `scale` screen points per page point in a viewport
/// `viewport` wide and tall.
///
/// - [`PageDisplay::Single`]: each page centred, at least [`side`](PageSpacing::side) from the
///   left, `gap` below the one before (none with [`page_gaps`](LayoutOptions::page_gaps) off),
///   from `margin` at the top.
/// - [`PageDisplay::TwoUp`]: pages in pairs, each pair centred and a shorter page centred vertically
///   on the taller one; with a cover the first
///   page sits alone right of the centre line.
/// - [`PageDisplay::Columns`]: rows of as many slots as fit (`(width - gap) / (slot + gap)`,
///   at least `min_columns`, at most the page count), each slot as wide as the widest page plus
///   the markup area; a narrower page is centred in its slot, keeping its markup area beside it.
///
/// Sizes that aren't finite or are below 1 pt count as 72 pt (WordCraft's `sane_dim`); the scale
/// is clamped to [`ZOOM_RANGE`] × [`SCREEN_POINTS_PER_POINT`], so every zoom the view offers lays
/// out as shown.
// pdfcraft: crates/ui-egui/src/canvas/zoom.rs:306 `layout`; wordcraft:
// crates/ui-egui/src/canvas/geometry.rs:126 `columns_for`, :139 `place_pages`.
pub fn place_pages(sizes: &[Size], scale: f64, viewport: Size, options: &LayoutOptions, spacing: &PageSpacing) -> PagedLayout {
    let scale = sane_scale(scale);
    let vw = sane_extent(viewport.width);
    let vh = sane_extent(viewport.height);
    let gap = if options.page_gaps { spacing.gap } else { 0.0 };
    let dims: Vec<Size> = sizes.iter().map(|s| Size::new(sane_dim(s.width), sane_dim(s.height))).collect();
    let shown: Vec<Size> = dims
        .iter()
        .map(|d| {
            let screen = Size::new(d.width * scale, d.height * scale);
            // Without gaps, whole-point sizes keep neighbouring edges on the same pixel.
            if options.page_gaps { screen } else { Size::new(screen.width.round(), screen.height.round()) }
        })
        .collect();
    let mut rects = Vec::with_capacity(shown.len());
    let mut y = spacing.margin;
    let mut columns = 1;
    let content_w;
    match options.display {
        PageDisplay::Single => {
            let widest = shown.iter().map(|s| s.width).fold(0.0, f64::max);
            content_w = (widest + 2.0 * spacing.side).max(vw);
            for size in &shown {
                rects.push(Rect::from_origin_size(Point::new(((content_w - size.width) / 2.0).max(spacing.side), y), *size));
                y += size.height + gap;
            }
        }
        PageDisplay::TwoUp => {
            columns = if shown.len() > 1 { 2 } else { 1 };
            let rows: Vec<&[Size]> = match shown.split_first() {
                Some((_, rest)) if options.cover => std::iter::once(&shown[..1]).chain(rest.chunks(2)).collect(),
                _ => shown.chunks(2).collect(),
            };
            let row_width = |row: &[Size]| row.iter().map(|s| s.width).sum::<f64>() + gap * (row.len() as f64 - 1.0);
            content_w = (rows.iter().map(|r| row_width(r)).fold(0.0, f64::max) + 2.0 * spacing.side).max(vw);
            for (index, row) in rows.into_iter().enumerate() {
                let row_h = row.iter().map(|s| s.height).fold(0.0, f64::max);
                let mut x = if options.cover && index == 0 {
                    (content_w / 2.0 + gap / 2.0).max(spacing.side)
                } else {
                    ((content_w - row_width(row)) / 2.0).max(spacing.side)
                };
                for size in row {
                    rects.push(Rect::from_origin_size(Point::new(x, y + (row_h - size.height) / 2.0), *size));
                    x += size.width + gap;
                }
                y += row_h + gap;
            }
        }
        PageDisplay::Columns => {
            let markup = if options.markup.is_finite() { options.markup.max(0.0) } else { 0.0 };
            let widest = dims.iter().map(|d| d.width).fold(0.0, f64::max);
            let slot = (widest + markup) * scale;
            let fit = ((vw - gap) / (slot + gap)).floor().max(1.0) as usize;
            columns = fit.max(options.min_columns).min(dims.len().max(1)).max(1);
            let row_w = columns as f64 * slot + (columns as f64 - 1.0) * gap;
            content_w = (row_w + 2.0 * gap).max(vw);
            for row in shown.chunks(columns).zip(dims.chunks(columns)) {
                let row_h = row.0.iter().map(|s| s.height).fold(0.0, f64::max);
                for (i, (size, dim)) in row.0.iter().zip(row.1).enumerate() {
                    let centred = ((widest - dim.width) * scale / 2.0).max(0.0);
                    let x = (content_w - row_w) / 2.0 + i as f64 * (slot + gap) + centred;
                    rects.push(Rect::from_origin_size(Point::new(x, y), *size));
                }
                y += row_h + gap;
            }
        }
    }
    let bottom = if rects.is_empty() { 0.0 } else { y - gap + spacing.margin };
    let bands = rects.iter().map(|r| (0.0, r.height())).collect();
    PagedLayout { rects, size: Size::new(content_w, bottom.max(vh)), columns, bands }
}

impl PagedLayout {
    /// Word's Hide White Space: single-column pages stacked so that only each page's text area
    /// (`bodies`: the top and bottom of its body, page points) plus
    /// [`white_space_pad`](PageSpacing::white_space_pad) shows, a [`seam`](PageSpacing::seam)
    /// between pages. The first page keeps its top and the last its bottom. Rects keep the full
    /// page size, so page coordinates still map as `(p - rect.min) / scale`; neighbouring rects
    /// overlap in the hidden margins. The pad scales with `scale` (page points), the seam does not
    /// (WordCraft's `WHITE_SPACE_PAD` times the scale, `SEAM` as is). Only for one column and more than one page; otherwise the
    /// layout is unchanged.
    // wordcraft: crates/ui-egui/src/canvas/geometry.rs:28 `collapse_white_space` (its
    // `WHITE_SPACE_PAD` and `SEAM` are the spacing's pad and seam).
    pub fn hide_white_space(&mut self, bodies: &[(f64, f64)], scale: f64, viewport_height: f64, spacing: &PageSpacing) {
        let count = self.rects.len();
        if self.columns != 1 || count < 2 {
            return;
        }
        let scale = sane_scale(scale);
        let mut cursor = self.rects[0].y0;
        let mut rects = Vec::with_capacity(count);
        let mut bands = Vec::with_capacity(count);
        for (i, rect) in self.rects.iter().enumerate() {
            let height = rect.height();
            let (body_top, body_bottom) = bodies.get(i).copied().unwrap_or((0.0, height / scale));
            let finite = |v: f64, fallback: f64| if v.is_finite() { v } else { fallback };
            let pad = spacing.white_space_pad * scale;
            let top = if i == 0 { 0.0 } else { (finite(body_top, 0.0) * scale - pad).clamp(0.0, height) };
            let bottom = if i + 1 == count { height } else { (finite(body_bottom, 0.0) * scale + pad).clamp(top, height) };
            let y = cursor - top;
            rects.push(Rect::from_origin_size(Point::new(rect.x0, y), rect.size()));
            bands.push((top, bottom));
            cursor = y + bottom + spacing.seam;
        }
        self.rects = rects;
        self.bands = bands;
        self.size.height = (cursor - spacing.seam + spacing.margin).max(viewport_height);
    }

    /// Whether `p` (content points) is on the gap between two pages, or on the seam with white
    /// space hidden: a double-click there toggles Hide White Space, as in Word.
    // wordcraft: crates/ui-egui/src/canvas/geometry.rs:57 `on_page_gap`.
    pub fn on_page_gap(&self, p: Point) -> bool {
        self.rects.windows(2).zip(self.bands.windows(2)).any(|(pair, bands)| {
            let (above, below) = (pair[0], pair[1]);
            if p.x < above.x0.min(below.x0) || p.x > above.x1.max(below.x1) {
                return false;
            }
            let (from, to) = (above.y0 + bands[0].1, below.y0 + bands[1].0);
            // Overlapping rects mean Hide White Space cut them: the seam is a hairline, so the
            // target is widened by its own width on each side.
            let reach = if below.y0 < above.y1 { to - from } else { 0.0 };
            p.y >= from - reach && p.y <= to + reach
        })
    }
}

/// The page whose shown part contains `p` (content points), if any; the canvas around and between
/// pages, and margins Hide White Space cuts, are no page.
// wordcraft: crates/ui-egui/src/canvas/geometry.rs:72 `on_visible_page`.
pub fn page_at(layout: &PagedLayout, p: Point) -> Option<usize> {
    layout.rects.iter().zip(&layout.bands).position(|(r, &(top, bottom))| Rect::new(r.x0, r.y0 + top, r.x1, r.y0 + bottom).contains(p))
}

/// The zoom a fit gives, as a scale (screen points per page point), or `None` for
/// [`Fit::None`]. `current` is the page shown when not continuous.
///
/// - The page fitted is the largest page (so the zoom holds still while pages of other sizes
///   scroll past), except a single page shown on its own, which fits that page.
/// - Width: `(width - 2 × side - gap × (n - 1)) / (page width × n)` with n the pages per row
///   (2 for spreads); Page: the smaller of that and `(height - 2 × margin) / page height`;
///   Height: the second alone. The width is at least 100.
/// - Clamped to [`ZOOM_RANGE`] × [`SCREEN_POINTS_PER_POINT`].
// pdfcraft: crates/ui-egui/src/canvas/zoom.rs:259 `fit_zoom`; wordcraft:
// crates/ui-egui/src/canvas/geometry.rs:178 (pageWidth, onePage, multiplePages).
pub fn fit_scale(sizes: &[Size], viewport: Size, fit: Fit, options: &LayoutOptions, spacing: &PageSpacing, current: Option<usize>) -> Option<f64> {
    if fit == Fit::None || sizes.is_empty() {
        return None;
    }
    let dims = |s: &Size| (sane_dim(s.width), sane_dim(s.height));
    let (width, height) = match current.and_then(|c| sizes.get(c)) {
        Some(page) if options.display == PageDisplay::Single && !options.continuous => dims(page),
        _ => sizes.iter().map(dims).fold((1.0_f64, 1.0_f64), |(w, h), (pw, ph)| (w.max(pw), h.max(ph))),
    };
    let per_row = match options.display {
        PageDisplay::Single => 1.0,
        PageDisplay::TwoUp => 2.0,
        PageDisplay::Columns => options.min_columns.max(1) as f64,
    };
    let gap = if options.page_gaps { spacing.gap } else { 0.0 };
    let available = (sane_extent(viewport.width) - 2.0 * spacing.side).max(MIN_FIT_WIDTH);
    let by_width = (available - gap * (per_row - 1.0)) / ((width + options.markup.max(0.0)) * per_row);
    let by_height = (sane_extent(viewport.height) - 2.0 * spacing.margin) / height;
    let scale = match fit {
        Fit::Width => by_width,
        Fit::Page => by_width.min(by_height),
        Fit::Height => by_height,
        Fit::None => return None,
    };
    scale.is_finite().then(|| scale.clamp(ZOOM_RANGE.start() * SCREEN_POINTS_PER_POINT, ZOOM_RANGE.end() * SCREEN_POINTS_PER_POINT))
}

/// When not continuous, the pages of the row shown: the current page, or the current spread. With
/// a cover, page 0 is alone and spreads are (1, 2), (3, 4), …; without, (0, 1), (2, 3). Empty for
/// no pages.
// pdfcraft: crates/ui-egui/src/canvas.rs:2831 `shown_row`.
pub fn shown_row(count: usize, current: usize, options: &LayoutOptions) -> Range<usize> {
    if count == 0 {
        return 0..0;
    }
    let current = current.min(count - 1);
    match options.display {
        PageDisplay::TwoUp if options.cover && current == 0 => 0..1,
        PageDisplay::TwoUp => {
            let first = spread_first(current, options.cover);
            first..(first + 2).min(count)
        }
        PageDisplay::Single | PageDisplay::Columns => current..current + 1,
    }
}

/// The first page of the spread page `page` is on (with or without a cover).
// pdfcraft: crates/ui-egui/src/canvas/zoom.rs:255 `spread_first`.
pub fn spread_first(page: usize, cover: bool) -> usize {
    if cover { page.saturating_sub((page + 1) % 2) } else { page - page % 2 }
}

/// The one token settings and the control channel use for a display and continuous scrolling:
/// `single`, `continuous` (single, scrolling), `two-page`, `two-up` (two-page, scrolling) and
/// `columns`. PdfCraft's names, kept for its settings files.
// pdfcraft: crates/ui-egui/src/canvas.rs:159 `display_token`.
pub fn display_token(display: PageDisplay, continuous: bool) -> &'static str {
    match (display, continuous) {
        (PageDisplay::Single, false) => "single",
        (PageDisplay::Single, true) => "continuous",
        (PageDisplay::TwoUp, true) => "two-up",
        (PageDisplay::TwoUp, false) => "two-page",
        (PageDisplay::Columns, _) => "columns",
    }
}

/// The display and continuous flag a [`display_token`] names, in any case; `two-up-single` is
/// `two-page`. `None` for anything else, so a typo is refused rather than switching silently.
// pdfcraft: crates/ui-egui/src/canvas.rs:169 `parse_display_token`.
pub fn parse_display_token(token: &str) -> Option<(PageDisplay, bool)> {
    match token.trim().to_ascii_lowercase().as_str() {
        "single" => Some((PageDisplay::Single, false)),
        "continuous" => Some((PageDisplay::Single, true)),
        "two-up" => Some((PageDisplay::TwoUp, true)),
        "two-page" | "two-up-single" => Some((PageDisplay::TwoUp, false)),
        "columns" => Some((PageDisplay::Columns, true)),
        _ => None,
    }
}

/// How a newly opened document is shown (Preferences › Documents and view). A file that asks for
/// its own layout or zoom gets that instead.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ViewDefaults {
    /// Single pages, two-up spreads or columns.
    pub display: PageDisplay,
    /// Every page scrolls past; off shows one row (a page or a spread) at a time.
    pub continuous: bool,
    /// Separate pages with a gap and shadow; off joins adjacent page edges.
    pub page_gaps: bool,
    /// [`Fit::Width`] or [`Fit::Page`], or [`Fit::None`] for a fixed `zoom`.
    pub fit: Fit,
    /// The zoom with [`Fit::None`] (1.0 = 100 %).
    pub zoom: f64,
}

impl Default for ViewDefaults {
    /// Single page, continuous, page gaps, fit width (PdfCraft's).
    fn default() -> Self {
        ViewDefaults { display: PageDisplay::Single, continuous: true, page_gaps: true, fit: Fit::Width, zoom: 1.0 }
    }
}

impl ViewDefaults {
    /// The zoom as settings name it: `fit-width`, `fit-page` or a percentage (`125%`). Fit
    /// Height is saved as `fit-width`, as PdfCraft does.
    // pdfcraft: crates/ui-egui/src/canvas.rs:206 `ViewDefaults::zoom_name`.
    pub fn zoom_name(&self) -> String {
        match self.fit {
            Fit::Page => "fit-page".into(),
            Fit::None => format!("{}%", (self.zoom * 100.0).round()),
            Fit::Width | Fit::Height => "fit-width".into(),
        }
    }

    /// These defaults with the zoom a [`zoom_name`](Self::zoom_name) names, in any case. `None`
    /// for anything else, including a percentage outside 8–6400 %.
    // pdfcraft: crates/ui-egui/src/canvas.rs:216 `ViewDefaults::with_zoom`.
    pub fn with_zoom(self, name: &str) -> Option<Self> {
        let name = name.trim();
        let (fit, zoom) = if name.eq_ignore_ascii_case("fit-width") {
            (Fit::Width, self.zoom)
        } else if name.eq_ignore_ascii_case("fit-page") {
            (Fit::Page, self.zoom)
        } else {
            let percent = name.strip_suffix('%').unwrap_or(name).trim().parse::<f64>().ok()?;
            let zoom = percent / 100.0;
            if !ZOOM_RANGE.contains(&zoom) {
                return None;
            }
            (Fit::None, zoom)
        };
        Some(Self { fit, zoom, ..self })
    }
}
