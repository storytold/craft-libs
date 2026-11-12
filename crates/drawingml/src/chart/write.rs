//! Chart parts: a `c:chartSpace` with cached categories and values.
//! An unchanged chart also keeps the embedded workbook and the other parts it points at.

use crate::chart::{Chart, ChartType, ErrorBars, Series, Trendline, TrendlineType};
use crate::style::Fill;

use crate::ns::{NS_A, NS_C, NS_R};

/// The Office 2016 chart namespace.
pub const NS_CX: &str = "http://schemas.microsoft.com/office/drawing/2014/chartex";
use craft_xml::{El, Local, Node, Writer};

use crate::write as dml;
use crate::xmlx::{A, TagExt};

/// Column letter for a 0-based index (A, B, … Z, AA…).
fn col(i: usize) -> String {
    let mut n = i + 1;
    let mut s = String::new();
    while n > 0 && s.len() < 4 {
        let r = (n - 1) % 26;
        s.insert(0, (b'A' + r as u8) as char);
        n = (n - 1) / 26;
    }
    s
}

fn fmt_num(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
}

fn simple_fill(w: &mut Writer, f: &Fill) {
    match f {
        Fill::None => w.empty("a:noFill", &[]),
        Fill::Solid { color } => dml::solid(w, color),
        Fill::Pattern(p) => {
            w.tag("a:pattFill").attr("prst", if p.preset.is_empty() { "pct5" } else { &p.preset }).open();
            w.open("a:fgClr", &[]);
            dml::color(w, &p.fg);
            w.close();
            w.open("a:bgClr", &[]);
            dml::color(w, &p.bg);
            w.close();
            w.close();
        }
        Fill::Gradient(_) | Fill::Picture(_) | Fill::Group | Fill::Background => {
            if let Fill::Gradient(g) = f
                && let Some(s) = g.sorted_stops().first()
            {
                dml::solid(w, &s.color);
            }
        }
    }
}

fn sp_pr(w: &mut Writer, fill: Option<&Fill>, line: Option<&crate::style::Line>) {
    if fill.is_none() && line.is_none() {
        return;
    }
    w.open("c:spPr", &[]);
    if let Some(f) = fill {
        simple_fill(w, f);
    }
    if let Some(l) = line {
        w.tag("a:ln").attr_opt("w", l.width.map(dml::emu_pos)).open();
        if let Some(f) = &l.fill {
            simple_fill(w, f);
        }
        w.close();
    }
    w.close();
}

/// `a:defRPr` carrying the chart's text colour, where PowerPoint keeps it: the paragraph
/// defaults of a title's rich text and of every `c:txPr`.
fn def_rpr(w: &mut Writer, color: Option<&crate::style::ColorRef>) {
    match color {
        Some(c) => {
            w.open("a:defRPr", &[]);
            dml::solid(w, c);
            w.close();
        }
        None => w.empty("a:defRPr", &[]),
    }
}

/// `c:txPr` with the chart's text colour (`Chart::text_color`) as the default run properties, on
/// the axes, the legend, the data labels and the chart itself, as PowerPoint writes it. Nothing
/// without a colour: the theme's default then applies on reading.
fn tx_pr(w: &mut Writer, c: &Chart) {
    let Some(color) = &c.text_color else { return };
    w.open("c:txPr", &[]);
    w.empty("a:bodyPr", &[]);
    w.empty("a:lstStyle", &[]);
    w.open("a:p", &[]);
    w.open("a:pPr", &[]);
    def_rpr(w, Some(color));
    w.close();
    w.tag("a:endParaRPr").attr("lang", "en-US").empty();
    w.close();
    w.close();
}

fn rich(w: &mut Writer, text: &str, color: Option<&crate::style::ColorRef>) {
    w.open("c:tx", &[]);
    w.open("c:rich", &[]);
    w.empty("a:bodyPr", &[]);
    w.empty("a:lstStyle", &[]);
    for line in text.split('\n') {
        w.open("a:p", &[]);
        if color.is_some() {
            w.open("a:pPr", &[]);
            def_rpr(w, color);
            w.close();
        }
        w.open("a:r", &[]);
        if let Some(c) = color {
            w.tag("a:rPr").attr("lang", "en-US").open();
            dml::solid(w, c);
            w.close();
        }
        w.leaf("a:t", &[], line);
        w.close();
        w.close();
    }
    w.close();
    w.close();
}

/// A title (the chart's or an axis's) in the chart's text colour.
fn title(w: &mut Writer, text: &str, c: &Chart) {
    w.open("c:title", &[]);
    rich(w, text, c.text_color.as_ref());
    w.tag("c:overlay").attr("val", 0).empty();
    w.close();
}

/// Points one cache writes. Excel and PowerPoint hold 32,000 points per series, so this is far
/// above any chart they make; a longer vector is cut here with a warning, not refused.
const MAX_CACHE_POINTS: usize = 1_000_000;

/// `vals` cut to [`MAX_CACHE_POINTS`].
fn capped<T>(vals: &[T]) -> &[T] {
    if vals.len() > MAX_CACHE_POINTS {
        log::warn!("pptx: chart data cache cut from {} to {MAX_CACHE_POINTS} points", vals.len());
    }
    &vals[..vals.len().min(MAX_CACHE_POINTS)]
}

fn str_cache(w: &mut Writer, f: &str, vals: &[String]) {
    let vals = capped(vals);
    w.open("c:strRef", &[]);
    w.leaf("c:f", &[], f);
    w.open("c:strCache", &[]);
    w.tag("c:ptCount").attr("val", vals.len()).empty();
    for (i, v) in vals.iter().enumerate() {
        w.tag("c:pt").attr("idx", i).open();
        w.leaf("c:v", &[], v);
        w.close();
    }
    w.close();
    w.close();
}

fn num_cache(w: &mut Writer, f: &str, vals: &[Option<f64>]) {
    let vals = capped(vals);
    w.open("c:numRef", &[]);
    w.leaf("c:f", &[], f);
    w.open("c:numCache", &[]);
    w.leaf("c:formatCode", &[], "General");
    w.tag("c:ptCount").attr("val", vals.len()).empty();
    for (i, v) in vals.iter().enumerate() {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            w.tag("c:pt").attr("idx", i).open();
            w.leaf("c:v", &[], &fmt_num(v));
            w.close();
        }
    }
    w.close();
    w.close();
}

/// Numbers written into the chart itself (`c:numLit`), with no worksheet range behind them.
fn num_lit(w: &mut Writer, vals: &[Option<f64>]) {
    let vals = capped(vals);
    w.open("c:numLit", &[]);
    w.tag("c:ptCount").attr("val", vals.len()).empty();
    for (i, v) in vals.iter().enumerate() {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            w.tag("c:pt").attr("idx", i).open();
            w.leaf("c:v", &[], &fmt_num(v));
            w.close();
        }
    }
    w.close();
}

