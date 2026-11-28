use super::format::{date_text, excel_stamp, format_value, is_date_code};
use super::model::{Axis, BubbleOpts, Grouping, Kind, LabelPos, PointFmt, Pos, Series, Symbol};
use super::plot::label;
use super::{Chart, El, FONT, MAX_CELLS, MAX_POINTS, MAX_RADAR_SERIES, MAX_SERIES, MAX_SPOKES, Rgb, fill_colour, int, measure, slot};

impl Chart {
    /// The chart of a `c:chartSpace`, read from the cached data of the plot area's first
    /// supported chart type. `None` when there's no such type or it has no series (the caller then
    /// draws a placeholder, or nothing).
    pub(super) fn parse(space: &El, theme: &[Rgb]) -> Option<Chart> {
        let chart = space.child("c:chart")?;
        let plot = chart.child("c:plotArea")?;
        let ty = plot.els().find(|e| Kind::of(&e.name).is_some())?;
        let kind = Kind::of(&ty.name)?;
        let scatter = kind.xy();
        let date1904 = space.child("c:date1904").is_some_and(|d| d.attr("val").is_none_or(truthy));
        let mut series = read_series(ty, scatter, theme);
        if series.is_empty() {
            return None;
        }
        let cats = ty.children("c:ser").find_map(|s| s.child("c:cat")).map(|c| labels(Some(c), MAX_POINTS, date1904)).unwrap_or_default();
        let n = if kind == Kind::Radar {
            series.iter().map(|s| s.values.len()).max().unwrap_or(0).max(cats.len()).min(MAX_SPOKES)
        } else if matches!(kind, Kind::Pie | Kind::Doughnut) {
            series.first().map_or(0, |s| s.values.len()).max(cats.len())
        } else {
            series.iter().map(|s| s.values.len()).max().unwrap_or(0).max(cats.len())
        };
        // Counted before the cell budget drops any series: Word counts them all.
        let series_n = series.iter().enumerate().map(|(j, s)| s.idx.unwrap_or(j).saturating_add(1)).max().unwrap_or(0);
        // Every series is padded to `n` points, so keep only as many as the cell budget holds.
        if let Some(keep) = MAX_CELLS.checked_div(n) {
            series.truncate(keep.max(1));
        }
        if kind == Kind::Radar {
            series.truncate(MAX_RADAR_SERIES);
        }
        let grouping = match ty.child("c:grouping").and_then(|g| g.attr("val")) {
            Some("stacked") if matches!(kind, Kind::Bar | Kind::Area | Kind::Line) => Grouping::Stacked,
            Some("percentStacked") if matches!(kind, Kind::Bar | Kind::Area | Kind::Line) => Grouping::Percent,
            _ => Grouping::Clustered,
        };
        let mut cats = cats;
        cats.resize(n, String::new());
        let radar_style = ty.child("c:radarStyle").and_then(|r| r.attr("val"));
        let type_labels = show_values(ty.child("c:dLbls"));
        let type_fmt = label_format(ty.child("c:dLbls"));
        for s in &mut series {
            s.values.resize(n, None);
            s.xs.resize(n, None);
            s.labels = Some(s.labels.or(type_labels).unwrap_or(false));
            // Absent (`None`) inherits the chart type's; explicitly source-linked (`Some(None)`) does not.
            if s.label_fmt.is_none() {
                s.label_fmt.clone_from(&type_fmt);
            }
            if s.labels == Some(true) {
                s.raw = s.values.clone();
            }
        }
        if grouping == Grouping::Percent {
            to_percent(&mut series, n);
        }
        let (cat, val) = chart_axes(plot, scatter);
        let val_fmt = if grouping == Grouping::Percent {
            "0%".to_string()
        } else {
            val.fmt.clone().or_else(|| series.iter().find_map(|s| s.fmt.clone())).unwrap_or_else(|| "General".to_string())
        };
        let num = |n: &str| ty.child(n).and_then(|g| g.attr("val")).and_then(int);
        let can_vary = varies(kind, radar_style, series.len());
        let vary = can_vary && ty.child("c:varyColors").and_then(|v| v.attr("val")).is_none_or(truthy);
        Some(Chart {
            kind,
            horizontal: kind == Kind::Bar && ty.child("c:barDir").and_then(|d| d.attr("val")) == Some("bar"),
            grouping,
            gap: num("c:gapWidth").unwrap_or(150).clamp(0, 500) as f32 / 100.0,
            overlap: num("c:overlap").unwrap_or(0).clamp(-100, 100) as f32 / 100.0,
            hole: num("c:holeSize").unwrap_or(50).clamp(10, 90) as f32 / 100.0,
            first_angle: num("c:firstSliceAng").unwrap_or(0).clamp(0, 360) as f64,
            radar_fill: radar_style == Some("filled"),
            radar_marks: radar_style == Some("marker"),
            vary,
            point_legend: matches!(kind, Kind::Pie | Kind::Doughnut)
                || vary
                || (can_vary && series.iter().any(|s| s.points.iter().any(PointFmt::coloured))),
            n,
            series_n,
            cats,
            title: title_of(chart, &series),
            series,
            accents: (4..10).map(|i| slot(theme, i)).collect(),
            legend: legend_pos(chart),
            cat,
            val,
            text_size: text_size(space),
            val_fmt,
            date1904,
            plot_border: plot.child("c:spPr").and_then(|p| p.child("a:ln")).and_then(|l| fill_colour(l, theme)),
            bubble: (kind == Kind::Bubble).then(|| bubble_opts(ty)),
        })
    }

