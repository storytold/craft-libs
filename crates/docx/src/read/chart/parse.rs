use super::format::{date_text, excel_stamp, format_value, is_date_code};
use super::model::{Axis, Crosses, DataTable, Grouping, Kind, LabelPos, Pos, Series, Symbol, TickPos};
use super::plot::Scale;
use super::plot::label;
use super::{
    AXIS, Chart, El, FONT, Fill, MAX_CELLS, MAX_POINTS, MAX_RADAR_SERIES, MAX_SERIES, MAX_SPOKES, Rgb, TEXT, TITLE_SIZE, fill_colour, int, measure,
    read_fill, slot,
};

impl Chart {
    pub(super) fn parse(space: &El, theme: &[Rgb]) -> Option<Chart> {
        let chart = space.child("c:chart")?;
        let plot = chart.child("c:plotArea")?;
        let ty = plot.els().find(|e| Kind::of(&e.name).is_some())?;
        let kind = Kind::of(&ty.name)?;
        let scatter = kind == Kind::Scatter;
        let date1904 = space.child("c:date1904").is_some_and(|d| d.attr("val").is_none_or(truthy));
        let horizontal = kind == Kind::Bar && ty.child("c:barDir").and_then(|d| d.attr("val")) == Some("bar");
        // One cell budget for the whole chart, shared by every chart type of a combination.
        let mut budget = MAX_CELLS;
        let mut series = read_series(ty, scatter, theme, &mut budget);
        // Bar spacing comes from the bar chart type, wherever it is in a combination.
        let mut bar_ty = ty;
        // The grouping of each other chart type of a combination (the first one's is `grouping`).
        let mut groupings: Vec<(Kind, Grouping)> = Vec::new();
        if matches!(kind, Kind::Bar | Kind::Line | Kind::Area) && !horizontal {
            for other in combined_types(plot, ty) {
                if let Some(k) = Kind::of(&other.name).filter(|k| *k != kind && !groupings.iter().any(|(g, _)| g == k)) {
                    groupings.push((k, grouping_of(other, k)));
                }
                // Past the series limit or the cell budget a chart type is not parsed: each would
                // cost a pass over series that are dropped.
                let room = MAX_SERIES.saturating_sub(series.len());
                if room > 0 && budget > 0 {
                    series.extend(read_series(other, false, theme, &mut budget).into_iter().take(room));
                }
                if bar_ty.name != "c:barChart" && other.name == "c:barChart" {
                    bar_ty = other;
                }
            }
        }
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
        // Every series is padded to `n` points, so keep only as many as the cell budget holds.
        if let Some(keep) = MAX_CELLS.checked_div(n) {
            series.truncate(keep.max(1));
        }
        if kind == Kind::Radar {
            series.truncate(MAX_RADAR_SERIES);
        }
        let grouping = grouping_of(ty, kind);
        let mut cats = cats;
        cats.resize(n, String::new());
        let radar_style = ty.child("c:radarStyle").and_then(|r| r.attr("val"));
        let data_table = if matches!(kind, Kind::Bar | Kind::Line | Kind::Area) && !horizontal {
            plot.child("c:dTable").map(|d| {
                let flag = |n: &str| d.child(n).is_some_and(|e| e.attr("val").is_none_or(truthy));
                DataTable {
                    horz: flag("c:showHorzBorder"),
                    vert: flag("c:showVertBorder"),
                    outline: flag("c:showOutline"),
                    keys: flag("c:showKeys"),
                    text: element_text_colour(d, theme).unwrap_or_else(|| chart_text_colour(space, theme)),
                    bold: element_bold(d).or_else(|| chart_bold(space)).unwrap_or(false),
                    line: d.child("c:spPr").and_then(|p| p.child("a:ln")).and_then(|l| fill_colour(l, theme)).unwrap_or(AXIS),
                }
            })
        } else {
            None
        };
        for s in &mut series {
            s.values.resize(n, None);
            s.xs.resize(n, None);
            // The file's values are kept for data labels and a data table (100% stacked charts
            // turn `values` into shares).
            if s.labels == Some(true) || data_table.is_some() {
                s.raw = s.values.clone();
            }
        }
        // Each 100% stacked chart type turns its own series into shares.
        for (k, g) in std::iter::once((kind, grouping)).chain(groupings.iter().copied()) {
            if g == Grouping::Percent {
                to_percent(&mut series, n, k);
            }
        }
        let (cat, val) = chart_axes(plot, scatter, theme);
        let text_colour = chart_text_colour(space, theme);
        let title_el = chart.child("c:title");
        let legend_el = chart.child("c:legend");
        let val_fmt = if grouping == Grouping::Percent {
            "0%".to_string()
        } else {
            val.fmt.clone().or_else(|| series.iter().find_map(|s| s.fmt.clone())).unwrap_or_else(|| "General".to_string())
        };
        let num = |n: &str| ty.child(n).and_then(|g| g.attr("val")).and_then(int);
        let bar_num = |n: &str| bar_ty.child(n).and_then(|g| g.attr("val")).and_then(int);
        Some(Chart {
            kind,
            horizontal,
            grouping,
            groupings,
            gap: bar_num("c:gapWidth").unwrap_or(150).clamp(0, 500) as f32 / 100.0,
            overlap: bar_num("c:overlap").unwrap_or(0).clamp(-100, 100) as f32 / 100.0,
            hole: num("c:holeSize").unwrap_or(50).clamp(10, 90) as f32 / 100.0,
            first_angle: num("c:firstSliceAng").unwrap_or(0).clamp(0, 360) as f64,
            radar_fill: radar_style == Some("filled"),
            radar_marks: radar_style == Some("marker"),
            vary: matches!(kind, Kind::Pie | Kind::Doughnut) && ty.child("c:varyColors").and_then(|v| v.attr("val")).is_none_or(truthy),
            n,
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
            plot_fill: plot.child("c:spPr").and_then(|p| own_fill(p, theme)),
            legend_box: legend_el.and_then(manual_layout),
            data_table,
            title_bold: title_el.and_then(element_bold).or_else(|| chart_bold(space)).unwrap_or(true),
            title_size: title_el.and_then(element_size).unwrap_or(TITLE_SIZE),
            legend_bold: legend_el.and_then(element_bold).or_else(|| chart_bold(space)).unwrap_or(false),
            text_bold: chart_bold(space).unwrap_or(false),
            title_colour: title_el.and_then(|t| element_text_colour(t, theme)).unwrap_or(text_colour),
            legend_colour: legend_el.and_then(|l| element_text_colour(l, theme)).unwrap_or(text_colour),
            text_colour,
        })
    }