#[derive(Clone, Copy, PartialEq)]
enum Group {
    Bar,
    Line,
    Area,
    Pie,
    Doughnut,
    Scatter,
    Radar,
    Bubble,
    Stock,
}

fn group_of(k: ChartType) -> Group {
    match k {
        ChartType::Line | ChartType::LineMarkers | ChartType::StackedLine => Group::Line,
        ChartType::Stock => Group::Stock,
        ChartType::Area | ChartType::StackedArea | ChartType::Surface => Group::Area,
        ChartType::Pie => Group::Pie,
        ChartType::Doughnut | ChartType::Sunburst => Group::Doughnut,
        ChartType::Scatter => Group::Scatter,
        ChartType::Bubble => Group::Bubble,
        ChartType::Radar | ChartType::FilledRadar => Group::Radar,
        _ => Group::Bar,
    }
}

fn error_bars(w: &mut Writer, e: &ErrorBars, g: Group) {
    let word = |v: &str, ok: &[&'static str], d: &'static str| ok.iter().copied().find(|o| *o == v).unwrap_or(d);
    w.open("c:errBars", &[]);
    if matches!(g, Group::Scatter | Group::Bubble) {
        w.tag("c:errDir").attr("val", "y").empty();
    }
    w.tag("c:errBarType").attr("val", word(&e.bar_type, &["both", "plus", "minus"], "both")).empty();
    let vt = word(&e.val_type, &["fixedVal", "percentage", "stdDev", "stdErr", "cust"], "fixedVal");
    w.tag("c:errValType").attr("val", vt).empty();
    w.tag("c:noEndCap").attr("val", if e.end_cap { 0 } else { 1 }).empty();
    if vt == "cust" {
        w.open("c:plus", &[]);
        num_lit(w, &e.plus);
        w.close();
        w.open("c:minus", &[]);
        num_lit(w, &e.minus);
        w.close();
    } else if e.value.is_finite() {
        w.tag("c:val").attr("val", fmt_num(e.value)).empty();
    }
    w.close();
}

/// Per-point fills of pie, doughnut and bar series.
fn point_fills(w: &mut Writer, s: &Series, g: Group) {
    if !matches!(g, Group::Pie | Group::Doughnut | Group::Bar) {
        return;
    }
    for (idx, f) in s.point_fills.iter().take(1000) {
        w.open("c:dPt", &[]);
        w.tag("c:idx").attr("val", idx).empty();
        if g == Group::Bar {
            w.tag("c:invertIfNegative").attr("val", 0).empty();
        }
        if g != Group::Bar {
            w.tag("c:bubble3D").attr("val", 0).empty();
        }
        sp_pr(w, Some(f), None);
        w.close();
    }
}

/// Data points (categories and values counted per series) one chart group writes to its caches.
const MAX_EXPORT_POINTS: usize = 4_000_000;

fn series(w: &mut Writer, c: &Chart, s: &Series, i: usize, kind: ChartType, g: Group) {
    w.open("c:ser", &[]);
    w.tag("c:idx").attr("val", i).empty();
    w.tag("c:order").attr("val", i).empty();
    w.open("c:tx", &[]);
    let r = &s.refs;
    let or = |given: &Option<String>, made: String| given.clone().unwrap_or(made);
    str_cache(w, &or(&r.name, format!("Sheet1!${}$1", col(i + 1))), std::slice::from_ref(&s.name));
    w.close();
    if g == Group::Stock && s.line.is_none() {
        // A stock chart's series are points joined by the high-low lines, not lines of their own.
        w.open("c:spPr", &[]);
        w.tag("a:ln").attr("w", 19050).open();
        w.empty("a:noFill", &[]);
        w.close();
        w.close();
    } else {
        sp_pr(w, s.fill.as_ref(), s.line.as_ref());
    }
    if g == Group::Stock {
        // High-low-close: the close is a dash on the high-low line; with up/down bars (or a set
        // marker) the series keep their own.
        let close = !c.up_down_bars && i + 1 == c.series.len() && c.series.len() == 3;
        let sym = s.marker.as_deref().filter(|m| !m.is_empty() && m.chars().all(|ch| ch.is_ascii_alphanumeric())).unwrap_or(if close {
            "dash"
        } else {
            "none"
        });
        w.open("c:marker", &[]);
        w.tag("c:symbol").attr("val", sym).empty();
        if sym != "none" {
            w.tag("c:size").attr("val", 7).empty();
        }
        w.close();
    }
    if g == Group::Bar {
        w.tag("c:invertIfNegative").attr("val", 0).empty();
    }
    if matches!(g, Group::Line | Group::Scatter | Group::Radar) {
        let sym = match (&s.marker, kind) {
            (Some(m), _) if !m.is_empty() => Some(m.as_str()),
            (_, ChartType::LineMarkers) => None,
            (_, ChartType::Scatter) => None,
            _ => Some("none"),
        };
        if let Some(sym) = sym.filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric())) {
            w.open("c:marker", &[]);
            w.tag("c:symbol").attr("val", sym).empty();
            w.close();
        }
    }
    point_fills(w, s, g);
    if let Some(d) = s.data_labels {
        data_labels(w, c, d);
    }
    // After labels, before the category and value caches (ECMA-376 series content order).
    if matches!(g, Group::Bar | Group::Line | Group::Area | Group::Scatter) {
        for t in s.trendlines.iter().take(16) {
            trendline(w, t);
        }
    }
    let n = c.categories.len().max(s.values.len()).min(MAX_CACHE_POINTS);
    let last = n + 1;
    if let Some(e) = &s.error_bars {
        error_bars(w, e, g);
    }
    if matches!(g, Group::Scatter | Group::Bubble) {
        let xs: Vec<Option<f64>> = if s.x_values.is_empty() { (1..=s.values.len()).map(|v| Some(v as f64)).collect() } else { s.x_values.clone() };
        w.open("c:xVal", &[]);
        num_cache(w, &or(&r.categories, format!("Sheet1!$A$2:$A${last}")), &xs);
        w.close();
        w.open("c:yVal", &[]);
        num_cache(w, &or(&r.values, format!("Sheet1!${0}$2:${0}${last}", col(i + 1))), &s.values);
        w.close();
        if g == Group::Bubble {
            let sizes: Vec<Option<f64>> = if s.sizes.is_empty() { s.values.iter().map(|_| Some(1.0)).collect() } else { s.sizes.clone() };
            w.open("c:bubbleSize", &[]);
            num_cache(w, &or(&r.sizes, format!("Sheet1!${0}$2:${0}${last}", col(i + 2))), &sizes);
            w.close();
            w.tag("c:bubble3D").attr("val", 0).empty();
        } else {
            w.tag("c:smooth").attr("val", if s.smooth { 1 } else { 0 }).empty();
        }
    } else {
        w.open("c:cat", &[]);
        str_cache(w, &or(&r.categories, format!("Sheet1!$A$2:$A${last}")), &c.categories);
        w.close();
        w.open("c:val", &[]);
        num_cache(w, &or(&r.values, format!("Sheet1!${0}$2:${0}${last}", col(i + 1))), &s.values);
        w.close();
        if g == Group::Line {
            w.tag("c:smooth").attr("val", if s.smooth { 1 } else { 0 }).empty();
        }
    }
    w.close();
}