    pub(super) fn stacked(&self) -> bool {
        self.grouping != Grouping::Clustered
    }

    /// Automatic colour `i` of `n`, as Word picks it: accent `i % 6`, with the `n` colours in
    /// `n / 6 + 1` groups of six (so six colours already make two), the early groups shaded and
    /// the late ones tinted, the further from the middle the more (measured in Word, #5961).
    fn auto_colour(&self, i: usize, n: usize) -> Rgb {
        let accent = self.accents.get(i % 6).copied().unwrap_or(Rgb::BLACK);
        let groups = (n / 6 + 1) as f64;
        // From the middle group: < 0 for the early groups, > 0 for the late ones.
        let d = 2.0 * (i / 6) as f64 - (groups - 1.0);
        let f = 1.0 - 0.7 * d.abs() / (groups + 1.0);
        if d < 0.0 {
            shade_linear(accent, f)
        } else if d > 0.0 {
            tint_linear(accent, f)
        } else {
            accent
        }
    }

    /// Colour of series `j`: its own, else the automatic colour its `c:idx` picks (as Word does;
    /// `c:order` doesn't matter), else the one its place in the file picks.
    pub(super) fn series_colour(&self, j: usize) -> Rgb {
        let s = self.series.get(j);
        s.and_then(|s| s.color).unwrap_or_else(|| self.auto_colour(s.and_then(|s| s.idx).unwrap_or(j), self.series_n))
    }

    /// Colour of point `i` of series `j`: its `c:dPt` fill, else the varied automatic colour when
    /// the chart varies and the series has no colour of its own (Word lets a series' `c:spPr`
    /// colour win over `c:varyColors`), else the series colour.
    pub(super) fn point_colour(&self, j: usize, i: usize) -> Rgb {
        self.point(j, i).and_then(|p| p.fill).unwrap_or_else(|| self.auto_point_colour(j, i))
    }

    /// Colour of the line segment into point `i` of series `j` (line, scatter, radar): its
    /// `c:dPt` line colour, else the varied accent or the series colour. A point's fill doesn't
    /// colour a line.
    pub(super) fn line_colour(&self, j: usize, i: usize) -> Rgb {
        self.point(j, i).and_then(|p| p.line).unwrap_or_else(|| self.auto_point_colour(j, i))
    }

    /// Colour of the marker of point `i` of series `j`: its `c:dPt` marker colour, else the
    /// series' marker fill, else the varied accent when the chart varies and the series has no
    /// colour of its own, else the automatic colour of the series index (not the series'
    /// explicit line colour, as Word draws them). A point's fill doesn't colour a marker (Word).
    pub(super) fn marker_colour(&self, j: usize, i: usize) -> Rgb {
        if let Some(c) = self.point(j, i).and_then(|p| p.marker) {
            return c;
        }
        let s = self.series.get(j);
        if let Some(c) = s.and_then(|s| s.marker_color) {
            return c;
        }
        if self.vary && s.is_some_and(|s| !self.keeps_colour(s)) {
            self.auto_colour(i, self.n)
        } else {
            self.auto_colour(s.and_then(|s| s.idx).unwrap_or(j), self.series_n)
        }
    }

