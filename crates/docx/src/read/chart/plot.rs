use super::canvas::{
    Rect, SWATCH, clip_text, fill, rect_segs, runs, seg, stroke, swatch, text, text_angle, text_at, text_w, vertical_text, widest, wrap,
};
use super::format::format_value;
use super::model::{DataLabel, DataTable, Grouping, Kind, LabelPos, Series, Symbol, TickPos};
use super::parse::{category_text, own_fill, series_name};
use super::polar::{draw_data_labels, label_of, value_text};
use super::{
    AXIS, Chart, El, Fill, GAP, GRID, GraphicItem, LINE, LINE_W, MAX_LABEL, MAX_TICKS, PathSeg, Rgb, ShapeKind, TextAlign, fill_colour, measure,
};

/// Line width of the stroked marker symbols (x, plus, star), points.
const MARKER_STROKE: f32 = 1.5;

pub(super) fn label(s: &str) -> String {
    s.chars().take(MAX_LABEL).collect()
}

/// Chart area: the chart space's fill and border (white and borderless by default).
pub(super) fn backdrop(out: &mut Vec<GraphicItem>, space: &El, theme: &[Rgb], w: f32, h: f32) {
    let sp = space.child("c:spPr");
    // A gradient is painted as the file states it; a solid fill as before.
    let gradient = sp.and_then(|p| own_fill(p, theme)).filter(|f| matches!(f, Fill::Gradient(_)));
    if let Some(fill) = gradient.clone() {
        out.push(GraphicItem::Fill { rect: [0.0, 0.0, w, h], fill });
    }
    let fill = match sp {
        _ if gradient.is_some() => None,
        Some(p) if p.child("a:noFill").is_some() => None,
        Some(p) => fill_colour(p, theme).or(Some(Rgb::WHITE)),
        None => Some(Rgb::WHITE),
    };
    let ln = sp.and_then(|p| p.child("a:ln"));
    let stroke = ln.and_then(|l| fill_colour(l, theme));
    let stroke_width = ln.and_then(|l| l.attr("w")).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.75).clamp(0.0, 20.0);
    if fill.is_some() || stroke.is_some() {
        out.push(GraphicItem::Shape { rect: [0.0, 0.0, w, h], kind: ShapeKind::Rectangle, fill, stroke, stroke_width, alpha: 1.0 });
    }
}

/// An axis scale: `lo` to `hi` in steps of `step`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Scale {
    pub(super) lo: f64,
    pub(super) hi: f64,
    pub(super) step: f64,
}

impl Scale {
    pub(super) fn ticks(&self) -> Vec<f64> {
        let n = (((self.hi - self.lo) / self.step + 1e-9).floor().max(0.0) as usize).min(MAX_TICKS);
        (0..=n).map(|k| self.lo + k as f64 * self.step).filter(|t| *t <= self.hi + 1e-9).collect()
    }
}

/// A scale for data `dlo..dhi` (zero included when `zero`), as Excel and Word pick it: an end
/// away from zero sits 5% of the data span beyond the data (an end at zero stays there), rounded
/// out to a step of 1, 2, 2.5, 5 or 10 times a power of ten (about seven steps). `min`/`max`
/// from the file replace the ends.
pub(super) fn nice_scale(dlo: f64, dhi: f64, zero: bool, min: Option<f64>, max: Option<f64>) -> Scale {
    let (dlo, dhi) = if dhi > dlo { (dlo, dhi) } else { (dlo, dlo + 1.0) };
    let span = dhi - dlo;
    // With zero, the ends reach it: all-negative data still starts its bars at zero.
    let (ends_lo, ends_hi) = if zero { (dlo.min(0.0), dhi.max(0.0)) } else { (dlo, dhi) };
    let lo = min.unwrap_or(if ends_lo < 0.0 { ends_lo - 0.05 * span } else { ends_lo });
    let hi = max.unwrap_or(if ends_hi > 0.0 { ends_hi + 0.05 * span } else { ends_hi });
    if !(hi > lo && hi.is_finite() && lo.is_finite()) {
        return Scale { lo: 0.0, hi: 1.0, step: 0.2 };
    }
    let raw = (hi - lo) / 7.5;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag);
    let lo_r = if min.is_some() { lo } else { ((lo / step) + 1e-9).floor() * step };
    let hi_r = if max.is_some() { hi } else { ((hi / step) - 1e-9).ceil() * step };
    if !(step.is_finite() && step > 0.0 && lo_r.is_finite() && hi_r.is_finite() && hi_r > lo_r) {
        return Scale { lo: 0.0, hi: 1.0, step: 0.2 };
    }
    Scale { lo: lo_r, hi: hi_r, step }
}

/// The plot rectangle and the scales that map values and categories onto it.
struct Frame {
    plot: Rect,
    horiz: bool,
    n: usize,
    vs: Scale,
    xs: Option<Scale>,
    mid: bool,
}

impl Frame {
    /// Width of one category band.
    fn band(&self) -> f32 {
        let len = if self.horiz { self.plot.h } else { self.plot.w };
        len / self.n.max(1) as f32
    }