fn trend_num(v: Option<f64>) -> Option<f64> {
    v.filter(|v| v.is_finite() && v.abs() <= 1.0e12)
}

fn trend_name(s: &str) -> Option<String> {
    let cleaned: String =
        s.chars().filter(|c| matches!(c, '\t' | '\n' | '\r') || (*c >= ' ' && *c != '\u{FFFE}' && *c != '\u{FFFF}')).take(255).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() { None } else { Some(cleaned.to_string()) }
}

fn trendline(w: &mut Writer, t: &Trendline) {
    w.open("c:trendline", &[]);
    if let Some(name) = t.name.as_deref().and_then(trend_name) {
        w.leaf("c:name", &[], &name);
    }
    w.tag("c:trendlineType").attr("val", t.kind.xml()).empty();
    if t.kind == TrendlineType::Poly
        && let Some(o) = t.order
    {
        w.tag("c:order").attr("val", o.clamp(2, 6)).empty();
    }
    if t.kind == TrendlineType::MovingAvg
        && let Some(p) = t.period
    {
        w.tag("c:period").attr("val", p.clamp(2, 255)).empty();
    }
    if let Some(v) = trend_num(t.forward) {
        w.tag("c:forward").attr("val", fmt_num(v)).empty();
    }
    if let Some(v) = trend_num(t.backward) {
        w.tag("c:backward").attr("val", fmt_num(v)).empty();
    }
    if let Some(v) = trend_num(t.intercept) {
        w.tag("c:intercept").attr("val", fmt_num(v)).empty();
    }
    if t.disp_r_sqr {
        w.tag("c:dispRSqr").attr("val", 1).empty();
    }
    if t.disp_eq {
        w.tag("c:dispEq").attr("val", 1).empty();
    }
    w.close();
}

fn data_labels(w: &mut Writer, c: &Chart, show: bool) {
    w.open("c:dLbls", &[]);
    tx_pr(w, c);
    for (n, v) in [
        ("c:showLegendKey", false),
        ("c:showVal", show),
        ("c:showCatName", false),
        ("c:showSerName", false),
        ("c:showPercent", false),
        ("c:showBubbleSize", false),
    ] {
        w.tag(n).attr("val", if v { 1 } else { 0 }).empty();
    }
    w.close();
}

const AX_CAT: u32 = 50_001;
const AX_VAL: u32 = 50_002;

fn axes(w: &mut Writer, c: &Chart, g: Group) {
    let horizontal = c.kind.horizontal() && g == Group::Bar;
    let (cat_pos, val_pos) = if horizontal { ("l", "b") } else { ("b", "l") };
    if g == Group::Scatter || g == Group::Bubble {
        w.open("c:valAx", &[]);
        w.tag("c:axId").attr("val", AX_CAT).empty();
        w.open("c:scaling", &[]);
        w.tag("c:orientation").attr("val", "minMax").empty();
        w.close();
        w.tag("c:delete").attr("val", 0).empty();
        w.tag("c:axPos").attr("val", "b").empty();
        if let Some(t) = &c.axis_titles.0 {
            title(w, t, c);
        }
        w.tag("c:numFmt").attr("formatCode", "General").attr("sourceLinked", 1).empty();
        w.tag("c:majorTickMark").attr("val", "out").empty();
        w.tag("c:minorTickMark").attr("val", "none").empty();
        w.tag("c:tickLblPos").attr("val", "nextTo").empty();
        tx_pr(w, c);
        w.tag("c:crossAx").attr("val", AX_VAL).empty();
        w.tag("c:crosses").attr("val", "autoZero").empty();
        w.tag("c:crossBetween").attr("val", "midCat").empty();
        w.close();
    } else {
        w.open("c:catAx", &[]);
        w.tag("c:axId").attr("val", AX_CAT).empty();
        w.open("c:scaling", &[]);
        w.tag("c:orientation").attr("val", if c.cat_reversed { "maxMin" } else { "minMax" }).empty();
        w.close();
        w.tag("c:delete").attr("val", 0).empty();
        w.tag("c:axPos").attr("val", cat_pos).empty();
        if let Some(t) = &c.axis_titles.0 {
            title(w, t, c);
        }
        w.tag("c:numFmt").attr("formatCode", "General").attr("sourceLinked", 1).empty();
        w.tag("c:majorTickMark").attr("val", "none").empty();
        w.tag("c:minorTickMark").attr("val", "none").empty();
        w.tag("c:tickLblPos").attr("val", "nextTo").empty();
        tx_pr(w, c);
        w.tag("c:crossAx").attr("val", AX_VAL).empty();
        w.tag("c:crosses").attr("val", "autoZero").empty();
        w.tag("c:auto").attr("val", 1).empty();
        w.tag("c:lblAlgn").attr("val", "ctr").empty();
        w.tag("c:lblOffset").attr("val", 100).empty();
        w.tag("c:noMultiLvlLbl").attr("val", 0).empty();
        w.close();
    }
    w.open("c:valAx", &[]);
    w.tag("c:axId").attr("val", AX_VAL).empty();
    w.open("c:scaling", &[]);
    if let Some(b) = c.y_log_base {
        w.tag("c:logBase").attr("val", fmt_num(b)).empty();
    }
    w.tag("c:orientation").attr("val", "minMax").empty();
    if let Some(m) = c.y_max.filter(|v| v.is_finite()) {
        w.tag("c:max").attr("val", fmt_num(m)).empty();
    }
    if let Some(m) = c.y_min.filter(|v| v.is_finite()) {
        w.tag("c:min").attr("val", fmt_num(m)).empty();
    }
    w.close();
    w.tag("c:delete").attr("val", 0).empty();
    w.tag("c:axPos").attr("val", val_pos).empty();
    if c.gridlines {
        w.empty("c:majorGridlines", &[]);
    }
    if c.minor_gridlines {
        w.empty("c:minorGridlines", &[]);
    }
    if let Some(t) = &c.axis_titles.1 {
        title(w, t, c);
    }
    match c.y_format.as_deref().filter(|f| !f.is_empty()) {
        Some(f) => w.tag("c:numFmt").attr("formatCode", f).attr("sourceLinked", 0).empty(),
        None => w.tag("c:numFmt").attr("formatCode", if c.kind.percent() { "0%" } else { "General" }).attr("sourceLinked", 1).empty(),
    }
    w.tag("c:majorTickMark").attr("val", "none").empty();
    w.tag("c:minorTickMark").attr("val", "none").empty();
    w.tag("c:tickLblPos").attr("val", "nextTo").empty();
    tx_pr(w, c);
    w.tag("c:crossAx").attr("val", AX_CAT).empty();
    w.tag("c:crosses").attr("val", "autoZero").empty();
    w.tag("c:crossBetween").attr("val", if g == Group::Area || g == Group::Scatter { "midCat" } else { "between" }).empty();
    w.close();
}