    /// Whether series `j`'s line is drawn a segment at a time, each in its point's line colour:
    /// the chart varies colours by point and the series has no colour of its own to keep them
    /// one, or a point has a line colour of its own. Never for a filled radar, whose points'
    /// colours Word ignores.
    pub(super) fn segmented(&self, j: usize) -> bool {
        let Some(s) = self.series.get(j) else { return false };
        !self.radar_fill && ((self.vary && !self.keeps_colour(s)) || s.points.iter().any(|p| p.line.is_some()))
    }

    /// Colour of point `i`'s legend key: the colour of what the point is drawn with, else the
    /// varied accent or the series colour. For a bar or slice that's its fill (Word outlines
    /// the key in a point's line colour but fills it as the bar); for a line, scatter or radar
    /// point, its marker or else its line colour (Word draws the key's marker and line in them,
    /// and ignores a fill).
    fn key_colour(&self, j: usize, i: usize) -> Rgb {
        let own = self.point(j, i).and_then(|p| match self.kind {
            Kind::Line | Kind::Scatter | Kind::Radar => {
                if self.draws_markers(j) {
                    p.marker.or(p.line)
                } else {
                    p.line
                }
            }
            Kind::Bar | Kind::Area | Kind::Pie | Kind::Doughnut | Kind::Bubble => p.fill,
        });
        own.unwrap_or_else(|| self.auto_point_colour(j, i))
    }

    /// The `c:dPt` colours read for point `i` of series `j` (each `None` that the file doesn't
    /// set); `None` for a series or point that doesn't exist.
    pub(super) fn point(&self, j: usize, i: usize) -> Option<&PointFmt> {
        self.series.get(j).and_then(|s| s.points.get(i))
    }

    /// A point's colour when it has none of its own: the varied accent, or the series colour.
    pub(super) fn auto_point_colour(&self, j: usize, i: usize) -> Rgb {
        if self.vary && self.series.get(j).is_some_and(|s| !self.keeps_colour(s)) { self.auto_colour(i, self.n) } else { self.series_colour(j) }
    }

    /// Whether series `s` has a colour of its own that keeps its points one colour when the chart
    /// varies (Word lets it win over `c:varyColors`): its fill for bars, areas, slices and
    /// bubbles, where an outline alone leaves the points varied; any colour it sets for lines,
    /// scatters and radars.
    fn keeps_colour(&self, s: &Series) -> bool {
        match self.kind {
            Kind::Bar | Kind::Area | Kind::Pie | Kind::Doughnut | Kind::Bubble => s.fill.is_some(),
            Kind::Line | Kind::Scatter | Kind::Radar => s.color.is_some(),
        }
    }

    /// Whether series `j` draws markers: a line series only with a marker symbol, a scatter
    /// series unless its symbol is `none`, a radar chart only in its marker style.
    pub(super) fn draws_markers(&self, j: usize) -> bool {
        let Some(s) = self.series.get(j) else { return false };
        match self.kind {
            Kind::Line => s.markers == Some(true),
            Kind::Scatter => s.markers != Some(false),
            Kind::Radar => self.radar_marks,
            Kind::Bar | Kind::Area | Kind::Pie | Kind::Doughnut | Kind::Bubble => false,
        }
    }

    /// The legend: one entry per category, in its point's colour, for pie and doughnut charts,
    /// for any chart that varies colours by point (even when a series colour keeps the points one
    /// colour, as Word does), and for a one-series chart that could vary with a point coloured on
    /// its own (#5978); else one entry per series. Points without a category name (scatter) are
    /// numbered.
    pub(super) fn legend_entries(&self) -> Vec<(String, Rgb)> {
        if self.point_legend {
            let n = if self.cats.is_empty() { self.n } else { self.cats.len() };
            return (0..n)
                .map(|i| {
                    let name = self.cats.get(i).filter(|c| !c.is_empty()).cloned().unwrap_or_else(|| (i + 1).to_string());
                    (name, self.key_colour(0, i))
                })
                .collect();
        }
        self.series
            .iter()
            .enumerate()
            .map(|(j, s)| (if s.name.is_empty() { format!("Series {}", j + 1) } else { s.name.clone() }, self.series_colour(j)))
            .collect()
    }
}