    /// Start of category `i`'s band: its left edge, or for bars its top edge (category 0 at the
    /// bottom).
    fn band_start(&self, i: usize) -> f32 {
        let k = i as f32;
        if self.horiz { self.plot.bottom() - (k + 1.0) * self.band() } else { self.plot.x + k * self.band() }
    }

    /// A value's position along the value direction: y for columns, x for bars.
    fn val(&self, v: f64) -> f32 {
        let t = ((v - self.vs.lo) / (self.vs.hi - self.vs.lo)).clamp(0.0, 1.0) as f32;
        if self.horiz { self.plot.x + t * self.plot.w } else { self.plot.bottom() - t * self.plot.h }
    }

    /// x of category `i` for line and area points.
    fn cat_x(&self, i: usize) -> f32 {
        if !self.mid {
            return self.plot.x + (i as f32 + 0.5) * self.band();
        }
        if self.n <= 1 { self.plot.x + self.plot.w / 2.0 } else { self.plot.x + self.plot.w * i as f32 / (self.n - 1) as f32 }
    }

    /// x of a scatter point.
    fn scat_x(&self, x: f64) -> f32 {
        match self.xs {
            Some(s) => self.plot.x + ((x - s.lo) / (s.hi - s.lo)).clamp(0.0, 1.0) as f32 * self.plot.w,
            None => self.plot.x,
        }
    }
}

/// The value scale and, for scatter charts, the x scale, with their ticks and tick texts.
pub(super) struct Axes {
    pub(super) vs: Scale,
    vticks: Vec<(f64, String)>,
    xs: Option<Scale>,
    xticks: Vec<(f64, String)>,
}

impl Axes {
    pub(super) fn of(c: &Chart) -> Axes {
        let (dlo, dhi) = extent(c);
        // A 100% stacked chart's axis runs 0..100%, unless other chart types share it: they keep
        // their own values, so the axis takes them in too (the stacked shares stay within 0..1).
        let percent = c.grouping == Grouping::Percent && !c.combined();
        let (vmin, vmax) = if percent { (Some(0.0), Some(1.0)) } else { (c.val.min, c.val.max) };
        // Lines and scatters don't need zero for a narrow range (1000..1050 stays on its own scale);
        // columns and areas stand on zero, so a combination with any of them keeps it.
        let lines_only = |k: Kind| matches!(k, Kind::Line | Kind::Scatter);
        let zero = !(lines_only(c.kind) && c.series.iter().all(|s| lines_only(s.kind))) || !narrow(dlo, dhi);
        let vs = nice_scale(dlo, dhi, zero, vmin, vmax);
        // In display units the labels count thousands (or millions…) of the values.
        let unit = c.val.unit.unwrap_or(1.0);
        let per_unit = |v: f64| Some(v / unit).filter(|d| d.is_finite()).unwrap_or(v);
        let vticks = vs.ticks().into_iter().map(|t| (t, format_value(per_unit(t), &c.val_fmt, per_unit(vs.step)))).collect();
        let xs = (c.kind == Kind::Scatter).then(|| {
            let (xl, xh) = x_extent(c);
            nice_scale(xl, xh, false, c.cat.min, c.cat.max)
        });
        // Scatter x values show in the axis's own format, else their cache's (dates included).
        let xfmt = c.cat.fmt.clone().or_else(|| c.series.iter().find_map(|s| s.x_fmt.clone())).unwrap_or_else(|| "General".to_string());
        let xticks = xs.map(|s| s.ticks().into_iter().map(|t| (t, category_text(t, &xfmt, s.step, c.date1904))).collect()).unwrap_or_default();
        Axes { vs, vticks, xs, xticks }
    }
}

/// The room around the plot for axis labels, and the category labels as they'll be drawn.
struct Margins {
    left: f32,
    /// Part of `left` taken by the rotated category-axis title (bars): the labels start after it.
    title: f32,
    top: f32,
    bottom: f32,
    /// Category labels in lines (none for scatter charts or a deleted category axis).
    wrapped: Vec<Vec<String>>,
    /// Every `skip`-th category label is shown under a column chart.
    skip: usize,
    /// The data table's rows that are drawn: its header and as many series as fit.
    table_rows: usize,
}

/// The rows of a data table (its header and a row per series) that fit in at most half of a
/// chart `area_h` tall, so the plot keeps the rest; the header always does.
fn table_rows(c: &Chart, area_h: f32, lh: f32) -> usize {
    let fit = if lh > 0.0 && area_h.is_finite() { (area_h * 0.5 / lh).floor().clamp(0.0, 1e6) as usize } else { 1 };
    (1 + c.series.len()).min(fit.max(1))
}