    /// Colour of the category (scatter: x) axis labels.
    pub(super) fn cat_colour(&self) -> Rgb {
        self.cat.text.unwrap_or(self.text_colour)
    }

    /// Colour of the value axis labels.
    pub(super) fn val_colour(&self) -> Rgb {
        self.val.text.unwrap_or(self.text_colour)
    }

    /// Whether the category and value axis labels are bold.
    pub(super) fn cat_bold(&self) -> bool {
        self.cat.bold.unwrap_or(self.text_bold)
    }

    pub(super) fn val_bold(&self) -> bool {
        self.val.bold.unwrap_or(self.text_bold)
    }

    /// Where the category axis crosses the value axis, as a value within its scale `vs`.
    pub(super) fn cat_cross(&self, vs: &Scale) -> f64 {
        let (lo, hi) = (vs.lo.min(vs.hi), vs.lo.max(vs.hi));
        match self.cat.crosses {
            Crosses::AutoZero => 0.0f64.clamp(lo, hi),
            Crosses::Min => lo,
            Crosses::Max => hi,
            Crosses::At(v) => v.clamp(lo, hi),
        }
    }

    pub(super) fn stacked(&self) -> bool {
        self.grouping != Grouping::Clustered
    }

    /// Whether the series of chart type `kind` stack: each chart type of a combination stacks by
    /// its own grouping (a line before stacked columns leaves the columns stacked).
    pub(super) fn stacks(&self, kind: Kind) -> bool {
        let grouping = if kind == self.kind { Some(self.grouping) } else { self.groupings.iter().find(|(k, _)| *k == kind).map(|(_, g)| *g) };
        grouping.is_some_and(|g| g != Grouping::Clustered)
    }