/// The series of a chart type element, within the chart's cell budget.
pub(super) fn read_series(ty: &El, scatter: bool, theme: &[Rgb]) -> Vec<Series> {
    let mut budget = MAX_CELLS;
    let mut series = Vec::new();
    for ser in ty.children("c:ser").take(MAX_SERIES) {
        let s = Series::parse(ser, scatter, theme, budget.min(MAX_POINTS));
        budget = budget.saturating_sub(s.values.len().max(s.xs.len()).max(s.sizes.as_ref().map_or(0, Vec::len)));
        series.push(s);
    }
    series
}

/// A `c:bubbleChart`'s `c:bubbleScale`, `c:sizeRepresents` and `c:showNegBubbles`.
fn bubble_opts(ty: &El) -> BubbleOpts {
    // ISO/IEC 29500-4:2016 also allows the scale as a percent ("150%"); ECMA-376 a bare number.
    let scale = ty
        .child("c:bubbleScale")
        .and_then(|b| b.attr("val"))
        .and_then(|v| int(v.trim().trim_end_matches('%')))
        .map_or(100.0, |v| v.clamp(0, 300) as f64);
    BubbleOpts {
        scale,
        by_width: ty.child("c:sizeRepresents").and_then(|s| s.attr("val")).map(str::trim) == Some("w"),
        // CT_Boolean: an element without `val` is true.
        negative: ty.child("c:showNegBubbles").is_some_and(|n| n.attr("val").is_none_or(truthy)),
    }
}

/// Values as their share of each category's total (100% stacked charts).
fn to_percent(series: &mut [Series], n: usize) {
    for i in 0..n {
        let total: f64 = series.iter().map(|s| s.values.get(i).copied().flatten().map_or(0.0, f64::abs)).sum();
        for s in series.iter_mut() {
            if let Some(Some(v)) = s.values.get_mut(i) {
                *v = if total > 0.0 { *v / total } else { 0.0 };
            }
        }
    }
}

/// The category axis (scatter: the horizontal value axis) and the value axis of a plot area.
pub(crate) fn chart_axes(plot: &El, scatter: bool) -> (Axis, Axis) {
    let vals_ax: Vec<&El> = plot.children("c:valAx").collect();
    let horiz_pos = |a: &&El| matches!(a.child("c:axPos").and_then(|p| p.attr("val")), Some("b" | "t"));
    let (val_el, x_el) = if scatter {
        let x = vals_ax.iter().copied().find(horiz_pos).or_else(|| vals_ax.first().copied());
        let y = vals_ax.iter().copied().find(|a| !horiz_pos(a)).or_else(|| vals_ax.iter().copied().find(|a| Some(*a) != x));
        (y, x)
    } else {
        (vals_ax.first().copied(), None)
    };
    let cat_el = plot.els().find(|e| matches!(e.name.as_str(), "c:catAx" | "c:dateAx"));
    (axis(if scatter { x_el } else { cat_el }), axis(val_el))
}

/// The chart's title. An empty `c:title` shows the series name (Word's automatic title of a
/// one-series chart) unless the automatic title is deleted.
pub(super) fn title_of(chart: &El, series: &[Series]) -> Option<String> {
    let auto_deleted = chart.child("c:autoTitleDeleted").and_then(|a| a.attr("val")).is_some_and(truthy);
    let auto = if !auto_deleted && series.len() == 1 { series.first().map(|s| s.name.clone()).filter(|s| !s.is_empty()) } else { None };
    chart.child("c:title").and_then(|t| title_text(t).or(auto))
}

/// Whether a `c:dLbls` shows the values (`None` when there's none, or it doesn't say).
fn show_values(d: Option<&El>) -> Option<bool> {
    let d = d?;
    if d.child("c:delete").and_then(|e| e.attr("val")).is_some_and(truthy) {
        return Some(false);
    }
    d.child("c:showVal").map(|v| v.attr("val").is_none_or(truthy))
}

/// The number format of data labels (`c:dLbls/c:numFmt`): `None` when the labels say nothing,
/// `Some(None)` when they are linked to the source (so a chart-level format does not apply),
/// `Some(Some(code))` for their own format code.
fn label_format(d: Option<&El>) -> Option<Option<String>> {
    let f = d?.child("c:numFmt")?;
    if f.attr("sourceLinked").is_some_and(truthy) {
        return Some(None);
    }
    f.attr("formatCode").filter(|f| f.chars().count() <= craft_wordmodel::chart::MAX_FMT).map(|f| Some(f.to_string()))
}