/// The margins the axis labels need around the plot in `area`. Category labels under a column
/// chart wrap to the band; labels that still don't fit show every k-th category. Hidden tick
/// labels take no room; an axis title adds one line on its side.
fn axis_margins(c: &Chart, ax: &Axes, area: Rect) -> Margins {
    let fs = c.text_size;
    let lh = fs * LINE;
    let scatter = c.kind == Kind::Scatter;
    let (cat_on, val_on) = (!c.cat.deleted, !c.val.deleted);
    // A data table heads its columns with the categories, so they aren't drawn under the axis.
    let (cat_labels, val_labels) = (cat_on && c.cat.labels && c.data_table.is_none(), val_on && c.val.labels);
    let title = title_left(c, cat_on, val_on, area.w);
    // A display-units label stands turned left of the value labels (under them for bars).
    let unit_room = if val_labels && c.val.unit_label.is_some() { lh + GAP } else { 0.0 };
    let left = if c.horizontal {
        if cat_labels { widest(&c.cats, fs).min(area.w * 0.4) + GAP } else { 0.0 }
    } else if val_labels {
        ax.vticks.iter().map(|(_, s)| text_w(s, fs)).fold(0.0, f32::max) + GAP + unit_room
    } else {
        0.0
    } + title;
    // A data table's series names sit left of the plot, under the value labels.
    let left = match &c.data_table {
        Some(t) => left.max(table_names_w(c, t, fs).min(area.w * 0.4)),
        None => left,
    };
    let band_w = (area.w - left).max(1.0) / c.n as f32;
    let wrapped: Vec<Vec<String>> = if cat_labels && !scatter {
        let width = if c.horizontal { left - title - GAP } else { band_w };
        c.cats.iter().map(|s| wrap(s, width, fs)).collect()
    } else {
        Vec::new()
    };
    let widest_line = wrapped.iter().flatten().map(|s| text_w(s, fs)).fold(0.0, f32::max);
    let skip = ((widest_line / band_w).ceil() as usize).max(1);
    let label_lines = wrapped.iter().map(Vec::len).max().unwrap_or(1) as f32;
    let table_rows = if c.data_table.is_some() { table_rows(c, area.h, lh) } else { 0 };
    let bottom = (if c.data_table.is_some() {
        table_rows as f32 * lh
    } else if cat_labels && !c.horizontal {
        if scatter { lh + GAP } else { label_lines * lh + GAP }
    } else if c.horizontal && val_labels {
        lh + GAP + unit_room
    } else {
        0.0
    }) + title_bottom(c, cat_on, val_on, lh);
    let top = if !c.horizontal && val_labels { lh / 2.0 } else { 0.0 };
    Margins { left, title, top, bottom, wrapped, skip, table_rows }
}

/// Room for the vertical axis title on the plot's left: one line when the axis that sits there
/// (the value axis for columns, the category axis for bars) has a title.
/// A horizontal title (`rot="0"`) gets a strip as wide as its wrapped lines, at most a quarter of
/// `area_w`, so the plot keeps its room.
fn title_left(c: &Chart, cat_on: bool, val_on: bool, area_w: f32) -> f32 {
    let side = if c.horizontal { cat_on && c.cat.title.is_some() } else { val_on && c.val.title.is_some() };
    if !side {
        return 0.0;
    }
    match flat_title_lines(c, area_w) {
        Some(lines) => widest(&lines, c.text_size).min((area_w * 0.25).max(0.0)) + GAP,
        None => c.text_size * LINE,
    }
}

/// The wrapped lines of the left axis's title when the file keeps it horizontal (`rot="0"`).
fn flat_title_lines(c: &Chart, area_w: f32) -> Option<Vec<String>> {
    let left = if c.horizontal { &c.cat } else { &c.val };
    let r = left.title_rot?.rem_euclid(360.0);
    // Only near-zero rotations fit the horizontal strip; non-finite rotations do not.
    if !r.is_finite() || (1.0..=359.0).contains(&r) {
        return None;
    }
    let width = if area_w.is_finite() { (area_w * 0.18).max(1.0) } else { 1.0 };
    left.title.as_ref().map(|t| wrap(t, width, c.text_size))
}

/// Room for the horizontal axis title under the plot: one line when the axis that sits there
/// (the category axis for columns, the value axis for bars) has a title.
fn title_bottom(c: &Chart, cat_on: bool, val_on: bool, lh: f32) -> f32 {
    let side = if c.horizontal { val_on && c.val.title.is_some() } else { cat_on && c.cat.title.is_some() };
    if side { lh } else { 0.0 }
}