/// Points `series` writes into its caches for group `g`, synthesized vectors included.
fn series_points(c: &Chart, s: &Series, g: Group) -> usize {
    let v = s.values.len();
    if matches!(g, Group::Scatter | Group::Bubble) {
        let xs = if s.x_values.is_empty() { v } else { s.x_values.len() };
        let sizes = if g != Group::Bubble {
            0
        } else if s.sizes.is_empty() {
            v
        } else {
            s.sizes.len()
        };
        xs.saturating_add(v).saturating_add(sizes)
    } else {
        c.categories.len().saturating_add(v)
    }
}

fn plot_group(w: &mut Writer, c: &Chart, kind: ChartType, list: &[(usize, &Series)]) {
    let g = group_of(kind);
    let grouping = if kind.percent() {
        "percentStacked"
    } else if kind.stacked() {
        "stacked"
    } else if matches!(g, Group::Bar) {
        "clustered"
    } else {
        "standard"
    };
    let vary = if c.vary_colors { 1 } else { 0 };
    let tag = match g {
        Group::Bar => "c:barChart",
        Group::Line => "c:lineChart",
        Group::Area => "c:areaChart",
        Group::Pie => "c:pieChart",
        Group::Doughnut => "c:doughnutChart",
        Group::Scatter => "c:scatterChart",
        Group::Radar => "c:radarChart",
        Group::Bubble => "c:bubbleChart",
        Group::Stock => "c:stockChart",
    };
    w.open(tag, &[]);
    match g {
        Group::Bar => {
            w.tag("c:barDir").attr("val", if kind.horizontal() { "bar" } else { "col" }).empty();
            w.tag("c:grouping").attr("val", grouping).empty();
        }
        Group::Line | Group::Area => w.tag("c:grouping").attr("val", grouping).empty(),
        Group::Scatter => w.tag("c:scatterStyle").attr("val", "lineMarker").empty(),
        Group::Radar => w.tag("c:radarStyle").attr("val", if kind == ChartType::FilledRadar { "filled" } else { "marker" }).empty(),
        _ => {}
    }
    // `c:stockChart` has no `c:varyColors`.
    if g != Group::Stock {
        w.tag("c:varyColors").attr("val", vary).empty();
    }
    // Each series repeats the categories in its own cache, so the output grows with series times
    // categories: stop adding series once a group's caches pass the budget (the first is always written).
    // The first series is always written; each of its caches is cut at MAX_CACHE_POINTS.
    let mut left = MAX_EXPORT_POINTS;
    for (n, (i, s)) in list.iter().enumerate() {
        let cost = series_points(c, s, g);
        if n > 0 && cost > left {
            log::warn!("pptx: chart export stopped after {n} series; the data caches passed {MAX_EXPORT_POINTS} points");
            break;
        }
        left = left.saturating_sub(cost);
        series(w, c, s, *i, kind, g);
    }
    if c.data_labels {
        data_labels(w, c, true);
    }
    match g {
        Group::Bar => {
            w.tag("c:gapWidth").attr("val", (c.gap_width.clamp(0.0, 5.0) * 100.0).round() as i64).empty();
            let ov = if kind.stacked() { 100 } else { (c.overlap.clamp(-1.0, 1.0) * 100.0).round() as i64 };
            if ov != 0 {
                w.tag("c:overlap").attr("val", ov).empty();
            }
        }
        Group::Line => w.tag("c:marker").attr("val", 1).empty(),
        Group::Stock => {
            w.open("c:hiLowLines", &[]);
            w.open("c:spPr", &[]);
            w.tag("a:ln").attr("w", 9525).open();
            w.open("a:solidFill", &[]);
            w.tag("a:schemeClr").attr("val", "tx1").empty();
            w.close();
            w.close();
            w.close();
            w.close();
            if c.up_down_bars {
                w.open("c:upDownBars", &[]);
                w.tag("c:gapWidth").attr("val", 150).empty();
                w.empty("c:upBars", &[]);
                w.empty("c:downBars", &[]);
                w.close();
            }
        }
        Group::Pie => w.tag("c:firstSliceAng").attr("val", 0).empty(),
        Group::Doughnut => {
            w.tag("c:firstSliceAng").attr("val", 0).empty();
            w.tag("c:holeSize").attr("val", (c.hole_size.clamp(0.1, 0.9) * 100.0).round() as i64).empty();
        }
        _ => {}
    }
    if !matches!(g, Group::Pie | Group::Doughnut) {
        w.tag("c:axId").attr("val", AX_CAT).empty();
        w.tag("c:axId").attr("val", AX_VAL).empty();
    }
    w.close();
}

/// The `chartEx` part of an Office 2016 chart (waterfall, funnel…): the kept part when the chart
/// is unchanged since it was read, else one written from the model. `None` for other chart types.
#[cfg(test)]
pub fn chartex_xml(c: &Chart) -> Option<Vec<u8>> {
    chartex_part(c).map(|(x, _)| x)
}

/// [`chartex_xml`], and whether it is the original part (so its kept relationships still apply).
pub fn chartex_part(c: &Chart) -> Option<(Vec<u8>, bool)> {
    kept_chartex(c).map(|x| (x, true)).or_else(|| c.kind.is_chartex().then(|| (build_chartex(c), false)))
}