    /// Whether any series is drawn as `kind`.
    pub(super) fn has(&self, kind: Kind) -> bool {
        self.series.iter().any(|s| s.kind == kind)
    }

    /// Whether the chart combines chart types (columns and a line…).
    pub(super) fn combined(&self) -> bool {
        self.series.iter().any(|s| s.kind != self.kind)
    }

    fn accent(&self, i: usize) -> Rgb {
        self.accents.get(i % 6).copied().unwrap_or(Rgb::BLACK)
    }

    /// Colour of series `j`: its own, else the accent its `c:idx` picks (as Word does; `c:order`
    /// doesn't matter), else the one its place in the file picks.
    pub(super) fn series_colour(&self, j: usize) -> Rgb {
        let s = self.series.get(j);
        s.and_then(|s| s.color).unwrap_or_else(|| self.accent(s.and_then(|s| s.idx).unwrap_or(j)))
    }

    /// Colour of series `j`'s markers: the marker's own fill, else the automatic colour of the
    /// series index (not the series' explicit line colour, as Word draws them).
    pub(super) fn marker_colour(&self, j: usize) -> Rgb {
        let s = self.series.get(j);
        s.and_then(|s| s.marker_color).unwrap_or_else(|| self.accent(s.and_then(|s| s.idx).unwrap_or(j)))
    }

    /// Colour of point `i` of series `j`: its `c:dPt` override, else the varied accent (pie) or
    /// the series colour.
    pub(super) fn point_colour(&self, j: usize, i: usize) -> Rgb {
        if let Some(Some(c)) = self.series.get(j).and_then(|s| s.points.get(i)) {
            return *c;
        }
        if self.vary { self.accent(i) } else { self.series_colour(j) }
    }

    pub(super) fn legend_entries(&self) -> Vec<(String, Rgb)> {
        if matches!(self.kind, Kind::Pie | Kind::Doughnut) {
            return self
                .cats
                .iter()
                .enumerate()
                .map(|(i, c)| (if c.is_empty() { (i + 1).to_string() } else { c.clone() }, self.point_colour(0, i)))
                .collect();
        }
        self.series.iter().enumerate().map(|(j, s)| (series_name(s, j), self.series_colour(j))).collect()
    }
}

/// A series' name, or Word's "Series n" for one without.
pub(super) fn series_name(s: &Series, j: usize) -> String {
    if s.name.is_empty() { format!("Series {}", j + 1) } else { s.name.clone() }
}

/// The series of a chart type element, within what is left of the chart's cell `budget`; parsing
/// stops once it is spent. Each series takes the chart type's kind and, where it says nothing of
/// its own, the chart type's data labels.
pub(super) fn read_series(ty: &El, scatter: bool, theme: &[Rgb], budget: &mut usize) -> Vec<Series> {
    let kind = Kind::of(&ty.name).unwrap_or(Kind::Bar);
    // A line chart shows markers on its series when it says so (`c:marker` of the chart type):
    // a series that names no symbol of its own gets its automatic marker.
    let type_markers = kind == Kind::Line && ty.child("c:marker").is_some_and(|m| m.attr("val").is_none_or(truthy));
    let type_labels = show_values(ty.child("c:dLbls"));
    let type_fmt = label_format(ty.child("c:dLbls"));
    let mut series = Vec::new();
    for ser in ty.children("c:ser").take(MAX_SERIES) {
        if *budget == 0 {
            break;
        }
        let mut s = Series::parse(ser, scatter, theme, (*budget).min(MAX_POINTS));
        *budget = budget.saturating_sub(s.values.len().max(s.xs.len()));
        s.kind = kind;
        if type_markers && s.markers.is_none() {
            s.markers = Some(true);
            s.symbol = Symbol::auto(s.idx.unwrap_or(series.len()));
        }
        s.labels = Some(s.labels.or(type_labels).unwrap_or(false));
        // Absent (`None`) inherits the chart type's; explicitly source-linked (`Some(None)`) does not.
        if s.label_fmt.is_none() {
            s.label_fmt.clone_from(&type_fmt);
        }
        series.push(s);
    }
    series
}