/// A bar, column, line, area or scatter chart in `area`: gridlines, series, axes, then labels.
pub(super) fn cartesian(out: &mut Vec<GraphicItem>, c: &Chart, area: Rect) {
    if c.n == 0 {
        return;
    }
    let ax = Axes::of(c);
    let m = axis_margins(c, &ax, area);
    let plot = Rect { x: area.x + m.left, y: area.y + m.top, w: area.w - m.left, h: area.h - m.top - m.bottom };
    if plot.w < 1.0 || plot.h < 1.0 {
        return;
    }
    let g = Frame { plot, horiz: c.horizontal, n: c.n, vs: ax.vs, xs: ax.xs, mid: c.val.mid };
    if let Some(fill) = &c.plot_fill {
        out.push(GraphicItem::Fill { rect: plot.arr(), fill: fill.clone() });
    }
    if c.val.grid {
        draw_grid(out, &g, &ax.vticks);
    }
    if c.cat.grid {
        draw_cat_grid(out, &g, &ax.xticks, c.kind);
    }
    let mut labels = Vec::new();
    match c.kind {
        Kind::Scatter => draw_scatter(out, c, &g, &mut labels),
        Kind::Pie | Kind::Doughnut | Kind::Radar => {}
        // A combination chart draws its areas, then its columns, then its lines over them.
        Kind::Bar | Kind::Line | Kind::Area => {
            if c.has(Kind::Area) {
                areas(out, c, &g, &mut labels);
            }
            if c.has(Kind::Bar) {
                bars(out, c, &g, &mut labels);
            }
            if c.has(Kind::Line) {
                lines(out, c, &g, &mut labels);
            }
        }
    }
    draw_axes(out, c, &g);
    draw_data_labels(out, &labels, c.text_size, g.horiz && c.kind == Kind::Bar);
    if !c.val.deleted && c.val.labels {
        let room = title_left(c, !c.cat.deleted, true, area.w);
        let (fs, lh) = (c.text_size, c.text_size * LINE);
        // A display-units label takes a strip of its own between the title and the labels.
        let unit_room = if c.val.unit_label.is_some() && !g.horiz { lh + GAP } else { 0.0 };
        draw_value_labels(out, c, &g, &ax.vticks, Rect { x: area.x + room + unit_room, w: m.left - room - unit_room, ..area });
        if let Some((label, colour, bold)) = &c.val.unit_label {
            let colour = colour.unwrap_or_else(|| c.val_colour());
            if g.horiz {
                text_at(out, plot.x + plot.w / 2.0, plot.bottom() + GAP / 2.0 + lh, label, fs, colour, *bold);
            } else {
                let label = clip_text(label, plot.h, fs);
                vertical_text(out, Rect { x: area.x + room, y: plot.y, w: lh, h: plot.h }, &label, fs, colour, *bold);
            }
        }
    }
    if let Some(t) = &c.data_table {
        draw_data_table(out, c, t, &g, m.left, m.table_rows);
    } else if !c.cat.deleted && c.cat.labels {
        draw_category_labels(out, c, &g, &ax.xticks, &m, area);
    }
    draw_axis_titles(out, c, &g, area);
}

/// The axis titles, one line: the title of the vertical axis (the value axis for columns, the
/// category axis for bars) is turned 90° on the plot's left; the horizontal axis's title sits
/// under the plot.
fn draw_axis_titles(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, area: Rect) {
    let lh = c.text_size * LINE;
    // The axis drawn along the plot's left (the value axis for columns, the category axis for
    // bars) gets its title rotated there; the other axis's title sits under the plot.
    let (left, bottom) = if c.horizontal { (&c.cat, &c.val) } else { (&c.val, &c.cat) };
    // Word turns the vertical axis's title counter-clockwise (reading upwards) unless the file
    // turns it the other way or keeps it horizontal (`rot="0"`, wrapped in its own strip); any
    // other rotation would not fit the strip and is read as counter-clockwise.
    let up = if left.title_rot.is_some_and(|r| (r.rem_euclid(360.0) - 90.0).abs() < 1.0) { 90.0 } else { -90.0 };
    if !left.deleted
        && let Some(lines) = flat_title_lines(c, area.w)
    {
        let strip = (g.plot.x - area.x - GAP).max(1.0);
        let top = g.plot.y + g.plot.h / 2.0 - lines.len() as f32 * lh / 2.0;
        for (k, l) in lines.iter().enumerate() {
            let l = clip_text(l, strip, c.text_size);
            text_angle(out, Rect { x: area.x, y: top + k as f32 * lh, w: strip, h: lh }, &l, c.text_size, false, c.text_colour, 0.0);
        }
    } else if !left.deleted
        && let Some(t) = &left.title
    {
        let t = clip_text(t, g.plot.h, c.text_size);
        text_angle(out, Rect { x: area.x, y: g.plot.y, w: lh, h: g.plot.h }, &t, c.text_size, false, c.text_colour, up);
    }
    if !bottom.deleted
        && let Some(t) = &bottom.title
    {
        let t = clip_text(t, g.plot.w, c.text_size);
        text_angle(out, Rect { x: g.plot.x, y: area.bottom() - lh, w: g.plot.w, h: lh }, &t, c.text_size, false, c.text_colour, 0.0);
    }
}

/// Width of a data table's name column: keys, series names and padding.
fn table_names_w(c: &Chart, t: &DataTable, fs: f32) -> f32 {
    let keys = if t.keys { SWATCH + GAP } else { 0.0 };
    let names = c.series.iter().enumerate().map(|(j, s)| text_w(&series_name(s, j), fs)).fold(0.0, f32::max);
    keys + names + 2.0 * GAP
}