fn legend_pos(chart: &El) -> Option<Pos> {
    chart.child("c:legend").map(|l| match l.child("c:legendPos").and_then(|p| p.attr("val")).unwrap_or("r") {
        "l" => Pos::Left,
        "t" | "tr" => Pos::Top,
        "b" => Pos::Bottom,
        _ => Pos::Right,
    })
}

/// The chart's text size (its `c:txPr` default run size), points.
pub(super) fn text_size(space: &El) -> f32 {
    space
        .child("c:txPr")
        .and_then(|t| t.find("a:defRPr"))
        .and_then(|d| d.attr("sz"))
        .and_then(int)
        .map_or(FONT, |v| (v as f32 / 100.0).clamp(6.0, 24.0))
}

impl Series {
    /// One `c:ser`: its name, colours, line, values (`c:val`; for scatter and bubble charts
    /// `c:yVal`, `c:xVal` and `c:bubbleSize`), point colours and outlines from `c:dPt`, and whether
    /// it shows data labels. At most `cap` points.
    fn parse(ser: &El, scatter: bool, theme: &[Rgb], cap: usize) -> Series {
        let name = ser.child("c:tx").and_then(|t| t.find("c:v")).map(|v| label(v.text().trim())).unwrap_or_default();
        let idx = ser.child("c:idx").and_then(|e| e.attr("val")).and_then(|v| v.trim().parse::<u32>().ok()).and_then(|v| usize::try_from(v).ok());
        let sp = ser.child("c:spPr");
        let ln = sp.and_then(|p| p.child("a:ln"));
        let marker = ser.child("c:marker").and_then(|m| m.child("c:spPr")).and_then(|p| fill_colour(p, theme));
        let marker_color = marker;
        let fill = sp.and_then(|p| fill_colour(p, theme));
        let color = fill.or_else(|| ln.and_then(|l| fill_colour(l, theme))).or(marker);
        let no_fill = sp.and_then(|p| p.child("a:noFill")).is_some();
        let line_w = ln.and_then(|l| l.attr("w")).and_then(|v| measure(v, 12_700.0)).map(|w| w.clamp(0.0, 20.0));
        let line = ln.is_none_or(|l| l.child("a:noFill").is_none());
        // `c:marker/c:symbol`: which symbol each point gets (`none` for no markers). Scatter
        // series show markers unless the file says not; line series only when it says so.
        let symbol = ser.child("c:marker").and_then(|m| m.child("c:symbol")).and_then(|s| s.attr("val")).map(str::to_string);
        let markers = symbol.as_deref().map(|s| s != "none").or_else(|| scatter.then_some(true));
        let symbol = symbol.as_deref().map_or(Symbol::Circle, Symbol::parse);
        let marker_size = ser
            .child("c:marker")
            .and_then(|m| m.child("c:size"))
            .and_then(|s| s.attr("val"))
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(2.0, 72.0));
        let smooth = ser.child("c:smooth").and_then(|s| s.attr("val")).is_some_and(truthy);
        let (values, fmt) = numbers(ser.child(if scatter { "c:yVal" } else { "c:val" }), cap);
        let (mut xs, x_fmt) = if scatter { numbers(ser.child("c:xVal"), cap) } else { (Vec::new(), None) };
        if scatter && xs.is_empty() {
            xs = (1..=values.len()).map(|i| Some(i as f64)).collect();
        }
        // Only bubble series have `c:bubbleSize`. One without cached values sizes no point.
        let sizes = ser.child("c:bubbleSize").filter(|_| scatter).map(|b| numbers(Some(b), cap).0);
        let mut points = vec![PointFmt::default(); values.len()];
        // A visible line's colour (`fill_colour` is `None` for `a:noFill`).
        let outline = ln.and_then(|l| fill_colour(l, theme));
        let mut outlines = vec![outline; values.len()];
        for dpt in ser.children("c:dPt") {
            let i = dpt.child("c:idx").and_then(|e| e.attr("val")).and_then(int).and_then(|i| usize::try_from(i).ok());
            let (Some(slot), Some(own_outline)) = (i.and_then(|i| points.get_mut(i)), i.and_then(|i| outlines.get_mut(i))) else { continue };
            // A later `c:dPt` for the same point replaces only what it has, its `c:spPr` (fill and
            // line) and its `c:marker` each as a whole, as Word reads it.
            if let Some(sp) = dpt.child("c:spPr") {
                slot.fill = fill_colour(sp, theme);
                slot.no_fill = sp.child("a:noFill").is_some();
                slot.line = sp.child("a:ln").and_then(|l| fill_colour(l, theme));
                // The point's own `a:ln`, even `a:noFill`, replaces the series line (bubble
                // outlines); without one the point has the series line.
                *own_outline = sp.child("a:ln").map_or(outline, |l| fill_colour(l, theme));
            }
            if let Some(m) = dpt.child("c:marker") {
                slot.marker = m.child("c:spPr").and_then(|p| fill_colour(p, theme));
            }
        }
        let labels = show_values(ser.child("c:dLbls"));
        let label_fmt = label_format(ser.child("c:dLbls"));
        let label_pos = ser.child("c:dLbls").and_then(|d| d.child("c:dLblPos")).and_then(|p| p.attr("val")).and_then(LabelPos::parse);
        Series {
            name,
            idx,
            color,
            fill,
            line_w,
            line,
            no_fill,
            markers,
            symbol,
            marker_size,
            smooth,
            values,
            xs,
            sizes,
            points,
            outlines,
            fmt,
            x_fmt,
            labels,
            label_fmt,
            label_pos,
            marker_color,
            raw: Vec::new(),
        }
    }
}