fn kept_chartex(c: &Chart) -> Option<Vec<u8>> {
    let raw = c.raw.as_ref()?;
    let doc = crate::xmlx::parse(raw.as_bytes()).ok()?;
    let mut root = doc.clone();
    crate::xmlx::resolve_mc(&mut root);
    let mut back = super::chartex::chartex_from(&root)?;
    back.raw = c.raw.clone();
    // The sidecar is not modelled chart state.
    back.kept = c.kept.clone();
    if &back != c {
        return None;
    }
    let mut keep = doc;
    // The workbook link stays only when its part is written back.
    drop_unkept_refs(&mut keep, c);
    let mut w = Writer::document();
    w.raw(&keep.to_xml());
    Some(w.into_bytes())
}

/// The chart part XML, and whether it is the original part (so its relationships still apply).
pub fn chart_xml(c: &Chart) -> (Vec<u8>, bool) {
    // A chart too large to model is written back as it was read.
    if c.verbatim
        && let Some(raw) = &c.raw
        && let Ok(doc) = crate::xmlx::parse(raw.as_bytes())
        && doc.is(Local("chartSpace"))
    {
        let mut keep = doc;
        drop_unkept_refs(&mut keep, c);
        let mut w = Writer::document();
        w.raw(&keep.to_xml());
        return (w.into_bytes(), true);
    }
    // Reuse the kept part when the chart is unchanged since it was read.
    if let Some(raw) = &c.raw
        && !raw.contains(NS_CX)
        && let Ok(doc) = crate::xmlx::parse(raw.as_bytes())
        && doc.is(Local("chartSpace"))
    {
        let mut root = doc.clone();
        crate::xmlx::resolve_mc(&mut root);
        if let Some(back) = super::read::chart_from(&root) {
            let mut me = c.clone();
            me.raw = None;
            // The sidecar is not modelled chart state. Comparing it would throw away an unchanged chart.
            me.kept = Default::default();
            if back == me {
                let mut keep = doc;
                // Without the parts those elements point at, PowerPoint repairs the file.
                drop_unkept_refs(&mut keep, c);
                let mut w = Writer::document();
                w.raw(&keep.to_xml());
                return (w.into_bytes(), true);
            }
        }
    }
    let mut w = Writer::document();
    w.tag("c:chartSpace").attr("xmlns:c", NS_C).attr("xmlns:a", NS_A).attr("xmlns:r", NS_R).open();
    w.tag("c:date1904").attr("val", 0).empty();
    w.tag("c:roundedCorners").attr("val", 0).empty();
    w.tag("c:style").attr("val", c.style.clamp(1, 48)).empty();
    w.open("c:chart", &[]);
    match &c.title {
        Some(t) => {
            title(&mut w, t, c);
            w.tag("c:autoTitleDeleted").attr("val", 0).empty();
        }
        None => w.tag("c:autoTitleDeleted").attr("val", 1).empty(),
    }
    w.open("c:plotArea", &[]);
    w.empty("c:layout", &[]);
    let indexed: Vec<(usize, &Series)> = c.series.iter().enumerate().collect();
    let main = match c.kind {
        ChartType::Combo => c.series.first().and_then(|s| s.kind).unwrap_or(ChartType::Column),
        k => k,
    };
    if c.kind == ChartType::Combo {
        // One plot group per series type, sharing the axes.
        let mut kinds: Vec<ChartType> = vec![];
        for s in &c.series {
            let k = s.kind.unwrap_or(main);
            let k = if matches!(group_of(k), Group::Pie | Group::Doughnut | Group::Scatter | Group::Bubble) { ChartType::Column } else { k };
            if !kinds.contains(&k) {
                kinds.push(k);
            }
        }
        for k in kinds {
            let list: Vec<(usize, &Series)> = indexed
                .iter()
                .filter(|(_, s)| {
                    let sk = s.kind.unwrap_or(main);
                    let sk =
                        if matches!(group_of(sk), Group::Pie | Group::Doughnut | Group::Scatter | Group::Bubble) { ChartType::Column } else { sk };
                    sk == k
                })
                .copied()
                .collect();
            plot_group(&mut w, c, k, &list);
        }
        axes(&mut w, c, Group::Bar);
    } else {
        if !matches!(
            c.kind,
            ChartType::Column
                | ChartType::StackedColumn
                | ChartType::PercentColumn
                | ChartType::Bar
                | ChartType::StackedBar
                | ChartType::PercentBar
                | ChartType::Line
                | ChartType::LineMarkers
                | ChartType::StackedLine
                | ChartType::Area
                | ChartType::StackedArea
                | ChartType::Pie
                | ChartType::Doughnut
                | ChartType::Scatter
                | ChartType::Bubble
                | ChartType::Radar
                | ChartType::FilledRadar
                | ChartType::Stock
        ) {
            log::warn!("pptx: {} chart written as a column chart", c.kind.label());
        }
        plot_group(&mut w, c, main, &indexed);
        let g = group_of(main);
        if !matches!(g, Group::Pie | Group::Doughnut) {
            axes(&mut w, c, g);
        }
    }
    if let Some(f) = &c.plot_fill {
        sp_pr(&mut w, Some(f), None);
    }
    w.close();
    if let Some(pos) = &c.legend {
        let p = match pos.as_str() {
            "b" | "t" | "l" | "r" | "tr" => pos.as_str(),
            _ => "r",
        };
        w.open("c:legend", &[]);
        w.tag("c:legendPos").attr("val", p).empty();
        w.tag("c:overlay").attr("val", u8::from(c.legend_overlay)).empty();
        tx_pr(&mut w, c);
        w.close();
    }
    w.tag("c:plotVisOnly").attr("val", 1).empty();
    w.tag("c:dispBlanksAs").attr("val", "gap").empty();
    w.close();
    if let Some(f) = &c.chart_fill {
        sp_pr(&mut w, Some(f), None);
    }
    tx_pr(&mut w, c);
    w.close();
    (w.into_bytes(), false)
}

/// The relationship ids of an unchanged chart that [`write_kept`] writes back: external ones, and
/// internal ones whose part was kept.
fn kept_rel_ids(c: &Chart) -> std::collections::HashSet<&str> {
    let parts: std::collections::HashSet<String> = c.kept.parts.iter().filter(|p| kept_payload(&p.name)).map(|p| norm(&p.name)).collect();
    c.kept.rels.iter().filter(|r| r.external || parts.contains(&norm(&r.part))).map(|r| r.id.as_str()).collect()
}

/// A part name in the form names are compared in: no leading `/`, `/` for `\`, lower case.
pub fn norm(name: &str) -> String {
    name.trim_start_matches('/').replace('\\', "/").to_ascii_lowercase()
}