/// The axis ids a chart type is plotted on.
fn axis_ids(ty: &El) -> Vec<&str> {
    ty.children("c:axId").take(4).filter_map(|a| a.attr("val")).collect()
}

/// The other chart types a combination chart draws with `ty`: the column, line and area types on
/// the same axes. Types on other axes (a secondary axis) and horizontal bars are left out.
fn combined_types<'a>(plot: &'a El, ty: &El) -> Vec<&'a El> {
    let axes = axis_ids(ty);
    plot.els()
        .filter(|e| !std::ptr::eq(*e, ty))
        .filter(|e| matches!(Kind::of(&e.name), Some(Kind::Bar | Kind::Line | Kind::Area)))
        .filter(|e| e.child("c:barDir").and_then(|d| d.attr("val")) != Some("bar"))
        .filter(|e| axis_ids(e) == axes)
        .take(MAX_SERIES)
        .collect()
}

/// Values as their share of each category's total (100% stacked charts). Only the series of the
/// stacked chart type `kind` count: the other types of a combination keep their values.
/// How the series of chart type `ty` (of kind `kind`) are grouped (`c:grouping`): only columns,
/// lines and areas stack.
fn grouping_of(ty: &El, kind: Kind) -> Grouping {
    match ty.child("c:grouping").and_then(|g| g.attr("val")) {
        Some("stacked") if matches!(kind, Kind::Bar | Kind::Area | Kind::Line) => Grouping::Stacked,
        Some("percentStacked") if matches!(kind, Kind::Bar | Kind::Area | Kind::Line) => Grouping::Percent,
        _ => Grouping::Clustered,
    }
}

fn to_percent(series: &mut [Series], n: usize, kind: Kind) {
    for i in 0..n {
        let total: f64 = series.iter().filter(|s| s.kind == kind).map(|s| s.values.get(i).copied().flatten().map_or(0.0, f64::abs)).sum();
        for s in series.iter_mut().filter(|s| s.kind == kind) {
            if let Some(Some(v)) = s.values.get_mut(i) {
                *v = if total > 0.0 { *v / total } else { 0.0 };
            }
        }
    }
}