/// A data table under the plot: the categories over the plot's bands, then a row per series with
/// its key and name left of the plot and its values under the categories.
fn draw_data_table(out: &mut Vec<GraphicItem>, c: &Chart, t: &DataTable, g: &Frame, margin: f32, rows: usize) {
    let (fs, lh) = (c.text_size, c.text_size * LINE);
    let plot = g.plot;
    let band = g.band();
    // The name column keeps to the left margin `axis_margins` reserved for it.
    let names_w = table_names_w(c, t, fs).min(margin).min(plot.x.max(0.0));
    let (x0, top) = (plot.x - names_w, plot.bottom());
    let rows = rows.clamp(1, 1 + c.series.len());
    let bottom = top + rows as f32 * lh;
    for (i, cat) in c.cats.iter().enumerate() {
        let x = g.band_start(i);
        text(out, Rect { x, y: top, w: band, h: lh }, &clip_text(cat, band, fs), fs, TextAlign::Center, t.bold, t.text);
    }
    for (j, s) in c.series.iter().enumerate().take(rows - 1) {
        let y = top + (j + 1) as f32 * lh;
        let mut x = x0 + GAP;
        if t.keys {
            swatch(out, x, y + (lh - SWATCH) / 2.0, c.series_colour(j));
            x += SWATCH + GAP;
        }
        // Cut short only past the plot's edge: the column was sized to the widest name.
        let name = clip_text(&series_name(s, j), (plot.x - x).max(1.0), fs);
        text(out, Rect { x, y, w: (plot.x - x - GAP).max(1.0), h: lh }, &name, fs, TextAlign::Left, t.bold, t.text);
        for i in 0..g.n {
            let Some(v) = s.raw.get(i).copied().flatten() else { continue };
            let x = g.band_start(i);
            text(out, Rect { x, y, w: band, h: lh }, &clip_text(&value_text(s, v), band, fs), fs, TextAlign::Center, t.bold, t.text);
        }
    }
    let line = |out: &mut Vec<GraphicItem>, segs| stroke(out, segs, t.line, 0.75);
    if t.horz {
        for r in 1..rows {
            let y = top + r as f32 * lh;
            line(out, seg(if r == 1 { plot.x } else { x0 }, y, plot.right(), y));
            if r == 1 {
                line(out, seg(x0, y, plot.x, y));
            }
        }
    }
    if t.vert {
        for i in 0..=g.n {
            let x = plot.x + i as f32 * band;
            line(out, seg(x, top, x, bottom));
        }
    }
    if t.outline {
        line(out, rect_segs(plot.x, top, plot.right(), top + lh));
        line(out, rect_segs(x0, top + lh, plot.right(), bottom));
    }
}

/// Major gridlines at the value ticks.
fn draw_grid(out: &mut Vec<GraphicItem>, g: &Frame, ticks: &[(f64, String)]) {
    let plot = g.plot;
    for (t, _) in ticks {
        let p = g.val(*t);
        let segs = if g.horiz { seg(p, plot.y, p, plot.bottom()) } else { seg(plot.x, p, plot.right(), p) };
        stroke(out, segs, GRID, 0.75);
    }
}

/// Gridlines along the category axis: across the plot at each category's centre (where line and
/// area points sit, on the tick marks when the axis crosses "midCat"), at the x ticks of a
/// scatter chart.
fn draw_cat_grid(out: &mut Vec<GraphicItem>, g: &Frame, xticks: &[(f64, String)], kind: Kind) {
    let plot = g.plot;
    if kind == Kind::Scatter {
        for (t, _) in xticks {
            let p = g.scat_x(*t);
            stroke(out, seg(p, plot.y, p, plot.bottom()), GRID, 0.75);
        }
        return;
    }
    for i in 0..g.n {
        let segs = if g.horiz {
            let p = g.band_start(i) + g.band() / 2.0;
            seg(plot.x, p, plot.right(), p)
        } else {
            // Bars stay in their bands; only line and area points move onto the ticks.
            let p = if matches!(kind, Kind::Line | Kind::Area) { g.cat_x(i) } else { g.band_start(i) + g.band() / 2.0 };
            seg(p, plot.y, p, plot.bottom())
        };
        stroke(out, segs, GRID, 0.75);
    }
}

/// The category axis line, the plot area's outline and the value axis line, as the file shows them.
fn draw_axes(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame) {
    let plot = g.plot;
    if !c.cat.deleted && c.cat.line {
        // Where the category axis crosses the value axis: at zero unless the file says otherwise.
        let p = if c.kind == Kind::Scatter { if g.horiz { plot.x } else { plot.bottom() } } else { g.val(c.cat_cross(&g.vs)) };
        let segs = if g.horiz { seg(p, plot.y, p, plot.bottom()) } else { seg(plot.x, p, plot.right(), p) };
        stroke(out, segs, c.cat.line_colour.unwrap_or(AXIS), c.cat.line_w.unwrap_or(0.75));
    }
    if let Some(color) = c.plot_border {
        stroke(out, rect_segs(plot.x, plot.y, plot.right(), plot.bottom()), color, 0.75);
    }
    if !c.val.deleted && c.val.line {
        let segs = if g.horiz { seg(plot.x, plot.bottom(), plot.right(), plot.bottom()) } else { seg(plot.x, plot.y, plot.x, plot.bottom()) };
        stroke(out, segs, c.val.line_colour.unwrap_or(AXIS), c.val.line_w.unwrap_or(0.75));
    }
}

/// Value tick labels: under the plot for bars, else right-aligned in `margin` (the area's left
/// margin).
fn draw_value_labels(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, ticks: &[(f64, String)], margin: Rect) {
    let (fs, lh) = (c.text_size, c.text_size * LINE);
    for (t, s) in ticks {
        let p = g.val(*t);
        if g.horiz {
            text_at(out, p, g.plot.bottom() + GAP / 2.0, s, fs, c.val_colour(), c.val_bold());
        } else {
            text(out, Rect { x: margin.x, y: p - lh / 2.0, w: margin.w - GAP, h: lh }, s, fs, TextAlign::Right, c.val_bold(), c.val_colour());
        }
    }
}