/// A part a chart may carry along beside its own `chartN.xml` (workbook, style, drawing, picture):
/// one in the `embeddings`, `charts`, `drawings` or `media` folder of a presentation, workbook or
/// document. Chart documents themselves are rewritten, and nothing outside these folders is
/// copied: a relationship must not be able to replace another part of the package on save.
pub fn sidecar_part(name: &str) -> bool {
    let n = norm(name);
    let segs: Vec<&str> = n.split('/').filter(|s| !s.is_empty()).collect();
    if segs.iter().any(|s| *s == "." || *s == "..") {
        return false;
    }
    let (Some(root), Some(area)) = (segs.first().copied(), segs.get(1).copied()) else { return false };
    if !matches!(root, "ppt" | "xl" | "word") || !matches!(area, "embeddings" | "charts" | "drawings" | "media") {
        return false;
    }
    let file = segs.last().copied().unwrap_or("");
    let stem = file.strip_prefix("chart").unwrap_or("");
    let num = stem.strip_suffix(".xml.rels").or_else(|| stem.strip_suffix(".xml")).unwrap_or("");
    !(stem != file && !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()))
}

/// A kept part that is written as a part of its own (rels parts are rewritten instead).
fn kept_payload(name: &str) -> bool {
    !name.to_ascii_lowercase().ends_with(".rels") && sidecar_part(name)
}

/// Drop `c:externalData` / `c:userShapes` (and the `cx:` ones) that point at a relationship that
/// is not written back: without the part PowerPoint repairs the file.
fn drop_unkept_refs(root: &mut El, c: &Chart) {
    let ids = kept_rel_ids(c);
    fn rec(e: &mut El, ids: &std::collections::HashSet<&str>, depth: usize) {
        if depth > crate::xmlx::MAX_DEPTH {
            return;
        }
        e.children.retain(|n| match n {
            Node::El(x) if x.is(Local("externalData")) || x.is(Local("userShapes")) => x.attr_local("r:id").is_some_and(|id| ids.contains(id)),
            _ => true,
        });
        for ch in e.elements_mut() {
            rec(ch, ids, depth + 1);
        }
    }
    rec(root, &ids, 0);
}

/// A `cx:tx` holding plain text.
fn cx_text(w: &mut Writer, t: &str) {
    w.open("cx:tx", &[]);
    w.open("cx:txData", &[]);
    w.open("cx:v", &[]);
    w.text(t);
    w.close();
    w.close();
    w.close();
}

/// A `cx:lvl` of points.
fn cx_level(w: &mut Writer, pts: &[String], format: Option<&str>) {
    let mut a = A::new().a("ptCount", pts.len());
    if let Some(f) = format {
        a = a.a("formatCode", f);
    }
    w.tag("cx:lvl").list(a).open();
    for (i, p) in pts.iter().enumerate() {
        w.tag("cx:pt").attr("idx", i).open();
        w.text(p);
        w.close();
    }
    w.close();
}

/// The `cx:layoutPr` of the region map series in a kept `chartEx` part.
fn kept_region_layout(raw: &str) -> Option<String> {
    let doc = crate::xmlx::parse(raw.as_bytes()).ok()?;
    let mut root = doc;
    crate::xmlx::resolve_mc(&mut root);
    let region = root.child(Local("chart"))?.child(Local("plotArea"))?.child(Local("plotAreaRegion"))?;
    let s = region.children(Local("series")).find(|s| s.attr_local("layoutId") == Some("regionMap"))?;
    let lp = s.child(Local("layoutPr"))?;
    // Written under our `cx` prefix; a part that bound chartex to another prefix is not copied.
    lp.name.starts_with("cx:").then(|| lp.to_xml())
}

/// Writes an Office 2016 chart from the model, as PowerPoint lays the part out: one `cx:data`
/// per series (categories and values), then the plot area with each series' layout, the axes
/// the layout uses, the title and the legend.
pub fn build_chartex(c: &Chart) -> Vec<u8> {
    let mut w = Writer::document();
    w.tag("cx:chartSpace").attr("xmlns:a", NS_A).attr("xmlns:r", NS_R).attr("xmlns:cx", NS_CX).open();
    w.open("cx:chartData", &[]);
    for (i, s) in c.series.iter().enumerate() {
        w.tag("cx:data").attr("id", i).open();
        if !c.categories.is_empty() {
            w.tag("cx:strDim").attr("type", "cat").open();
            cx_level(&mut w, &c.categories, None);
            for lv in &c.category_levels {
                cx_level(&mut w, lv, None);
            }
            w.close();
        }
        let ty = match c.kind {
            ChartType::Treemap | ChartType::Sunburst => "size",
            ChartType::RegionMap => "colorVal",
            _ => "val",
        };
        w.tag("cx:numDim").attr("type", ty).open();
        let vals: Vec<String> = s.values.iter().map(|v| v.map(fmt_num).unwrap_or_default()).collect();
        cx_level(&mut w, &vals, Some(s.number_format.as_deref().unwrap_or("General")));
        w.close();
        w.close();
    }
    w.close();
    w.open("cx:chart", &[]);
    if let Some(t) = &c.title {
        w.tag("cx:title").attr("pos", "t").attr("align", "ctr").attr("overlay", "0").open();
        cx_text(&mut w, t);
        w.close();
    }
    w.open("cx:plotArea", &[]);
    w.open("cx:plotAreaRegion", &[]);
    let layout = match c.kind {
        ChartType::Waterfall => "waterfall",
        ChartType::Funnel => "funnel",
        ChartType::Treemap => "treemap",
        ChartType::Sunburst => "sunburst",
        ChartType::BoxWhisker => "boxWhisker",
        ChartType::RegionMap => "regionMap",
        _ => "clusteredColumn",
    };
    let one = !matches!(c.kind, ChartType::BoxWhisker);
    for (i, s) in c.series.iter().enumerate().take(if one { 1 } else { usize::MAX }) {
        w.tag("cx:series").attr("layoutId", layout).attr("uniqueId", format!("{{DC00000{i}-0000-4000-8000-000000000000}}")).open();
        cx_text(&mut w, &s.name);
        if c.series_data_labels(s) {
            w.tag("cx:dataLabels").attr("pos", if c.kind == ChartType::Funnel { "ctr" } else { "outEnd" }).open();
            w.tag("cx:visibility").attr("seriesName", "0").attr("categoryName", "0").attr("value", "1").empty();
            w.close();
        }
        w.tag("cx:dataId").attr("val", i).empty();
        match c.kind {
            ChartType::Waterfall if !c.subtotals.is_empty() => {
                w.open("cx:layoutPr", &[]);
                w.open("cx:subtotals", &[]);
                for t in &c.subtotals {
                    w.tag("cx:idx").attr("val", t).empty();
                }
                w.close();
                w.close();
            }
            ChartType::Histogram | ChartType::Pareto => {
                w.open("cx:layoutPr", &[]);
                w.tag("cx:binning").attr("intervalClosed", "r").open();
                if let Some(size) = c.bin_size {
                    w.tag("cx:binSize").attr("val", fmt_num(size)).empty();
                } else if let Some(n) = c.bin_count {
                    w.tag("cx:binCount").attr("val", n).empty();
                }
                w.close();
                w.close();
            }
            ChartType::BoxWhisker => {
                w.open("cx:layoutPr", &[]);
                w.tag("cx:visibility").attr("meanLine", "0").attr("meanMarker", "1").attr("nonoutliers", "0").attr("outliers", "1").empty();
                w.tag("cx:statistics").attr("quartileMethod", if c.inclusive_quartiles { "inclusive" } else { "exclusive" }).empty();
                w.close();
            }
            ChartType::RegionMap => {
                // The map's geography (projection, region level and the cached boundaries
                // PowerPoint downloaded) is not modelled: carry it over from the part we read.
                if let Some(lp) = c.raw.as_deref().and_then(kept_region_layout) {
                    w.raw(&lp);
                }
            }
            _ => {}
        }
        w.close();
    }
    if c.kind == ChartType::Pareto {
        w.tag("cx:series").attr("layoutId", "paretoLine").attr("ownerIdx", "0").attr("uniqueId", "{DC0000FF-0000-4000-8000-000000000000}").open();
        w.tag("cx:axisId").attr("val", "2").empty();
        w.close();
    }
    w.close();
    if !matches!(c.kind, ChartType::Treemap | ChartType::Sunburst | ChartType::RegionMap) {
        w.tag("cx:axis").attr("id", "0").open();
        w.tag("cx:catScaling").attr("gapWidth", if matches!(c.kind, ChartType::Histogram | ChartType::Pareto) { "0" } else { "0.5" }).empty();
        w.empty("cx:tickLabels", &[]);
        w.close();
        if c.kind != ChartType::Funnel {
            w.tag("cx:axis").attr("id", "1").open();
            w.empty("cx:valScaling", &[]);
            if c.gridlines {
                w.empty("cx:majorGridlines", &[]);
            }
            w.empty("cx:tickLabels", &[]);
            w.close();
        }
        if c.kind == ChartType::Pareto {
            w.tag("cx:axis").attr("id", "2").open();
            w.tag("cx:valScaling").attr("max", "1").attr("min", "0").empty();
            w.empty("cx:units", &[]);
            w.empty("cx:tickLabels", &[]);
            w.close();
        }
    }
    w.close();
    if let Some(pos) = &c.legend {
        w.tag("cx:legend").attr("pos", pos.as_str()).attr("align", "ctr").attr("overlay", "0").empty();
    }
    w.close();
    w.close();
    w.into_bytes()
}