/// The category axis (scatter: the horizontal value axis) and the value axis of a plot area;
/// `theme` resolves their text colours.
pub(crate) fn chart_axes(plot: &El, scatter: bool, theme: &[Rgb]) -> (Axis, Axis) {
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
    (axis(if scatter { x_el } else { cat_el }, theme), axis(val_el, theme))
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
    fn parse(ser: &El, scatter: bool, theme: &[Rgb], cap: usize) -> Series {
        let name = ser.child("c:tx").and_then(|t| t.find("c:v")).map(|v| label(v.text().trim())).unwrap_or_default();
        let idx = ser.child("c:idx").and_then(|e| e.attr("val")).and_then(|v| v.trim().parse::<u32>().ok()).and_then(|v| usize::try_from(v).ok());
        let sp = ser.child("c:spPr");
        let ln = sp.and_then(|p| p.child("a:ln"));
        let marker = ser.child("c:marker").and_then(|m| m.child("c:spPr")).and_then(|p| fill_colour(p, theme));
        let marker_color = marker;
        let color = sp.and_then(|p| fill_colour(p, theme)).or_else(|| ln.and_then(|l| fill_colour(l, theme))).or(marker);
        let no_fill = sp.and_then(|p| p.child("a:noFill")).is_some();
        let line_w = ln.and_then(|l| l.attr("w")).and_then(|v| measure(v, 12_700.0)).map(|w| w.clamp(0.0, 20.0));
        let line = ln.is_none_or(|l| l.child("a:noFill").is_none());
        // `c:marker/c:symbol`: which symbol each point gets (`none` for no markers). Scatter
        // series show markers unless the file says not; line series only when it says so.
        let symbol = ser.child("c:marker").and_then(|m| m.child("c:symbol")).and_then(|s| s.attr("val")).map(str::to_string);
        let markers = symbol.as_deref().map(|s| s != "none").or_else(|| scatter.then_some(true));
        // `auto` is the series' automatic marker, picked by its index.
        let symbol = match symbol.as_deref() {
            Some("auto") => Symbol::auto(idx.unwrap_or(0)),
            Some(s) => Symbol::parse(s),
            None => Symbol::Circle,
        };
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
        let mut points = vec![None; values.len()];
        for dpt in ser.children("c:dPt") {
            let i = dpt.child("c:idx").and_then(|e| e.attr("val")).and_then(int).and_then(|i| usize::try_from(i).ok());
            let c = dpt.child("c:spPr").and_then(|p| fill_colour(p, theme));
            if let Some(slot) = i.and_then(|i| points.get_mut(i)) {
                *slot = c;
            }
        }
        let labels = show_values(ser.child("c:dLbls"));
        let label_fmt = label_format(ser.child("c:dLbls"));
        let label_pos = ser.child("c:dLbls").and_then(|d| d.child("c:dLblPos")).and_then(|p| p.attr("val")).and_then(LabelPos::parse);
        Series {
            name,
            idx,
            color,
            line_w,
            line,
            no_fill,
            markers,
            symbol,
            marker_size,
            smooth,
            values,
            xs,
            points,
            fmt,
            x_fmt,
            labels,
            label_fmt,
            label_pos,
            marker_color,
            raw: Vec::new(),
            kind: Kind::Bar,
            shading: sp.and_then(|p| p.child("a:gradFill")).and_then(|g| read_fill(g, theme, None)).filter(|f| matches!(f, Fill::Gradient(_))),
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

pub(super) fn axis(a: Option<&El>, theme: &[Rgb]) -> Axis {
    let Some(a) = a else { return Axis { deleted: true, labels: true, line: true, ..Default::default() } };
    let scaling = a.child("c:scaling");
    let du = a.child("c:dispUnits");
    let units = du.and_then(|d| {
        if let Some(b) = d.child("c:builtInUnit") {
            return builtin_unit(b.attr("val").unwrap_or("thousands"));
        }
        // A custom unit is any positive number; one far outside what a chart could count in is
        // taken as the nearest end of a sane range.
        let v = d.child("c:custUnit")?.attr("val")?.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0)?;
        Some((v.clamp(1e-6, 1e18), ""))
    });
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
        text: element_text_colour(a, theme),
        bold: element_bold(a),
        // Where this axis crosses the perpendicular one, as a value on that axis's scale.
        crosses: match a.child("c:crossesAt").and_then(|e| e.attr("val")).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()) {
            Some(v) => Crosses::At(v),
            None => match a.child("c:crosses").and_then(|e| e.attr("val")) {
                Some("min") => Crosses::Min,
                Some("max") => Crosses::Max,
                _ => Crosses::AutoZero,
            },
        },
        tick_pos: match a.child("c:tickLblPos").and_then(|e| e.attr("val")) {
            Some("low") => TickPos::Low,
            Some("high") => TickPos::High,
            _ => TickPos::NextTo,
        },
        line_colour: a.child("c:spPr").and_then(|p| p.child("a:ln")).and_then(|l| fill_colour(l, theme)),
        line_w: a
            .child("c:spPr")
            .and_then(|p| p.child("a:ln"))
            .and_then(|l| l.attr("w"))
            .and_then(|v| measure(v, 12_700.0))
            .map(|w| w.clamp(0.0, 20.0)),
        unit: units.map(|(u, _)| u),
        unit_label: units.and_then(|(_, name)| {
            let l = du?.child("c:dispUnitsLbl")?;
            Some((title_text(l).unwrap_or_else(|| name.to_string()), element_text_colour(l, theme), element_bold(l).unwrap_or(false)))
        }),
    }
}

/// A chart element's hand-set place (`c:layout/c:manualLayout` in edge mode): x, y, width and
/// height as shares of the chart (0 when the file gives none). Other modes (`factor`, a shift
/// from the automatic place) are left to the automatic layout.
fn manual_layout(e: &El) -> Option<[f32; 4]> {
    let m = e.child("c:layout")?.child("c:manualLayout")?;
    let edge = |n: &str| m.child(n).and_then(|e| e.attr("val")) == Some("edge");
    if !edge("c:xMode") || !edge("c:yMode") {
        return None;
    }
    let share = |n: &str| {
        m.child(n).and_then(|e| e.attr("val")).and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0))
    };
    Some([share("c:x")?, share("c:y")?, share("c:w").unwrap_or(0.0), share("c:h").unwrap_or(0.0)])
}