/// Longest number-format code honoured (the limit spreadsheet apps use); longer ones are ignored.
const MAX_FORMAT_CODE: usize = 255;

/// Category labels of a `c:cat` (string or number cache), capped. Numbers show in the cache's
/// format code (dates included); text and `General` numbers as they are stored.
pub(super) fn labels(el: Option<&El>, cap: usize, date1904: bool) -> Vec<String> {
    let Some(el) = el else { return Vec::new() };
    let Some(cache) = el.find("c:strCache").or_else(|| el.find("c:numCache")).or_else(|| el.find("c:strLit")).or_else(|| el.find("c:numLit")) else {
        return Vec::new();
    };
    let code =
        cache.child("c:formatCode").map(|f| f.text()).filter(|f| !f.is_empty() && f.len() <= MAX_FORMAT_CODE && !f.eq_ignore_ascii_case("General"));
    pts(cache, cap)
        .into_iter()
        .map(|p| {
            let text = p.unwrap_or_default();
            match (code.as_deref(), text.trim().parse::<f64>()) {
                (Some(code), Ok(v)) if v.is_finite() => label(&category_text(v, code, 1.0, date1904)),
                _ => label(&text),
            }
        })
        .collect()
}

/// A number in its format code: a date or time for date codes (serial days, from 1904 when
/// `date1904`), else as [`format_value`] with tick step `step`.
pub(super) fn category_text(v: f64, code: &str, step: f64, date1904: bool) -> String {
    match excel_stamp(v, date1904) {
        Some(t) if is_date_code(code) => date_text(t, code),
        _ => format_value(v, code, step),
    }
}

/// Values of a `c:val`-style element (number cache or literal), capped; with its format code.
pub(super) fn numbers(el: Option<&El>, cap: usize) -> (Vec<Option<f64>>, Option<String>) {
    let Some(cache) = el.and_then(|e| e.find("c:numCache").or_else(|| e.find("c:numLit"))) else {
        return (Vec::new(), None);
    };
    let fmt = cache.child("c:formatCode").map(|f| f.text()).filter(|f| !f.is_empty() && f.len() <= MAX_FORMAT_CODE);
    let vals = pts(cache, cap).into_iter().map(|p| p.and_then(|t| t.trim().parse::<f64>().ok()).filter(|v| v.is_finite())).collect();
    (vals, fmt)
}

/// The `c:pt` texts of a cache by index. Indexes and counts from the file are capped.
fn pts(cache: &El, cap: usize) -> Vec<Option<String>> {
    let declared = cache.child("c:ptCount").and_then(|c| c.attr("val")).and_then(int).unwrap_or(0).clamp(0, cap as i64) as usize;
    let mut out: Vec<Option<String>> = vec![None; declared];
    for p in cache.children("c:pt") {
        let Some(i) = p.attr("idx").and_then(int).and_then(|i| usize::try_from(i).ok()) else { continue };
        if i >= cap {
            continue;
        }
        if out.len() <= i {
            out.resize(i + 1, None);
        }
        if let Some(slot) = out.get_mut(i) {
            *slot = p.child("c:v").map(|v| v.text());
        }
    }
    out
}