#[cfg(test)]
mod region_map_tests {
    use super::*;

    /// A region map part as PowerPoint writes it (our own numbers; the cached geography is a
    /// stand-in blob).
    const MAP: &str = r#"<cx:chartSpace xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex"><cx:chartData><cx:data id="0"><cx:strDim type="cat"><cx:f>Sheet1!$A$2:$A$4</cx:f><cx:lvl ptCount="3"><cx:pt idx="0">Minsk</cx:pt><cx:pt idx="1">Brest</cx:pt><cx:pt idx="2">Gomel</cx:pt></cx:lvl></cx:strDim><cx:numDim type="colorVal"><cx:f>Sheet1!$B$2:$B$4</cx:f><cx:lvl ptCount="3" formatCode="General"><cx:pt idx="0">12</cx:pt><cx:pt idx="1">5</cx:pt><cx:pt idx="2">8</cx:pt></cx:lvl></cx:numDim></cx:data></cx:chartData><cx:chart><cx:title pos="t" align="ctr" overlay="0"/><cx:plotArea><cx:plotAreaRegion><cx:series layoutId="regionMap" uniqueId="{00000000-0000-4000-8000-000000000001}"><cx:tx><cx:txData><cx:f>Sheet1!$B$1</cx:f><cx:v>Sales</cx:v></cx:txData></cx:tx><cx:dataId val="0"/><cx:layoutPr><cx:geography cultureLanguage="en-US" cultureRegion="BY" attribution="Powered by Bing"><cx:geoCache provider="{E9337A44-BEBE-4D9F-B70C-5C5E7DAFC167}"><cx:binary>ZGVja2NyYWZ0LXRlc3Q=</cx:binary></cx:geoCache></cx:geography></cx:layoutPr></cx:series></cx:plotAreaRegion></cx:plotArea><cx:legend pos="r" align="min" overlay="0"/></cx:chart></cx:chartSpace>"#;

    #[test]
    fn region_maps_keep_their_geography_through_save_and_edit() {
        let c = crate::chart::chartex::read_chartex_xml(MAP.as_bytes()).expect("a region map is read");
        assert_eq!(c.kind, ChartType::RegionMap);
        assert_eq!(c.categories, ["Minsk", "Brest", "Gomel"]);
        assert_eq!(c.series.first().map(|s| s.values.clone()), Some(vec![Some(12.0), Some(5.0), Some(8.0)]));
        // Unchanged: written back as read.
        let kept = String::from_utf8(chartex_xml(&c).unwrap_or_default()).unwrap_or_default();
        assert!(kept.contains("ZGVja2NyYWZ0LXRlc3Q=") && kept.contains("cultureRegion=\"BY\""), "{kept}");
        // Edited: rebuilt from the model, with the geography carried over.
        let mut e = c.clone();
        if let Some(s) = e.series.first_mut() {
            s.values[1] = Some(20.0);
        }
        let built = String::from_utf8(chartex_xml(&e).unwrap_or_default()).unwrap_or_default();
        assert!(built.contains("<cx:geoCache") && built.contains("ZGVja2NyYWZ0LXRlc3Q=") && built.contains("colorVal"), "{built}");
        assert!(!built.contains("<cx:axis"), "{built}");
        let back = crate::chart::chartex::read_chartex_xml(built.as_bytes()).expect("the rebuilt part reads");
        assert_eq!(back.kind, ChartType::RegionMap);
        assert_eq!(back.series.first().and_then(|s| s.values.get(1).copied().flatten()), Some(20.0));
    }
}

#[cfg(test)]
mod tests {
    use crate::chart::{Chart, ChartType, Series};
    use crate::color::{ColorTransform, Rgba};
    use crate::style::ColorRef;