/// A built-in display unit (`c:builtInUnit`): what it divides by, and its label.
fn builtin_unit(name: &str) -> Option<(f64, &'static str)> {
    Some(match name {
        "hundreds" => (1e2, "Hundreds"),
        "thousands" => (1e3, "Thousands"),
        "tenThousands" => (1e4, "10000"),
        "hundredThousands" => (1e5, "100000"),
        "millions" => (1e6, "Millions"),
        "tenMillions" => (1e7, "10000000"),
        "hundredMillions" => (1e8, "100000000"),
        "billions" => (1e9, "Billions"),
        "trillions" => (1e12, "Trillions"),
        _ => return None,
    })
}

/// A chart element's text size, points, when its rich text's first run, its default run
/// properties or its `c:txPr` state one (`sz`, hundredths of a point).
pub(super) fn element_size(e: &El) -> Option<f32> {
    let sz = |r: &El| r.attr("sz").and_then(int).map(|v| (v as f32 / 100.0).clamp(6.0, 72.0));
    let rich = e.child("c:tx").and_then(|t| t.child("c:rich"));
    rich.and_then(|r| r.find("a:rPr"))
        .and_then(sz)
        .or_else(|| rich.and_then(|r| r.find("a:defRPr")).and_then(sz))
        .or_else(|| e.child("c:txPr").and_then(|t| t.find("a:defRPr")).and_then(sz))
}

/// Whether a chart element's text is bold, when its rich text's first run, its default run
/// properties or its `c:txPr` say (`b`).
pub(super) fn element_bold(e: &El) -> Option<bool> {
    let b = |r: &El| r.attr("b").map(truthy);
    let rich = e.child("c:tx").and_then(|t| t.child("c:rich"));
    rich.and_then(|r| r.find("a:rPr"))
        .and_then(b)
        .or_else(|| rich.and_then(|r| r.find("a:defRPr")).and_then(b))
        .or_else(|| e.child("c:txPr").and_then(|t| t.find("a:defRPr")).and_then(b))
}

/// The text colour a chart element (title, legend, axis) states for itself: its rich text's
/// first run, else the default run properties of its rich text or `c:txPr`.
pub(super) fn element_text_colour(e: &El, theme: &[Rgb]) -> Option<Rgb> {
    let rich = e.child("c:tx").and_then(|t| t.child("c:rich"));
    rich.and_then(|r| r.find("a:rPr"))
        .and_then(|r| fill_colour(r, theme))
        .or_else(|| rich.and_then(|r| run_defaults_colour(r, theme)))
        .or_else(|| e.child("c:txPr").and_then(|t| run_defaults_colour(t, theme)))
}

/// The colour of a text body's default run properties (`a:defRPr`).
fn run_defaults_colour(body: &El, theme: &[Rgb]) -> Option<Rgb> {
    body.find("a:defRPr").and_then(|d| fill_colour(d, theme))
}

/// Whether the chart's own text properties (`c:txPr`) make its text bold.
pub(super) fn chart_bold(space: &El) -> Option<bool> {
    space.child("c:txPr").and_then(|t| t.find("a:defRPr")).and_then(|d| d.attr("b")).map(truthy)
}

/// The fill a `c:spPr` states for its area: solid, pattern (its foreground) or gradient.
pub(super) fn own_fill(sp: &El, theme: &[Rgb]) -> Option<Fill> {
    let f = sp.els().find(|e| matches!(e.name.as_str(), "a:solidFill" | "a:gradFill" | "a:pattFill"))?;
    read_fill(f, theme, None)
}

/// The chart's own text colour (its `c:txPr`), the grey Word's default style uses without one.
pub(super) fn chart_text_colour(space: &El, theme: &[Rgb]) -> Rgb {
    space.child("c:txPr").and_then(|t| run_defaults_colour(t, theme)).unwrap_or(TEXT)
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

fn truthy(v: &str) -> bool {
    matches!(v.trim(), "1" | "true")
}