/// Category labels: scatter x ticks under the plot, bar labels in the left margin, column
/// labels under their bands (every `m.skip`-th).
fn draw_category_labels(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, xticks: &[(f64, String)], m: &Margins, area: Rect) {
    let (fs, lh) = (c.text_size, c.text_size * LINE);
    let plot = g.plot;
    if c.kind == Kind::Scatter {
        for (t, s) in xticks {
            text_at(out, g.scat_x(*t), plot.bottom() + GAP / 2.0, s, fs, c.cat_colour(), c.cat_bold());
        }
    } else if g.horiz {
        for (i, lines) in m.wrapped.iter().enumerate() {
            let top = g.band_start(i) + g.band() / 2.0 - lines.len() as f32 * lh / 2.0;
            for (k, s) in lines.iter().enumerate() {
                let r = Rect { x: area.x + m.title, y: top + k as f32 * lh, w: m.left - m.title - GAP, h: lh };
                text(out, r, s, fs, TextAlign::Right, c.cat_bold(), c.cat_colour());
            }
        }
    } else {
        // Under the axis where it crosses (next to it), or under the plot (low). High labels,
        // which Word puts over the plot, are drawn under it too: the margins hold no room above.
        let top = match c.cat.tick_pos {
            TickPos::NextTo => g.val(c.cat_cross(&g.vs)).min(plot.bottom()),
            TickPos::Low | TickPos::High => plot.bottom(),
        };
        let band = g.band();
        for (i, lines) in m.wrapped.iter().enumerate().filter(|(i, _)| i % m.skip == 0) {
            for (k, s) in lines.iter().enumerate() {
                let y = top + GAP / 2.0 + k as f32 * lh;
                text(out, Rect { x: g.cat_x(i) - band / 2.0, y, w: band, h: lh }, s, fs, TextAlign::Center, c.cat_bold(), c.cat_colour());
            }
        }
    }
}

/// Whether data `lo..hi` is a narrow band away from zero: its span is under a sixth of the end
/// farther from zero (the mirror image for negative data).
pub(super) fn narrow(lo: f64, hi: f64) -> bool {
    if lo > 0.0 {
        (hi - lo) / hi < 1.0 / 6.0
    } else if hi < 0.0 {
        (hi - lo) / lo.abs() < 1.0 / 6.0
    } else {
        false
    }
}

/// Lowest and highest value of the plot; the series of a stacked chart type add up per
/// category, each chart type of a combination by its own grouping.
fn extent(c: &Chart) -> (f64, f64) {
    let mut pts: Vec<f64> = Vec::new();
    let mut kinds: Vec<Kind> = Vec::new();
    for s in &c.series {
        if !kinds.contains(&s.kind) {
            kinds.push(s.kind);
        }
    }
    for k in kinds {
        let of_kind = || c.series.iter().filter(move |s| s.kind == k);
        if !c.stacks(k) {
            pts.extend(of_kind().flat_map(|s| s.values.iter().flatten().copied()));
            continue;
        }
        for i in 0..c.n {
            let (mut p, mut q) = (0.0, 0.0);
            for s in of_kind() {
                match s.values.get(i).copied().flatten() {
                    Some(v) if v >= 0.0 => p += v,
                    Some(v) => q += v,
                    None => {}
                }
            }
            pts.push(p);
            pts.push(q);
        }
    }
    let lo = pts.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = pts.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if lo.is_finite() && hi.is_finite() { (lo, hi) } else { (0.0, 0.0) }
}

/// Range of scatter x values (index + 1 where missing).
fn x_extent(c: &Chart) -> (f64, f64) {
    let xs: Vec<f64> = c.series.iter().flat_map(|s| s.xs.iter().flatten().copied()).collect();
    let lo = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if lo.is_finite() && hi.is_finite() { (lo, hi) } else { (0.0, 1.0) }
}