    #[test]
    fn export_cost_counts_the_caches_each_group_writes() {
        let s = Series { values: vec![Some(1.0); 600], ..Default::default() };
        let mut c = Chart::sample(ChartType::Column);
        c.categories = vec!["a".into(); 1000];
        // Category and value caches: 1,000 + 600, however the series is stored.
        assert_eq!(super::series_points(&c, &s, super::Group::Bar), 1600);
        // A bubble series without x values or sizes still writes three value-length caches.
        assert_eq!(super::series_points(&c, &s, super::Group::Bubble), 1800);
        assert_eq!(super::series_points(&c, &s, super::Group::Scatter), 1200);
        let s = Series { values: vec![Some(1.0); 600], x_values: vec![Some(1.0); 5], sizes: vec![Some(1.0); 7], ..Default::default() };
        assert_eq!(super::series_points(&c, &s, super::Group::Bubble), 612);
    }

    /// The chart's text colour is written where PowerPoint keeps it (`c:txPr` on the axes, legend,
    /// data labels and chart, the title's run defaults) and read back as the same colour.
    #[test]
    fn chart_text_colour_round_trips() {
        let mut c = Chart::sample(ChartType::Column);
        c.title = Some("Revenue".into());
        c.legend = Some("b".into());
        c.data_labels = true;
        c.axis_titles = (Some("Quarter".into()), Some("EUR".into()));
        let tinted = ColorRef::scheme(crate::color::SchemeSlot::Accent2).with(ColorTransform::LumMod(75000));
        c.text_color = Some(tinted.clone());
        let xml = String::from_utf8(super::chart_xml(&c).0).unwrap();
        // Axes (2), legend, data labels and the chart root; a title keeps it in its rich text.
        assert!(xml.matches("<c:txPr>").count() >= 5, "{xml}");
        assert!(xml.contains(r#"<a:schemeClr val="accent2"><a:lumMod val="75000"/></a:schemeClr>"#), "{xml}");
        assert!(xml.contains("<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr><a:solidFill>"), "{xml}");
        assert_eq!(crate::chart::read::read_chart_xml(xml.as_bytes()).unwrap().text_color, Some(tinted));
        // An RGB colour and no colour at all round-trip too.
        c.text_color = Some(ColorRef::rgb(Rgba::rgb(255, 255, 255)));
        let xml = super::chart_xml(&c).0;
        assert_eq!(crate::chart::read::read_chart_xml(&xml).unwrap().text_color, c.text_color);
        c.text_color = None;
        let xml = String::from_utf8(super::chart_xml(&c).0).unwrap();
        assert!(!xml.contains("<c:txPr>"), "{xml}");
        assert_eq!(crate::chart::read::read_chart_xml(xml.as_bytes()).unwrap().text_color, None);
    }

    /// An imported chart's `c:txPr` colour survives an edit that rebuilds the part from the model
    /// (the colour used to be read but never written, so a save after any edit lost it).
    #[test]
    fn imported_text_colour_survives_an_edit_and_a_save() {
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:val><c:numRef><c:numCache><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:txPr><a:bodyPr/><a:p><a:pPr><a:defRPr><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="65000"/><a:lumOff val="35000"/></a:schemeClr></a:solidFill></a:defRPr></a:pPr></a:p></c:txPr><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:crossAx val="1"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#;
        let c = crate::chart::read::read_chart_xml(xml.as_bytes()).unwrap();
        let grey = ColorRef::scheme(crate::color::SchemeSlot::Tx1).with(ColorTransform::LumMod(65000)).with(ColorTransform::LumOff(35000));
        assert_eq!(c.text_color, Some(grey.clone()));
        // Unchanged: the part is written back as read.
        assert!(super::chart_xml(&c).1);
        // Edited (a title; the editor drops `raw` on any edit): rebuilt with the colour.
        let mut e = c.clone();
        e.title = Some("Units".into());
        e.raw = None;
        let (out, kept) = super::chart_xml(&e);
        assert!(!kept);
        let back = crate::chart::read::read_chart_xml(&out).unwrap();
        assert_eq!(back.text_color, Some(grey));
        assert_eq!(back.title.as_deref(), Some("Units"));
    }

    #[test]
    fn reversed_category_axis_round_trips() {
        let mut c = Chart::sample(ChartType::Bar);
        c.cat_reversed = true;
        let xml = super::chart_xml(&c).0;
        assert!(String::from_utf8_lossy(&xml).contains(r#"<c:orientation val="maxMin"/>"#));
        assert!(crate::chart::read::read_chart_xml(&xml).unwrap().cat_reversed);
        c.cat_reversed = false;
        assert!(!crate::chart::read::read_chart_xml(&super::chart_xml(&c).0).unwrap().cat_reversed);
    }
}

#[cfg(test)]
mod cache_cap_tests {
    use super::*;

    /// A cache longer than the cap is cut, never refused; anything a real chart holds (32,000
    /// points per series) is far under it. Zero-sized elements, so the test allocates nothing.
    #[test]
    fn caches_are_cut_at_the_cap_not_refused() {
        const { assert!(MAX_CACHE_POINTS >= 32_000) };
        let over = vec![(); MAX_CACHE_POINTS + 1];
        assert_eq!(capped(&over).len(), MAX_CACHE_POINTS);
        let real = vec![(); 32_000];
        assert_eq!(capped(&real).len(), 32_000);
    }

    #[test]
    fn series_formulas_are_read_and_written_back_and_made_up_when_absent() {
        let mut c = Chart { kind: ChartType::Bar, categories: vec!["a".into(), "b".into()], ..Default::default() };
        c.series.push(Series { name: "S".into(), values: vec![Some(1.0), Some(2.0)], ..Default::default() });
        let made = String::from_utf8(super::chart_xml(&c).0).unwrap();
        assert!(made.contains("<c:f>Sheet1!$B$2:$B$3</c:f>"), "{made}");
        let back = crate::chart::read::read_chart_xml(made.as_bytes()).unwrap();
        assert_eq!(back.series[0].refs.values.as_deref(), Some("Sheet1!$B$2:$B$3"));
        c.series[0].refs = crate::chart::SeriesRefs {
            name: Some("Data!$C$1".into()),
            categories: Some("Data!$A$2:$A$3".into()),
            values: Some("'My sheet'!$C$2:$C$3".into()),
            sizes: None,
        };
        let xml = String::from_utf8(super::chart_xml(&c).0).unwrap();
        assert!(xml.contains("<c:f>Data!$C$1</c:f>") && xml.contains("<c:f>Data!$A$2:$A$3</c:f>") && xml.contains("<c:f>'My sheet'!$C$2:$C$3</c:f>"));
        assert_eq!(crate::chart::read::read_chart_xml(xml.as_bytes()).unwrap().series[0].refs, c.series[0].refs);
    }
}