pub(super) fn axis(a: Option<&El>) -> Axis {
    let Some(a) = a else { return Axis { deleted: true, labels: true, line: true, ..Default::default() } };
    let scaling = a.child("c:scaling");
    let num =
        |n: &str| scaling.and_then(|s| s.child(n)).and_then(|e| e.attr("val")).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
    Axis {
        deleted: a.child("c:delete").and_then(|d| d.attr("val")).is_some_and(truthy),
        grid: a.child("c:majorGridlines").is_some(),
        line: a.child("c:spPr").and_then(|p| p.child("a:ln")).and_then(|l| l.child("a:noFill")).is_none(),
        mid: a.child("c:crossBetween").and_then(|c| c.attr("val")) == Some("midCat"),
        min: num("c:min"),
        max: num("c:max"),
        fmt: a
            .child("c:numFmt")
            .filter(|f| f.attr("sourceLinked") != Some("1"))
            .and_then(|f| f.attr("formatCode"))
            .filter(|f| f.chars().count() <= craft_wordmodel::chart::MAX_FMT)
            .map(str::to_string),
        title: a.child("c:title").and_then(title_text),
        title_rot: a
            .child("c:title")
            .and_then(|t| t.find("a:bodyPr"))
            .and_then(|b| b.attr("rot"))
            .and_then(|v| v.trim().parse::<i64>().ok())
            .map(|v| (v as f64 / 60_000.0).clamp(-360.0, 360.0) as f32),
        labels: a.child("c:tickLblPos").and_then(|p| p.attr("val")) != Some("none"),
    }
}

fn title_text(t: &El) -> Option<String> {
    let tx = t.child("c:tx")?;
    let s = match tx.child("c:rich") {
        Some(rich) => {
            let mut s = String::new();
            rich_into(rich, &mut s);
            s
        }
        None => tx.find("c:v").map(|v| v.text()).unwrap_or_default(),
    };
    let s = label(s.trim());
    (!s.is_empty()).then_some(s)
}

/// The `a:t` runs under an element, in order.
fn rich_into(e: &El, s: &mut String) {
    for c in e.els() {
        if c.name == "a:t" {
            s.push_str(&c.text());
        } else {
            rich_into(c, s);
        }
    }
}

/// Whether a chart of this kind and series count can vary colours by point (`c:varyColors`), as
/// Word draws it ([MS-OI29500] §21.2.2.227, and measured): area and filled radar charts never do;
/// pie and doughnut always can; the others only with a single series. Bubble charts keep their
/// series colours (not yet checked against Word).
fn varies(kind: Kind, radar_style: Option<&str>, series: usize) -> bool {
    match kind {
        Kind::Pie | Kind::Doughnut => true,
        Kind::Area | Kind::Bubble => false,
        Kind::Radar if radar_style == Some("filled") => false,
        Kind::Bar | Kind::Line | Kind::Scatter | Kind::Radar => series == 1,
    }
}

/// `c` darkened as Word shades an automatic chart colour: each channel × `f` in linear RGB.
fn shade_linear(c: Rgb, f: f64) -> Rgb {
    let ch = |v: u8| to_srgb(to_linear(v) * f);
    Rgb(ch(c.0), ch(c.1), ch(c.2))
}

/// `c` lightened as Word tints an automatic chart colour: each channel's distance from white × `f`
/// in linear RGB.
fn tint_linear(c: Rgb, f: f64) -> Rgb {
    let ch = |v: u8| to_srgb(1.0 - (1.0 - to_linear(v)) * f);
    Rgb(ch(c.0), ch(c.1), ch(c.2))
}

/// An sRGB channel as linear light, 0..1.
fn to_linear(v: u8) -> f64 {
    let v = f64::from(v) / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Linear light (clamped to 0..1) as an sRGB channel.
fn to_srgb(x: f64) -> u8 {
    let x = x.clamp(0.0, 1.0);
    let v = if x <= 0.003_130_8 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

fn truthy(v: &str) -> bool {
    matches!(v.trim(), "1" | "true")
}