fn bars(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, labels: &mut Vec<DataLabel>) {
    let stacked = c.stacks(Kind::Bar);
    let bar_series: Vec<(usize, &Series)> = c.series.iter().enumerate().filter(|(_, s)| s.kind == Kind::Bar).collect();
    let slots = if stacked { 1 } else { bar_series.len().max(1) };
    let ov = if stacked { 0.0 } else { c.overlap };
    let ns = slots as f32;
    let band = g.band();
    let thick = band / (ns - (ns - 1.0) * ov + c.gap);
    let group = thick * (ns - (ns - 1.0) * ov);
    let mut pos = vec![0.0f64; g.n];
    let mut neg = vec![0.0f64; g.n];
    for (k, &(j, s)) in bar_series.iter().enumerate().filter(|(_, (_, s))| has_values(s)) {
        let slot = if stacked { 0 } else { k };
        let slot = (if g.horiz { slots.saturating_sub(1 + slot) } else { slot }) as f32;
        for i in 0..g.n {
            let Some(v) = s.values.get(i).copied().flatten() else { continue };
            let (v0, v1) = if !stacked {
                (0.0, v)
            } else if v >= 0.0 {
                let Some(p) = pos.get_mut(i) else { continue };
                let b = *p;
                *p += v;
                (b, *p)
            } else {
                let Some(q) = neg.get_mut(i) else { continue };
                let b = *q;
                *q += v;
                (b, *q)
            };
            let a = g.band_start(i) + (band - group) / 2.0 + slot * thick * (1.0 - ov);
            let (p0, p1) = (g.val(v0), g.val(v1));
            let (p0, p1) = (p0.min(p1), p0.max(p1));
            let segs = if g.horiz { rect_segs(p0, a, p1, a + thick) } else { rect_segs(a, p0, a + thick, p1) };
            // A shaded series paints each column with its gradient (a point's own colour wins).
            let own_colour = s.points.get(i).is_some_and(Option::is_some);
            match &s.shading {
                Some(shading) if !s.no_fill && !own_colour => {
                    let rect = if g.horiz { [p0, a, p1 - p0, thick] } else { [a, p0, thick, p1 - p0] };
                    out.push(GraphicItem::Fill { rect, fill: shading.clone() });
                }
                _ if !s.no_fill => fill(out, segs.clone(), c.point_colour(j, i)),
                _ => {}
            }
            // An explicit outline shows without a fill too (the series' colour is the line's here).
            if let (true, true, Some(w)) = (s.line, s.no_fill, s.line_w) {
                stroke(out, segs, c.point_colour(j, i), w);
            }
            if let Some(text) = label_of(s, i) {
                let mid = a + thick / 2.0;
                let (x, y) = match (g.horiz, stacked) {
                    (true, true) => ((p0 + p1) / 2.0, mid),
                    (false, true) => (mid, (p0 + p1) / 2.0),
                    // Beyond the bar's end: right of a positive bar, above a positive column.
                    (true, false) => (if v >= 0.0 { p1 } else { p0 }, mid),
                    (false, false) => (mid, if v >= 0.0 { p0 } else { p1 }),
                };
                labels.push(DataLabel { x, y, text, inside: stacked, pos: LabelPos::Above, off: 0.0 });
            }
        }
    }
}

/// Whether a series has any point to draw.
pub(super) fn has_values(s: &Series) -> bool {
    s.values.iter().any(Option::is_some)
}

/// Line series; stacked ones are drawn at the running total of the series up to them.
fn lines(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, labels: &mut Vec<DataLabel>) {
    let stacked = c.stacks(Kind::Line);
    let mut total = vec![0.0f64; g.n];
    for (j, s) in c.series.iter().enumerate().filter(|(_, s)| s.kind == Kind::Line && has_values(s)) {
        let pts: Vec<Option<(f32, f32)>> = (0..g.n)
            .map(|i| {
                let v = s.values.get(i).copied().flatten()?;
                let y = match total.get_mut(i) {
                    Some(t) if stacked => {
                        *t += v;
                        *t
                    }
                    _ => v,
                };
                Some((g.cat_x(i), g.val(y)))
            })
            .collect();
        for (i, p) in pts.iter().enumerate() {
            if let (Some((x, y)), Some(text)) = (p, label_of(s, i)) {
                labels.push(DataLabel {
                    x: *x,
                    y: *y,
                    text,
                    inside: false,
                    pos: s.label_pos.unwrap_or(LabelPos::Right),
                    off: marker_half(s, s.markers == Some(true)) + 2.0,
                });
            }
        }
        if s.line {
            stroke(out, curve(pts.iter().copied(), s.smooth), c.series_colour(j), s.line_w.unwrap_or(LINE_W));
        }
        if s.markers == Some(true) {
            markers(out, pts.iter().flatten().copied(), c.marker_colour(j), s);
        }
    }
}

/// A series' point markers in the series' symbol and size (`colour` fills or strokes them), for
/// lines and scatter charts.
/// Half a series' marker size, points; zero when it draws no markers.
fn marker_half(s: &Series, drawn: bool) -> f32 {
    if drawn { s.marker_size.filter(|v| v.is_finite()).unwrap_or(6.0).clamp(2.0, 72.0) / 2.0 } else { 0.0 }
}

fn markers(out: &mut Vec<GraphicItem>, pts: impl Iterator<Item = (f32, f32)>, colour: Rgb, s: &Series) {
    const MARKER: f32 = 6.0;
    let size = s.marker_size.filter(|v| v.is_finite()).unwrap_or(MARKER).clamp(2.0, 72.0);
    let h = size / 2.0;
    let shape = |out: &mut Vec<GraphicItem>, rect: [f32; 4], kind: ShapeKind| {
        out.push(GraphicItem::Shape { rect, kind, fill: Some(colour), stroke: None, stroke_width: 0.0, alpha: 1.0 });
    };
    let mut strokes = Vec::new();
    for (x, y) in pts {
        match s.symbol {
            Symbol::Circle => shape(out, [x - h, y - h, size, size], ShapeKind::Ellipse),
            Symbol::Square => shape(out, [x - h, y - h, size, size], ShapeKind::Rectangle),
            Symbol::Dot => shape(out, [x - h / 2.0, y - h / 2.0, h, h], ShapeKind::Ellipse),
            Symbol::Dash => shape(out, [x - h, y - size / 6.0, size, size / 3.0], ShapeKind::Rectangle),
            Symbol::Diamond => fill(
                out,
                vec![PathSeg::Move(x, y - h), PathSeg::Line(x + h, y), PathSeg::Line(x, y + h), PathSeg::Line(x - h, y), PathSeg::Close],
                colour,
            ),
            Symbol::Triangle => {
                fill(out, vec![PathSeg::Move(x, y - h), PathSeg::Line(x + h, y + h), PathSeg::Line(x - h, y + h), PathSeg::Close], colour)
            }
            Symbol::X | Symbol::Plus | Symbol::Star => {
                if s.symbol != Symbol::Plus {
                    strokes.extend(seg(x - h, y - h, x + h, y + h));
                    strokes.extend(seg(x - h, y + h, x + h, y - h));
                }
                if s.symbol != Symbol::X {
                    strokes.extend(seg(x - h, y, x + h, y));
                    strokes.extend(seg(x, y - h, x, y + h));
                }
            }
        }
    }
    stroke(out, strokes, colour, MARKER_STROKE);
}

/// A polyline through `pts`, a smooth curve (Catmull-Rom pieces) when `smooth` and there are at
/// least three points in a row; gaps (`None`) end a run.
fn curve(pts: impl Iterator<Item = Option<(f32, f32)>>, smooth: bool) -> Vec<PathSeg> {
    if !smooth {
        return runs(pts);
    }
    let pts: Vec<Option<(f32, f32)>> = pts.collect();
    let mut segs = Vec::new();
    let mut i = 0;
    while i < pts.len() {
        let Some(first) = pts[i] else {
            i += 1;
            continue;
        };
        // Collect one run of points.
        let mut run = vec![first];
        while let Some(p) = pts.get(i + 1).copied().flatten() {
            run.push(p);
            i += 1;
        }
        i += 1;
        if run.len() < 3 {
            segs.extend(runs(run.into_iter().map(Some)));
            continue;
        }
        segs.push(PathSeg::Move(run[0].0, run[0].1));
        for k in 0..run.len() - 1 {
            // Catmull-Rom: tangent at p1 from p0 to p2 (ends mirror their own tangent).
            let p0 = run[k.max(1) - 1];
            let p1 = run[k];
            let p2 = run[k + 1];
            let p3 = run.get(k + 2).copied().unwrap_or(p2);
            let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
            let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
            segs.push(PathSeg::Cubic(c1.0, c1.1, c2.0, c2.1, p2.0, p2.1));
        }
    }
    segs
}

fn areas(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, labels: &mut Vec<DataLabel>) {
    let stacked = c.stacks(Kind::Area);
    let mut base = vec![0.0f64; g.n];
    for (j, s) in c.series.iter().enumerate().filter(|(_, s)| s.kind == Kind::Area && has_values(s)) {
        let vals = (0..g.n).map(|i| s.values.get(i).copied().flatten().unwrap_or(0.0));
        let top: Vec<f64> = base.iter().zip(vals).map(|(b, v)| b + v).collect();
        let mut segs = Vec::with_capacity(2 * g.n + 1);
        for (i, t) in top.iter().enumerate() {
            let (x, y) = (g.cat_x(i), g.val(*t));
            segs.push(if i == 0 { PathSeg::Move(x, y) } else { PathSeg::Line(x, y) });
        }
        for (i, b) in base.iter().enumerate().rev() {
            segs.push(PathSeg::Line(g.cat_x(i), g.val(*b)));
        }
        segs.push(PathSeg::Close);
        if !s.no_fill {
            fill(out, segs, c.series_colour(j));
        }
        for (i, (t, b)) in top.iter().zip(&base).enumerate() {
            if let Some(text) = label_of(s, i) {
                labels.push(DataLabel { x: g.cat_x(i), y: g.val((t + b) / 2.0), text, inside: true, pos: LabelPos::Above, off: 0.0 });
            }
        }
        if stacked {
            base = top;
        }
    }
}

/// Scatter series as lines, and as small markers when the series asks for them.
fn draw_scatter(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, labels: &mut Vec<DataLabel>) {
    for (j, s) in c.series.iter().enumerate() {
        let pts: Vec<Option<(f32, f32)>> =
            s.xs.iter()
                .zip(&s.values)
                .map(|(x, y)| match (x, y) {
                    (Some(x), Some(y)) => Some((g.scat_x(*x), g.val(*y))),
                    _ => None,
                })
                .collect();
        for (i, p) in pts.iter().enumerate() {
            if let (Some((x, y)), Some(text)) = (p, label_of(s, i)) {
                labels.push(DataLabel {
                    x: *x,
                    y: *y,
                    text,
                    inside: false,
                    pos: s.label_pos.unwrap_or(LabelPos::Right),
                    off: marker_half(s, s.markers != Some(false)) + 2.0,
                });
            }
        }
        if s.line {
            stroke(out, curve(pts.iter().copied(), s.smooth), c.series_colour(j), s.line_w.unwrap_or(LINE_W));
        }
        if s.markers != Some(false) {
            markers(out, pts.iter().flatten().copied(), c.marker_colour(j), s);
        }
    }
}
