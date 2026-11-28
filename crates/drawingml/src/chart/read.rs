//! Chart parts (`c:chartSpace`) → [`Chart`]. The original XML is kept in `Chart.raw`.

use crate::chart::{Chart, ChartDataTable, ChartType, ErrorBars, Series, Trendline, TrendlineType};
use crate::style::Fill;

use craft_xml::{El, Local, value};

use crate::read as dml;

/// Fills of chart parts never reference images we keep; read colour and pattern fills only.
fn simple_fill(sp: Option<&El>) -> Option<Fill> {
    let sp = sp?;
    for e in sp.elements() {
        match e.local() {
            "noFill" => return Some(Fill::None),
            "solidFill" => return dml::color(e).map(|color| Fill::Solid { color }),
            "gradFill" => return Some(Fill::Gradient(dml::gradient(e))),
            "pattFill" => return Some(Fill::Pattern(dml::pattern(e))),
            _ => {}
        }
    }
    None
}

fn simple_line(sp: Option<&El>) -> Option<crate::style::Line> {
    let ln = sp?.child(Local("ln"))?;
    let mut l = crate::style::Line { width: dml::pt(ln, "w"), ..Default::default() };
    l.fill = simple_fill(Some(ln));
    Some(l)
}

pub fn rich_text(t: &El) -> Option<String> {
    let tx = t.child(Local("tx"))?;
    if let Some(r) = tx.child(Local("rich")) {
        let s = r.children(Local("p")).map(|p| p.deep_text()).collect::<Vec<_>>().join("\n");
        return Some(s);
    }
    tx.find(Local("v")).map(|v| v.deep_text())
}

/// Most category/value/size slots one chart may decode across all its series and caches, so
/// sparse `ptCount`/`idx` declarations can't expand far beyond the XML they came from.
pub const MAX_CHART_POINTS: usize = 1_000_000;
/// Data slots (values and categories) the charts of one presentation may hold in all; a chart past
/// it is read as a placeholder frame. Each frame that references a chart decodes its own copy.
pub const MAX_PRESENTATION_CHART_POINTS: usize = 4_000_000;

/// Chart part bytes all chart frames of a presentation may decode, counting every attempt: a
/// chart that is rejected, or has no data slots, would otherwise be decoded again for each frame
/// that shows it.
pub const MAX_CHART_DECODE_BYTES: usize = 256 * 1024 * 1024;

/// Slots a decoded chart holds, counting the chart itself as at least one.
pub fn point_count(c: &Chart) -> usize {
    let series = |s: &crate::chart::Series| {
        let bars = s.error_bars.as_ref().map_or(0, |e| e.plus.len().saturating_add(e.minus.len()));
        s.values.len().saturating_add(s.x_values.len()).saturating_add(s.sizes.len()).saturating_add(bars)
    };
    let levels: usize = c.category_levels.iter().map(Vec::len).sum();
    1usize.saturating_add(c.categories.len()).saturating_add(levels).saturating_add(c.series.iter().map(series).fold(0, usize::saturating_add))
}

/// Most slots one cache may hold.
const MAX_CACHE_POINTS: usize = 100_000;

/// String values from `c:cat`/`c:tx` (str or num refs, caches or literals). Every slot returned
/// is taken from `budget`; once it runs out the values are cut short.
fn strings(e: &El, budget: &mut usize) -> Vec<String> {
    let cap = MAX_CACHE_POINTS.min(*budget);
    let out = strings_capped(e, cap);
    *budget = budget.saturating_sub(out.len());
    out
}

fn strings_capped(e: &El, cap: usize) -> Vec<String> {
    let cache =
        e.find(Local("strCache")).or_else(|| e.find(Local("numCache"))).or_else(|| e.find(Local("strLit"))).or_else(|| e.find(Local("numLit")));
    if let Some(c) = cache {
        let n = (c.child(Local("ptCount")).and_then(|p| p.attr_local("val").and_then(value::to_u32_lossy)).unwrap_or(0) as usize).min(cap);
        let mut out = vec![String::new(); n];
        for pt in c.children(Local("pt")).take(MAX_CACHE_POINTS) {
            let i = pt.attr_local("idx").and_then(value::to_u32_lossy).unwrap_or(0) as usize;
            if i >= cap {
                continue;
            }
            let v = pt.child(Local("v")).map(|v| v.deep_text()).unwrap_or_default();
            if i >= out.len() {
                out.resize(i + 1, String::new());
            }
            if let Some(s) = out.get_mut(i) {
                *s = v;
            }
        }
        return out;
    }
    if let Some(ml) = e.find(Local("multiLvlStrCache")) {
        // Use the innermost level.
        if let Some(l) = ml.child(Local("lvl")) {
            return l.children(Local("pt")).take(cap).map(|p| p.child(Local("v")).map(|v| v.deep_text()).unwrap_or_default()).collect();
        }
    }
    if cap > 0
        && let Some(v) = e.child(Local("v"))
    {
        return vec![v.deep_text()];
    }
    vec![]
}

fn numbers(e: &El, budget: &mut usize) -> Vec<Option<f64>> {
    strings(e, budget).into_iter().map(|s| s.trim().parse::<f64>().ok().filter(|v| v.is_finite())).collect()
}

/// Parse a chart part. Returns `None` for something that isn't a chart we can read.
pub fn read_chart_xml(bytes: &[u8]) -> Option<Chart> {
    let doc = crate::xmlx::parse(bytes).ok()?;
    let mut root = doc;
    crate::xmlx::resolve_mc(&mut root);
    if !root.is(Local("chartSpace")) {
        return None;
    }
    let mut c = chart_from(&root)?;
    c.raw = Some(String::from_utf8_lossy(bytes).into_owned());
    Some(c)
}

pub fn chart_from(root: &El) -> Option<Chart> {
    chart_within(root, MAX_CHART_POINTS)
}

/// [`chart_from`] decoding at most `budget` data slots in all.
fn chart_within(root: &El, budget: usize) -> Option<Chart> {
    read_chart(root, budget, 10)
}

/// Exact former metadata projection for unchanged legacy native saves, using the same caps.
pub(super) fn legacy_chart_from(root: &El) -> Option<Chart> {
    read_chart(root, MAX_CHART_POINTS, 0)
}

/// The published first typed projection, before stacked visibility and secondary-only axes.
pub(super) fn metadata_v1_chart_from(root: &El) -> Option<Chart> {
    read_chart(root, MAX_CHART_POINTS, 1)
}

/// The published stacked-marker projection, before series IDs were retained.
pub(super) fn metadata_v2_chart_from(root: &El) -> Option<Chart> {
    read_chart(root, MAX_CHART_POINTS, 2)
}

/// The series-ID projection, before reciprocal duplicate-axis selection was corrected.
pub(super) fn metadata_v3_chart_from(root: &El) -> Option<Chart> {
    read_chart(root, MAX_CHART_POINTS, 3)
}

/// The published reciprocal-pair projection, before selected-element and category-format fixes.
pub(super) fn metadata_v4_chart_from(root: &El) -> Option<Chart> {
    read_chart(root, MAX_CHART_POINTS, 4)
}

/// The published source-group projection, before all group consumers kept the resolved axis.
pub(super) fn metadata_v5_chart_from(root: &El) -> Option<Chart> {
    read_chart(root, MAX_CHART_POINTS, 5)
}

/// The published group-element projection, before identity reached ownership and formats.
pub(super) fn metadata_v6_chart_from(root: &El) -> Option<Chart> {
    read_chart(root, MAX_CHART_POINTS, 6)
}

/// The published element-identity projection, before scatter ambiguity and linked defaults.
pub(super) fn metadata_v7_chart_from(root: &El) -> Option<Chart> {
    read_chart(root, MAX_CHART_POINTS, 7)
}

/// The published linked-default projection, before orphan ownership and source-format fixes.
pub(super) fn metadata_v8_chart_from(root: &El) -> Option<Chart> {
    read_chart(root, MAX_CHART_POINTS, 8)
}

/// The published orphan-owner projection, before exact secondary and value-format fixes.
pub(super) fn metadata_v9_chart_from(root: &El) -> Option<Chart> {
    read_chart(root, MAX_CHART_POINTS, 9)
}

#[cfg(test)]
std::thread_local! {
    static RAW_PROJECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn chart_for_version(root: &El, version: u16) -> Option<Chart> {
    #[cfg(test)]
    RAW_PROJECTIONS.with(|count| count.set(count.get() + 1));
    match version {
        0 => legacy_chart_from(root),
        1 => metadata_v1_chart_from(root),
        2 => metadata_v2_chart_from(root),
        3 => metadata_v3_chart_from(root),
        4 => metadata_v4_chart_from(root),
        5 => metadata_v5_chart_from(root),
        6 => metadata_v6_chart_from(root),
        7 => metadata_v7_chart_from(root),
        8 => metadata_v8_chart_from(root),
        9 => metadata_v9_chart_from(root),
        _ => chart_from(root),
    }
}

pub(super) fn original_kind(c: &Chart) -> Option<ChartType> {
    let raw = c.raw.as_deref().filter(|raw| !raw.contains(super::write::NS_CX))?;
    let mut root = crate::xmlx::parse(raw.as_bytes()).ok()?;
    if !root.is(Local("chartSpace")) {
        return None;
    }
    crate::xmlx::resolve_mc(&mut root);
    chart_for_version(&root, c.raw_metadata_version).map(|chart| chart.kind)
}

fn read_chart(root: &El, mut budget: usize, version: u16) -> Option<Chart> {
    let legacy = version == 0;
    let ch = root.child(Local("chart"))?;
    let pa = ch.child(Local("plotArea"))?;
    let mut c = Chart {
        categories: vec![],
        series: vec![],
        title: None,
        legend: None,
        gridlines: false,
        raw_metadata_version: version,
        ..Default::default()
    };
    c.style = root.child(Local("style")).and_then(|c| c.attr_local("val")).and_then(|v| v.parse().ok()).unwrap_or(2);
    if let Some(t) = ch.child(Local("title")) {
        c.title = Some(rich_text(t).unwrap_or_else(|| "Chart Title".into()));
    }
    if ch.child(Local("autoTitleDeleted")).and_then(|a| a.attr_local("val").and_then(value::to_bool)) == Some(true) {
        c.title = None;
    }
    c.legend = ch.child(Local("legend")).map(|l| l.child(Local("legendPos")).and_then(|c| c.attr_local("val")).unwrap_or("r").to_string());
    c.legend_overlay =
        ch.child(Local("legend")).and_then(|l| l.child(Local("overlay"))).and_then(|o| o.attr_local("val").and_then(value::to_bool)).unwrap_or(false);
    c.legend_entries = legend_entries(ch.child(Local("legend")));
    c.chart_fill = simple_fill(root.child(Local("spPr")));
    c.plot_fill = simple_fill(pa.child(Local("spPr")));

    let mut kinds: Vec<ChartType> = vec![];
    let mut series = vec![];
    // Axis placement identifies the primary value axis; a nonempty group on another value
    // axis is secondary, even when both groups have the same chart type.
    let val_map: std::collections::HashMap<&str, bool> = pa
        .children(Local("valAx"))
        .filter_map(|a| {
            Some((
                a.child(Local("axId")).and_then(|c| c.attr_local("val"))?,
                matches!(a.child(Local("axPos")).and_then(|c| c.attr_local("val")), Some("l" | "r")),
            ))
        })
        .collect();
    let relations = (version >= 6).then(|| AxisRelations::new(pa, version));
    let primary_axis = primary_value_axis(pa, &val_map, version, relations.as_ref());
    // A fallback element is not evidence of group ownership. Resolve this once and share
    // the evidence with series ownership and secondary-scale selection.
    let resolved_primary = primary_axis.filter(|axis| {
        version >= 7
            && pa.elements().any(|g| {
                group_chart_type(g.local(), g).is_some()
                    && g.children(Local("ser")).next().is_some()
                    && relations.as_ref().and_then(|index| index.value_axis(g, &val_map)).is_some_and(|a| std::ptr::eq(a, *axis))
            })
    });
    let primary_val = primary_axis.and_then(|a| a.child(Local("axId"))).and_then(|a| a.attr_local("val")).map(str::to_string);
    let mut group_ordinal = 0_u32;
    for g in pa.elements() {
        let name = g.local();
        if !name.ends_with("Chart") {
            continue;
        }
        let Some((mut kind, group_markers)) = group_metadata(name, g, version) else { continue };
        // Membership token for this read, not a persistent numeric group ID across rebuilds.
        let source_group = group_ordinal;
        group_ordinal = group_ordinal.saturating_add(1);
        if legacy
            && kind == ChartType::LineMarkers
            && g.children(Local("ser")).any(|s| s.path(&[Local("marker"), Local("symbol")]).and_then(|m| m.attr_local("val")) == Some("none"))
        {
            kind = ChartType::Line;
        }
        if name == "stockChart" {
            c.up_down_bars = g.child(Local("upDownBars")).is_some();
        }
        let group_labels = read_group_settings(g, &mut c);
        kinds.push(kind);
        let val_axis = group_value_axis(&val_map, g);
        let secondary = if version >= 8 {
            group_secondary(g, resolved_primary, primary_val.as_deref(), &val_map, relations.as_ref(), version)
        } else {
            match (resolved_primary, relations.as_ref().and_then(|index| index.value_axis(g, &val_map))) {
                (Some(primary), Some(axis)) => !std::ptr::eq(primary, axis),
                _ => val_axis.is_some() && val_axis != primary_val,
            }
        };
        for s in g.children(Local("ser")).take(1000) {
            let needs_categories = c.categories.is_empty();
            let mut ser = read_series(s, g, (kind, secondary, group_labels), &mut c.categories, &mut budget, version);
            if version >= 5 {
                ser.source_group = Some(source_group);
            }
            if version >= 3 {
                ser.source_idx = s.child(Local("idx")).and_then(|i| i.attr_local("val")).and_then(|v| v.trim().parse::<u32>().ok());
            }
            if version >= 2 && kind == ChartType::StackedLine {
                ser.marker_visible = Some(match ser.marker.as_deref() {
                    Some("none") => false,
                    Some(_) => true,
                    None => group_markers,
                });
            }
            if legacy {
                ser.kind = Some(kind);
                if ser.marker.as_deref() == Some("none") {
                    ser.marker = None;
                }
            }
            if needs_categories && !c.categories.is_empty() {
                let axis = if version >= 5 { group_category_axis(pa, g, primary_axis, &val_map, relations.as_ref()) } else { category_axis(pa, g) };
                c.category_format = category_format(axis, s, version);
            }
            let order = s.child(Local("order")).and_then(|o| o.attr_local("val")).and_then(|v| v.trim().parse::<u32>().ok());
            series.push((order.map(|v| v as usize).unwrap_or(series.len()), ser));
        }
    }
    // Decode and charge the source-order caches before sorting complete records. Equal orders
    // keep document order; an omitted or invalid order keeps its source position as its key.
    // ECMA-376-1 §21.2.2.128: c:order is the zero-based order of the series collection.
    if !legacy {
        series.sort_by_key(|(order, _)| *order);
    }
    c.series = series.into_iter().map(|(_, s)| s).collect();
    // `autoTitleDeleted val="0"` without `c:title`: PowerPoint and LibreOffice draw the series name as the title of a single-series chart.
    if ch.child(Local("title")).is_none()
        && ch.child(Local("autoTitleDeleted")).and_then(|a| a.attr_local("val").and_then(value::to_bool)) == Some(false)
        && c.series.len() == 1
    {
        c.title = c.series.first().map(|s| s.name.clone()).filter(|n| !n.is_empty());
    }
    let first = *kinds.first()?;
    if legacy {
        c.kind = if kinds.iter().any(|k| *k != first) { ChartType::Combo } else { first };
        if c.kind != ChartType::Combo {
            for s in &mut c.series {
                s.kind = None;
                s.secondary_axis = false;
            }
        }
    } else {
        let first = c.series.first().and_then(|s| s.kind).unwrap_or(first);
        let mixed = if c.series.is_empty() { kinds.iter().any(|k| *k != first) } else { c.series.iter().any(|s| s.kind != Some(first)) };
        c.kind = if mixed { ChartType::Combo } else { first };
    }
    c.lang = root.child(Local("lang")).and_then(|c| c.attr_local("val")).map(str::to_string);
    // A bare `<c:date1904/>` means true, as in the schema's default.
    c.date_1904 = root.child(Local("date1904")).is_some_and(|d| d.attr_local("val").and_then(value::to_bool).unwrap_or(true));
    c.pivot_source = root.path(&[Local("pivotSource"), Local("name")]).map(|n| n.deep_text().trim().to_string()).filter(|n| !n.is_empty());
    c.data_table = pa.child(Local("dTable")).map(|t| {
        let flag = |name: &str| t.child(Local(name)).is_some_and(|e| e.attr_local("val").and_then(value::to_bool).unwrap_or(true));
        ChartDataTable {
            show_keys: flag("showKeys"),
            horizontal_border: flag("showHorzBorder"),
            vertical_border: flag("showVertBorder"),
            outline: flag("showOutline"),
        }
    });
    let (val_ax, cat_ax) = read_axes(&mut c, pa, &primary_val, (primary_axis, resolved_primary), &val_map, version, relations.as_ref());
    read_text_styles(&mut c, root, ch, val_ax, cat_ax);
    Some(c)
}

/// A group's value axis: its vertical one when it has two (scatter), else its one.
fn group_value_axis(val_map: &std::collections::HashMap<&str, bool>, g: &El) -> Option<String> {
    let ids: Vec<(&str, bool)> =
        g.children(Local("axId")).filter_map(|a| a.attr_local("val")).filter_map(|id| val_map.get(id).copied().map(|v| (id, v))).collect();
    ids.iter().find(|(_, vertical)| *vertical).or(ids.first()).map(|(id, _)| id.to_string())
}

/// One ownership decision for current series flags and secondary-group fallback.
/// A resolved group cannot become secondary merely because its ID-only role is corrupted.
fn group_secondary(
    group: &El,
    primary: Option<&El>,
    primary_id: Option<&str>,
    val_map: &std::collections::HashMap<&str, bool>,
    relations: Option<&AxisRelations<'_>>,
    version: u16,
) -> bool {
    match (primary, relations.and_then(|index| index.value_axis(group, val_map))) {
        (Some(primary), Some(axis)) => !std::ptr::eq(primary, axis),
        // No populated group owns the selected primary element, so this resolved owner
        // must be secondary, even when its numeric ID also occurs on the unused anchor.
        (None, Some(_)) if version >= 9 => true,
        _ => group_value_axis(val_map, group).is_some_and(|id| Some(id.as_str()) != primary_id),
    }
}

/// Exact relation slots for one read. Duplicate equally valid elements are ambiguous.
/// Index once, then resolve only the group's two unique declared roles, never all axis pairs.
struct AxisRelations<'a> {
    version: u16,
    values: std::collections::HashMap<(&'a str, &'a str), Option<&'a El>>,
    partners: std::collections::HashMap<(&'a str, &'a str), Option<&'a El>>,
}

impl<'a> AxisRelations<'a> {
    fn new(pa: &'a El, version: u16) -> Self {
        let mut index = Self { version, values: Default::default(), partners: Default::default() };
        for axis in pa.elements() {
            let Some(id) = axis.child(Local("axId")).and_then(|a| a.attr_local("val")) else { continue };
            let Some(cross) = axis.child(Local("crossAx")).and_then(|a| a.attr_local("val")) else { continue };
            if axis.is(Local("valAx")) {
                index.values.entry((id, cross)).and_modify(|slot| *slot = None).or_insert(Some(axis));
            }
            if matches!(axis.local(), "catAx" | "dateAx")
                || (axis.is(Local("valAx")) && !matches!(axis.child(Local("axPos")).and_then(|a| a.attr_local("val")), Some("l" | "r")))
            {
                index.partners.entry((id, cross)).and_modify(|slot| *slot = None).or_insert(Some(axis));
            }
        }
        index
    }

    fn value_axis(&self, group: &El, val_map: &std::collections::HashMap<&str, bool>) -> Option<&'a El> {
        let mut ids = group.children(Local("axId")).filter_map(|a| a.attr_local("val"));
        let first = ids.next()?;
        let mut second = None;
        for id in ids {
            if id == first || second == Some(id) {
                continue;
            }
            if second.is_some() {
                // More than two unique IDs has no unambiguous pair; keep the old fallback.
                return None;
            }
            second = Some(id);
        }
        let second = second?;
        if self.version >= 7 {
            // Direction belongs to the exact pair, not the last axis sharing either ID.
            // A value-axis partner is indexed only when horizontal; cat/date are eligible.
            let candidate = |id, cross| {
                let axis = self.values.get(&(id, cross)).copied().flatten()?;
                let partner = self.partners.get(&(cross, id)).copied().flatten()?;
                if self.version >= 8 && matches!(group.local(), "scatterChart" | "bubbleChart") {
                    // Scatter/Bubble have a vertical Y and horizontal X, not a reversible
                    // pair. An ambiguous Y must not manufacture a unique horizontal Y.
                    if !matches!(axis.child(Local("axPos")).and_then(|p| p.attr_local("val")), Some("l" | "r"))
                        || !partner.is(Local("valAx"))
                        || !matches!(partner.child(Local("axPos")).and_then(|p| p.attr_local("val")), Some("b" | "t"))
                    {
                        return None;
                    }
                }
                Some(axis)
            };
            return match (candidate(first, second), candidate(second, first)) {
                (Some(axis), None) | (None, Some(axis)) => Some(axis),
                _ => None, // Missing, duplicate or two eligible directions retain the fallback.
            };
        }
        let value = group_value_axis(val_map, group)?;
        let cross = if value == first {
            second
        } else if value == second {
            first
        } else {
            return None;
        };
        let axis = self.values.get(&(value.as_str(), cross)).copied().flatten()?;
        // The reciprocal partner must itself be eligible and unique, not a foreign pair.
        self.partners.get(&(cross, value.as_str())).copied().flatten()?;
        Some(axis)
    }
}

/// The primary value axis is the one on the left (bottom for bar charts); the secondary one
/// sits on the right or top, or crosses the category axis at its maximum.
fn primary_value_axis<'a>(
    pa: &'a El,
    val_map: &std::collections::HashMap<&str, bool>,
    version: u16,
    relations: Option<&AxisRelations<'a>>,
) -> Option<&'a El> {
    let legacy = version == 0;
    let secondary_placed = |a: &El| {
        a.child(Local("crosses")).and_then(|c| c.attr_local("val")) == Some("max")
            || matches!(a.child(Local("axPos")).and_then(|c| c.attr_local("val")), Some("r" | "t"))
    };
    let val_axes_by_id: std::collections::HashMap<&str, &El> =
        pa.children(Local("valAx")).filter_map(|a| Some((a.child(Local("axId")).and_then(|c| c.attr_local("val"))?, a))).collect();
    let used: Vec<(String, Option<&El>)> = pa
        .elements()
        .filter(|g| {
            if legacy { g.local().ends_with("Chart") } else { group_chart_type(g.local(), g).is_some() && g.children(Local("ser")).next().is_some() }
        })
        .filter_map(|g| Some((group_value_axis(val_map, g)?, relations.and_then(|index| index.value_axis(g, val_map)))))
        .collect();
    let selected = |(id, resolved): &(String, Option<&'a El>)| (*resolved).or_else(|| val_axes_by_id.get(id.as_str()).copied());
    if let Some(axis) = used.iter().filter_map(selected).find(|axis| !secondary_placed(axis)) {
        return Some(axis);
    }
    if version >= 2 {
        // A secondary-only plot still owns the right/top pair. The writer keeps the unused
        // primary pair; require reciprocal IDs so a stray or incomplete axis cannot win.
        // The old predicate asked whether any partner matched. Index all reciprocal
        // relations, not only the last element for an ID; duplicates keep that semantics.
        let partners: std::collections::HashSet<(&str, &str)> = pa
            .elements()
            .filter_map(|a| {
                if !matches!(a.local(), "catAx" | "dateAx" | "valAx") {
                    return None;
                }
                let id = a.child(Local("axId"))?.attr_local("val")?;
                let horizontal = if version >= 4 {
                    !matches!(a.child(Local("axPos")).and_then(|p| p.attr_local("val")), Some("l" | "r"))
                } else {
                    // Preserve the exact earlier last-ID-map interpretation for native saves.
                    val_map.get(id) == Some(&false)
                };
                (!a.is(Local("valAx")) || horizontal).then_some((id, a.child(Local("crossAx"))?.attr_local("val")?))
            })
            .collect();
        if let Some(axis) = pa.children(Local("valAx")).find(|a| {
            let Some(id) = a.child(Local("axId")).and_then(|a| a.attr_local("val")) else { return false };
            let Some(cross) = a.child(Local("crossAx")).and_then(|a| a.attr_local("val")) else { return false };
            !secondary_placed(a)
                && matches!(a.child(Local("axPos")).and_then(|p| p.attr_local("val")), Some("l" | "b"))
                && partners.contains(&(cross, id))
        }) {
            return Some(axis);
        }
    }
    used.first().and_then(selected)
}

// Cache current marker inheritance once per group. The two older save projections keep
// their former interpretation, including a bare marker element's old false default.
fn group_metadata(name: &str, g: &El, version: u16) -> Option<(ChartType, bool)> {
    let kind = group_chart_type(name, g)?;
    let markers = version >= 2 && g.child(Local("marker")).is_some_and(|m| m.attr_local("val").and_then(value::to_bool).unwrap_or(true));
    let kind = if markers && kind == ChartType::Line { ChartType::LineMarkers } else { kind };
    Some((kind, markers))
}

/// The chart type of a plot group (`name` is its element name), or `None` for an unknown group.
fn group_chart_type(name: &str, g: &El) -> Option<ChartType> {
    let grouping = g.child(Local("grouping")).and_then(|c| c.attr_local("val")).unwrap_or("clustered");
    Some(match name {
        "barChart" | "bar3DChart" => {
            let bar = g.child(Local("barDir")).and_then(|c| c.attr_local("val")) == Some("bar");
            match (bar, grouping) {
                (false, "stacked") => ChartType::StackedColumn,
                (false, "percentStacked") => ChartType::PercentColumn,
                (false, _) => ChartType::Column,
                (true, "stacked") => ChartType::StackedBar,
                (true, "percentStacked") => ChartType::PercentBar,
                (true, _) => ChartType::Bar,
            }
        }
        "lineChart" | "line3DChart" => {
            let markers = g.child(Local("marker")).and_then(|c| c.attr_local("val")).is_some_and(|v| v == "1" || v == "true");
            match grouping {
                "stacked" | "percentStacked" => ChartType::StackedLine,
                _ if markers => ChartType::LineMarkers,
                _ => ChartType::Line,
            }
        }
        "areaChart" | "area3DChart" => {
            if matches!(grouping, "stacked" | "percentStacked") {
                ChartType::StackedArea
            } else {
                ChartType::Area
            }
        }
        "pieChart" | "pie3DChart" | "ofPieChart" => ChartType::Pie,
        "doughnutChart" => ChartType::Doughnut,
        "scatterChart" => ChartType::Scatter,
        "bubbleChart" => ChartType::Bubble,
        "radarChart" => {
            if g.child(Local("radarStyle")).and_then(|c| c.attr_local("val")) == Some("filled") {
                ChartType::FilledRadar
            } else {
                ChartType::Radar
            }
        }
        "stockChart" => ChartType::Stock,
        "surfaceChart" | "surface3DChart" => ChartType::Surface,
        _ => return None,
    })
}

/// Chart-wide settings a plot group carries (vary colours, gap, overlap, hole, data labels).
/// Returns the group's own data-label setting.
fn read_group_settings(g: &El, c: &mut Chart) -> Option<bool> {
    // A missing <c:varyColors> means "vary by point" (the schema default is true), as PowerPoint draws it.
    c.vary_colors = g.child(Local("varyColors")).and_then(|c| c.attr_local("val")).is_none_or(|v| v == "1" || v == "true");
    // A bar group without c:gapWidth / c:overlap takes the schema's defaults (150 %, 0 %),
    // not the 219 % / -27 % PowerPoint writes into its new charts.
    let bar_group = g.is(Local("barChart")) || g.is(Local("bar3DChart"));
    if let Some(v) = g.child(Local("gapWidth")).and_then(|c| c.attr_local("val")).and_then(|v| v.parse::<f64>().ok()) {
        c.gap_width = (v / 100.0).clamp(0.0, 5.0);
    } else if bar_group {
        c.gap_width = 1.5;
    }
    if let Some(v) = g.child(Local("overlap")).and_then(|c| c.attr_local("val")).and_then(|v| v.parse::<f64>().ok()) {
        c.overlap = (v / 100.0).clamp(-1.0, 1.0);
    } else if bar_group {
        c.overlap = 0.0;
    }
    if let Some(v) = g.child(Local("holeSize")).and_then(|c| c.attr_local("val")).and_then(|v| v.parse::<f64>().ok()) {
        c.hole_size = (v / 100.0).clamp(0.0, 0.9);
    }
    if let Some(v) = g.child(Local("firstSliceAng")).and_then(|c| c.attr_local("val")).and_then(|v| v.trim().parse::<u16>().ok()) {
        c.first_slice_angle = v % 360;
    }
    // `<c:showVal/>` without a val attribute means true, as in the schema's default.
    // The setting belongs to this plot group: a group that asks for no labels keeps them
    // off, even when another group of a combo chart asks for them (#868).
    let group_labels = g.path(&[Local("dLbls"), Local("showVal")]).map(|v| v.attr_local("val").and_then(value::to_bool).unwrap_or(true));
    if group_labels == Some(true) {
        c.data_labels = true;
    }
    group_labels
}

/// Data labels' number format: the series' labels', the group's, else the source data's.
fn label_number_format(s: &El, g: &El, version: u16) -> Option<String> {
    let label_fmt = |d: Option<&El>| {
        d.and_then(|d| d.child(Local("numFmt")))
            .filter(|n| if version >= 8 { !source_linked(n) } else { n.attr_local("sourceLinked") != Some("1") })
            .and_then(|n| n.attr_local("formatCode"))
            .map(str::to_string)
    };
    let source = s
        .child(Local("val"))
        .or_else(|| s.child(Local("yVal")))
        .and_then(|v| v.find(Local("numCache")).or_else(|| if version >= 9 { v.find(Local("numLit")) } else { None }))
        .and_then(|c| c.child(Local("formatCode")))
        .map(|f| f.deep_text());
    label_fmt(s.child(Local("dLbls"))).or_else(|| label_fmt(g.child(Local("dLbls")))).or(source).filter(|f| !f.is_empty())
}

/// Error bars on the values: a scatter chart's y bars, any other chart's only ones.
fn read_error_bars(s: &El, kind: ChartType, budget: &mut usize) -> Option<ErrorBars> {
    let e = s.children(Local("errBars")).find(|e| {
        e.child(Local("errDir")).and_then(|c| c.attr_local("val")).is_none_or(|d| d == "y" || !matches!(kind, ChartType::Scatter | ChartType::Bubble))
    })?;
    let d = ErrorBars::default();
    Some(ErrorBars {
        bar_type: e.child(Local("errBarType")).and_then(|c| c.attr_local("val")).unwrap_or("both").to_string(),
        val_type: e.child(Local("errValType")).and_then(|c| c.attr_local("val")).unwrap_or("fixedVal").to_string(),
        value: e
            .child(Local("val"))
            .and_then(|c| c.attr_local("val"))
            .and_then(|v| v.parse().ok())
            .filter(|v: &f64| v.is_finite())
            .unwrap_or(d.value),
        plus: e.child(Local("plus")).map(|p| numbers(p, budget)).unwrap_or_default(),
        minus: e.child(Local("minus")).map(|p| numbers(p, budget)).unwrap_or_default(),
        end_cap: e.child(Local("noEndCap")).and_then(|v| v.attr_local("val").and_then(value::to_bool)) != Some(true),
    })
}

/// One `c:ser` of group `g`. `group` is (chart type, on the secondary axis, the group's data-label
/// setting); the first series with categories sets `categories`.
fn read_series(s: &El, g: &El, group: (ChartType, bool, Option<bool>), categories: &mut Vec<String>, budget: &mut usize, version: u16) -> Series {
    let (kind, secondary, group_labels) = group;
    let marker = s.path(&[Local("marker"), Local("symbol")]).and_then(|m| m.attr_local("val")).map(String::from);
    let kind = if matches!(kind, ChartType::Line | ChartType::LineMarkers) {
        match marker.as_deref() {
            Some("none") => ChartType::Line,
            Some(_) => ChartType::LineMarkers,
            None => kind,
        }
    } else {
        kind
    };
    let mut ser = Series {
        name: s.child(Local("tx")).map(|t| strings(t, budget)).and_then(|v| v.into_iter().next()).unwrap_or_default(),
        fill: simple_fill(s.child(Local("spPr"))),
        line: simple_line(s.child(Local("spPr"))),
        smooth: s.child(Local("smooth")).and_then(|v| v.attr_local("val").and_then(value::to_bool)).unwrap_or(false),
        marker,
        kind: Some(kind),
        secondary_axis: secondary,
        ..Default::default()
    };
    // `<c:showVal/>` without a val attribute means true, as in the schema's default.
    if let Some(d) =
        s.child(Local("dLbls")).and_then(|d| d.child(Local("showVal"))).map(|v| v.attr_local("val").and_then(value::to_bool).unwrap_or(true))
    {
        ser.data_labels = Some(d);
    } else if let Some(false) = group_labels {
        // The group asks for no labels and the series does not say otherwise.
        ser.data_labels = Some(false);
    }
    for t in s.children(Local("trendline")).take(16) {
        if let Some(t) = trendline(t) {
            ser.trendlines.push(t);
        }
    }
    ser.number_format = label_number_format(s, g, version);
    for dp in s.children(Local("dPt")) {
        if let (Some(i), Some(f)) =
            (dp.child(Local("idx")).and_then(|i| i.attr_local("val").and_then(value::to_u32_lossy)), simple_fill(dp.child(Local("spPr"))))
        {
            ser.point_fills.push((i as usize, f));
        }
    }
    if let Some(cat) = s.child(Local("cat")).or_else(|| s.child(Local("xVal"))) {
        let cats = strings(cat, budget);
        if matches!(kind, ChartType::Scatter | ChartType::Bubble) {
            ser.x_values = cats.iter().map(|v| v.trim().parse::<f64>().ok()).collect();
        }
        if categories.is_empty() {
            *categories = cats;
        }
    }
    if let Some(v) = s.child(Local("val")).or_else(|| s.child(Local("yVal"))) {
        ser.values = numbers(v, budget);
    }
    ser.error_bars = read_error_bars(s, kind, budget);
    if let Some(b) = s.child(Local("bubbleSize")) {
        ser.sizes = numbers(b, budget);
    }
    let f = |e: Option<&El>| data_ref(e?);
    ser.refs = crate::chart::SeriesRefs {
        name: f(s.child(Local("tx"))),
        categories: f(s.child(Local("cat")).or_else(|| s.child(Local("xVal")))),
        values: f(s.child(Local("val")).or_else(|| s.child(Local("yVal")))),
        sizes: f(s.child(Local("bubbleSize"))),
    };
    ser
}

/// The formula of a data source (`c:numRef`, `c:strRef` or `c:multiLvlStrRef`), if it has one.
fn data_ref(e: &El) -> Option<String> {
    let r = e.elements().find(|r| matches!(r.local(), "numRef" | "strRef" | "multiLvlStrRef"))?;
    let f = r.child(Local("f"))?.text();
    let f = f.trim();
    // Formulas are short; a huge one is not kept.
    (!f.is_empty() && f.len() <= 8192).then(|| f.to_string())
}

/// Axis scaling, titles, number formats and visibility. Returns the primary value axis and the
/// category axis.
fn read_axes<'a>(
    c: &mut Chart,
    pa: &'a El,
    primary_val: &Option<String>,
    primary_axes: (Option<&'a El>, Option<&'a El>),
    val_map: &std::collections::HashMap<&str, bool>,
    version: u16,
    relations: Option<&AxisRelations<'a>>,
) -> (Option<&'a El>, Option<&'a El>) {
    let (primary_axis, resolved_primary) = primary_axes;
    let legacy = version == 0;
    let vertical = |a: &&El| matches!(a.child(Local("axPos")).and_then(|c| c.attr_local("val")), Some("l" | "r"));
    let by_id = |id: &Option<String>| {
        pa.children(Local("valAx")).find(|a| id.is_some() && a.child(Local("axId")).and_then(|c| c.attr_local("val")) == id.as_deref())
    };
    // Earlier native projections deliberately preserve their first-by-ID interpretation.
    let selected = if version >= 5 { primary_axis } else { None };
    let val_ax =
        selected.or_else(|| by_id(primary_val)).or_else(|| pa.children(Local("valAx")).find(vertical)).or_else(|| pa.children(Local("valAx")).last());
    let num = |a: &El, k: &str| a.child(Local(k)).and_then(|c| c.attr_local("val")).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite());
    // An axis' number format: its own unless linked to the source (`sourceLinked="1"`, or no
    // c:numFmt at all), in which case the data's format of the first series plotted on it.
    let source_format = |ax: &El| {
        let id = ax.child(Local("axId")).and_then(|c| c.attr_local("val"));
        let owns_axis = |g: &El| {
            version >= 7
                && group_chart_type(g.local(), g).is_some()
                && g.children(Local("ser")).next().is_some()
                && relations.and_then(|index| index.value_axis(g, val_map)).is_some_and(|a| std::ptr::eq(a, ax))
        };
        let resolved_owner = pa.elements().any(owns_axis);
        pa.elements()
            .filter(|g| {
                if resolved_owner {
                    owns_axis(g)
                } else {
                    // An unresolved group may use numeric IDs, but a resolved group belongs
                    // to its actual element even when another axis shares that numeric ID.
                    (version < 10 || relations.and_then(|index| index.value_axis(g, val_map)).is_none())
                        && g.children(Local("axId")).any(|a| a.attr_local("val") == id)
                }
            })
            .flat_map(|g| g.children(Local("ser")))
            .find_map(|s| {
                s.child(Local("val"))
                    .or_else(|| s.child(Local("yVal")))
                    .and_then(|v| v.find(Local("numCache")).or_else(|| if version >= 10 { v.child(Local("numLit")) } else { None }))
                    .and_then(|c| c.child(Local("formatCode")))
                    .map(|f| f.deep_text())
                    .filter(|f| version < 10 || !f.is_empty())
            })
            .filter(|f| !f.is_empty() && (version >= 10 || f != "General"))
    };
    let format_code = |a: &El| match a.child(Local("numFmt")) {
        Some(n) if if version >= 8 { !source_linked(n) } else { n.attr_local("sourceLinked") != Some("1") } => {
            n.attr_local("formatCode").map(str::to_string)
        }
        n => source_format(a).or_else(|| n.and_then(|n| n.attr_local("formatCode")).map(str::to_string)),
    };
    // `c:tickLblPos val="none"` hides the labels just as `c:delete` does.
    let deleted = |a: &El| {
        a.child(Local("delete")).and_then(|d| d.attr_local("val").and_then(value::to_bool)) == Some(true)
            || a.child(Local("tickLblPos")).and_then(|c| c.attr_local("val")) == Some("none")
    };
    // A gridline whose `c:spPr/a:ln` has `a:noFill` is not drawn, though the element is there.
    let grid = |a: &El, k: &str| {
        a.child(Local(k))
            .is_some_and(|g| g.child(Local("spPr")).and_then(|p| p.child(Local("ln"))).is_none_or(|l| l.child(Local("noFill")).is_none()))
    };
    if c.series.iter().any(|s| s.secondary_axis) {
        let primary_id = val_ax.and_then(|a| a.child(Local("axId")).and_then(|c| c.attr_local("val")));
        let resolved_secondary = if version >= 10 {
            // An exact secondary owner outranks an earlier unresolved ID-only candidate,
            // including when the primary is an unused anchor with no resolved group.
            pa.elements()
                .filter(|g| group_chart_type(g.local(), g).is_some() && g.children(Local("ser")).next().is_some())
                .filter(|g| group_secondary(g, resolved_primary, primary_id, val_map, relations, version))
                .find_map(|g| relations.and_then(|index| index.value_axis(g, val_map)))
        } else {
            resolved_primary.and_then(|primary| {
                pa.elements()
                    .filter(|g| group_chart_type(g.local(), g).is_some() && g.children(Local("ser")).next().is_some())
                    .filter_map(|g| relations.and_then(|index| index.value_axis(g, val_map)))
                    .find(|axis| !std::ptr::eq(*axis, primary))
            })
        };
        let secondary_axis = resolved_secondary.or_else(|| {
            if legacy {
                pa.children(Local("valAx")).find(|a| a.child(Local("axId")).and_then(|c| c.attr_local("val")) != primary_id && vertical(a))
            } else {
                // Resolve the other used value axis through its group, excluding unused axes and
                // the horizontal x axis of a scatter group.
                let secondary_group =
                    pa.elements().filter(|g| group_chart_type(g.local(), g).is_some() && g.children(Local("ser")).next().is_some()).find(|g| {
                        if version >= 8 {
                            group_secondary(g, resolved_primary, primary_id, val_map, relations, version)
                        } else {
                            group_value_axis(val_map, g).is_some_and(|id| Some(id.as_str()) != primary_id)
                        }
                    });
                secondary_group.and_then(|g| {
                    relations.and_then(|index| index.value_axis(g, val_map)).or_else(|| {
                        let id = group_value_axis(val_map, g)?;
                        pa.children(Local("valAx")).find(|a| a.child(Local("axId")).and_then(|c| c.attr_local("val")) == Some(id.as_str()))
                    })
                })
            }
        });
        if let Some(a) = secondary_axis {
            let sc = a.child(Local("scaling"));
            c.y2_min = sc.and_then(|s| num(s, "min"));
            c.y2_max = sc.and_then(|s| num(s, "max"));
            c.y2_major = num(a, "majorUnit");
            c.y2_format = format_code(a);
            c.y2_hidden = deleted(a);
        }
    }
    let cat_ax = if legacy {
        pa.child(Local("catAx")).or_else(|| pa.child(Local("dateAx"))).or_else(|| pa.children(Local("valAx")).find(|a| !vertical(a)))
    } else {
        val_ax
            .and_then(|a| category_axis_for_value(pa, a, version >= 4))
            .or_else(|| {
                primary_val.as_deref().and_then(|id| {
                    pa.elements()
                        .find(|g| {
                            group_chart_type(g.local(), g).is_some()
                                && g.children(Local("ser")).next().is_some()
                                && g.children(Local("axId")).any(|a| a.attr_local("val") == Some(id))
                        })
                        .and_then(|g| category_axis(pa, g))
                })
            })
            .or_else(|| {
                if primary_val.is_none() {
                    pa.child(Local("catAx")).or_else(|| pa.child(Local("dateAx"))).or_else(|| pa.children(Local("valAx")).find(|a| !vertical(a)))
                } else {
                    None
                }
            })
    };
    if let Some(v) = val_ax {
        c.gridlines = grid(v, "majorGridlines");
        c.minor_gridlines = grid(v, "minorGridlines");
        let sc = v.child(Local("scaling"));
        c.y_min =
            sc.and_then(|s| s.child(Local("min")).and_then(|c| c.attr_local("val"))).and_then(|v| v.parse().ok()).filter(|v: &f64| v.is_finite());
        c.y_max =
            sc.and_then(|s| s.child(Local("max")).and_then(|c| c.attr_local("val"))).and_then(|v| v.parse().ok()).filter(|v: &f64| v.is_finite());
        c.axis_titles.1 = v.child(Local("title")).map(|t| rich_text(t).unwrap_or_else(|| "Axis Title".into()));
        c.y_major = num(v, "majorUnit");
        c.y_log_base = sc
            .and_then(|s| s.child(Local("logBase")).and_then(|c| c.attr_local("val")))
            .and_then(|v| v.parse().ok())
            .filter(|b: &f64| b.is_finite() && *b >= 2.0);
        c.y_format = format_code(v);
        c.y_hidden = deleted(v);
    }
    if let Some(a) = cat_ax {
        c.cat_reversed =
            a.child(Local("scaling")).and_then(|s| s.child(Local("orientation")).and_then(|c| c.attr_local("val"))).is_some_and(|o| o == "maxMin");
        c.axis_titles.0 = a.child(Local("title")).map(|t| rich_text(t).unwrap_or_else(|| "Axis Title".into()));
    }
    (val_ax, cat_ax)
}

/// The category axis (or scatter x axis) referenced by a plot group.
fn category_axis<'a>(pa: &'a El, group: &El) -> Option<&'a El> {
    let ids: Vec<&str> = group.children(Local("axId")).filter_map(|id| id.attr_local("val")).collect();
    pa.elements()
        .find(|axis| {
            matches!(axis.local(), "catAx" | "dateAx")
                && axis.child(Local("axId")).and_then(|id| id.attr_local("val")).is_some_and(|id| ids.contains(&id))
        })
        .or_else(|| {
            pa.children(Local("valAx")).find(|axis| {
                matches!(axis.child(Local("axPos")).and_then(|pos| pos.attr_local("val")), Some("b" | "t"))
                    && axis.child(Local("axId")).and_then(|id| id.attr_local("val")).is_some_and(|id| ids.contains(&id))
            })
        })
}

/// The category partner of this exact value axis. Versions 1–3 use the ID-only path;
/// version 4 and later prefer the reciprocal partner, then retain that fallback.
fn category_axis_for_value<'a>(pa: &'a El, value_axis: &El, reciprocal: bool) -> Option<&'a El> {
    let id = value_axis.child(Local("crossAx"))?.attr_local("val")?;
    let eligible = |axis: &&El| {
        matches!(axis.local(), "catAx" | "dateAx" | "valAx")
            && (!axis.is(Local("valAx")) || !matches!(axis.child(Local("axPos")).and_then(|a| a.attr_local("val")), Some("l" | "r")))
            && axis.child(Local("axId")).and_then(|a| a.attr_local("val")) == Some(id)
    };
    let paired = if reciprocal {
        let value_id = value_axis.child(Local("axId")).and_then(|a| a.attr_local("val"));
        pa.elements().find(|axis| eligible(axis) && value_id.is_some() && axis.child(Local("crossAx")).and_then(|a| a.attr_local("val")) == value_id)
    } else {
        None
    };
    paired.or_else(|| pa.elements().find(eligible))
}

/// Resolve the pair belonging to the group that actually supplied the categories.
fn group_category_axis<'a>(
    pa: &'a El,
    group: &El,
    primary_axis: Option<&'a El>,
    val_map: &std::collections::HashMap<&str, bool>,
    relations: Option<&AxisRelations<'a>>,
) -> Option<&'a El> {
    let value_id = group_value_axis(val_map, group);
    let value_axis = relations.and_then(|index| index.value_axis(group, val_map)).or_else(|| {
        primary_axis.filter(|a| value_id.is_some() && a.child(Local("axId")).and_then(|a| a.attr_local("val")) == value_id.as_deref()).or_else(|| {
            pa.children(Local("valAx"))
                .find(|a| value_id.is_some() && a.child(Local("axId")).and_then(|a| a.attr_local("val")) == value_id.as_deref())
        })
    });
    value_axis
        .and_then(|a| category_axis_for_value(pa, a, true))
        .filter(|a| {
            let id = a.child(Local("axId")).and_then(|a| a.attr_local("val"));
            id.is_some() && group.children(Local("axId")).any(|a| a.attr_local("val") == id)
        })
        .or_else(|| category_axis(pa, group))
}

/// CT_NumFmt sourceLinked defaults true; boolean spellings share one current interpretation.
fn source_linked(numfmt: &El) -> bool {
    numfmt.attr_local("sourceLinked").and_then(value::to_bool).unwrap_or(true)
}

/// The number format of the category labels, for categories that are numbers or dates: the category
/// axis' own format, unless it is linked to the source (`sourceLinked`, or no `c:numFmt`), in which
/// case the format code of the category cells. `None` when the categories are text (their cached
/// strings show as they are) or the file gives only General.
fn category_format(cat_ax: Option<&El>, series: &El, version: u16) -> Option<String> {
    // Called only when the budgeted series read first supplies categories. Never rescan caches
    // or take a format from a skipped series or another plot group's axis.
    let cat = series.child(Local("cat")).or_else(|| series.child(Local("xVal")))?;
    if cat.find(Local("strCache")).or_else(|| cat.find(Local("strLit"))).is_some() {
        return None;
    }
    let cache = cat.find(Local("numCache")).or_else(|| cat.find(Local("numLit")))?;
    let source = cache.child(Local("formatCode")).map(|f| f.deep_text()).filter(|f| !f.is_empty() && (version >= 9 || f != "General"));
    let numfmt = cat_ax.and_then(|a| a.child(Local("numFmt")));
    let own = numfmt.and_then(|n| n.attr_local("formatCode")).filter(|f| !f.is_empty()).map(str::to_string);
    match numfmt {
        // An explicit format wins, "General" included (serial numbers, not dates).
        Some(n) if if version >= 8 { !source_linked(n) } else { n.attr_local("sourceLinked").and_then(value::to_bool) != Some(true) } => {
            own.or(source)
        }
        _ => source.or(own.filter(|f| f != "General")).filter(|f| version < 9 || f != "General"),
    }
}

/// Text sizes and colours as the file sets them.
fn read_text_styles(c: &mut Chart, root: &El, ch: &El, val_ax: Option<&El>, cat_ax: Option<&El>) {
    // Text sizes as the file sets them (PowerPoint draws chart text at these sizes, not scaled to
    // the chart): the axes' and legend's, else the whole chart's; the title's own.
    let chart_size = text_size(root.child(Local("txPr")));
    c.text_size = [val_ax, cat_ax, ch.child(Local("legend"))].into_iter().flatten().find_map(|e| text_size(e.child(Local("txPr")))).or(chart_size);
    // Text colour likewise: the axes' and legend's, else the whole chart's.
    c.text_color = [val_ax, cat_ax, ch.child(Local("legend")), Some(root)].into_iter().flatten().find_map(|e| text_color(e.child(Local("txPr"))));
    c.title_size = ch
        .child(Local("title"))
        .and_then(|t| text_size(t.child(Local("tx")).and_then(|x| x.child(Local("rich")))).or_else(|| text_size(t.child(Local("txPr")))));
}

fn trend_num(e: &El, k: &str) -> Option<f64> {
    e.child(Local(k)).and_then(|c| c.attr_local("val")).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && v.abs() <= 1.0e12)
}

fn trend_name(s: &str) -> Option<String> {
    let cleaned: String =
        s.chars().filter(|c| matches!(c, '\t' | '\n' | '\r') || (*c >= ' ' && *c != '\u{FFFE}' && *c != '\u{FFFF}')).take(255).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() { None } else { Some(cleaned.to_string()) }
}

/// One `c:trendline`. Unknown types are skipped.
fn trendline(e: &El) -> Option<Trendline> {
    let kind = TrendlineType::from_xml(e.child(Local("trendlineType")).and_then(|c| c.attr_local("val"))?)?;
    Some(Trendline {
        kind,
        name: e.child(Local("name")).map(|n| n.deep_text()).as_deref().and_then(trend_name),
        order: if kind == TrendlineType::Poly {
            e.child(Local("order")).and_then(|o| o.attr_local("val").and_then(value::to_u32_lossy)).map(|v| v.clamp(2, 6))
        } else {
            None
        },
        period: if kind == TrendlineType::MovingAvg {
            e.child(Local("period")).and_then(|o| o.attr_local("val").and_then(value::to_u32_lossy)).map(|v| v.clamp(2, 255))
        } else {
            None
        },
        forward: trend_num(e, "forward"),
        backward: trend_num(e, "backward"),
        intercept: trend_num(e, "intercept"),
        disp_eq: e.child(Local("dispEq")).and_then(|d| d.attr_local("val").and_then(value::to_bool)).unwrap_or(false),
        disp_r_sqr: e.child(Local("dispRSqr")).and_then(|d| d.attr_local("val").and_then(value::to_bool)).unwrap_or(false),
    })
}

/// The colour of the first `a:defRPr` or `a:rPr` with a solid fill under `e`.
fn text_color(e: Option<&El>) -> Option<crate::style::ColorRef> {
    fn rec(e: &El, depth: usize) -> Option<crate::style::ColorRef> {
        if depth > 8 {
            return None;
        }
        for c in e.elements() {
            if (c.is(Local("defRPr")) || c.is(Local("rPr")))
                && let Some(Fill::Solid { color }) = simple_fill(Some(c))
            {
                return Some(color);
            }
            if let Some(v) = rec(c, depth + 1) {
                return Some(v);
            }
        }
        None
    }
    rec(e?, 0)
}

/// Legend entries read from one chart; a legend lists at most one per series or category.
const MAX_LEGEND_ENTRIES: usize = 1024;

/// The legend's entries with text of their own (`c:legendEntry/c:txPr`), over the legend's.
fn legend_entries(legend: Option<&El>) -> Vec<crate::chart::LegendEntry> {
    legend
        .into_iter()
        .flat_map(|l| l.children(Local("legendEntry")))
        .take(MAX_LEGEND_ENTRIES)
        .filter_map(|e| {
            // The paragraph's own run properties first, then the list style's defaults
            // (`a:lstStyle` comes before `a:p`, and is the less specific of the two).
            let tx = e.child(Local("txPr"));
            let para = tx.and_then(|t| t.child(Local("p")));
            let entry = crate::chart::LegendEntry {
                idx: e.child(Local("idx")).and_then(|i| i.attr_local("val")).and_then(|v| v.parse().ok())?,
                size: text_size(para).or_else(|| text_size(tx)),
                bold: text_bold(para).or_else(|| text_bold(tx)),
                color: text_color(para).or_else(|| text_color(tx)),
            };
            (entry.size.is_some() || entry.bold.is_some() || entry.color.is_some()).then_some(entry)
        })
        .collect()
}

/// The weight (`b`) of the first `a:defRPr` or `a:rPr` that sets one under `e`.
fn text_bold(e: Option<&El>) -> Option<bool> {
    fn rec(e: &El, depth: usize) -> Option<bool> {
        if depth > 8 {
            return None;
        }
        for c in e.elements() {
            if (c.is(Local("defRPr")) || c.is(Local("rPr")))
                && let Some(b) = c.attr_local("b").and_then(value::to_bool)
            {
                return Some(b);
            }
            if let Some(v) = rec(c, depth + 1) {
                return Some(v);
            }
        }
        None
    }
    rec(e?, 0)
}

/// The size in points of the first `a:defRPr` or `a:rPr` with one under `e` (`sz` in hundredths).
fn text_size(e: Option<&El>) -> Option<f64> {
    fn rec(e: &El, depth: usize) -> Option<f64> {
        if depth > 8 {
            return None;
        }
        for c in e.elements() {
            if (c.is(Local("defRPr")) || c.is(Local("rPr")))
                && let Some(v) = c.attr_local("sz").and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v > 0.0)
            {
                return Some((v / 100.0).clamp(1.0, 400.0));
            }
            if let Some(v) = rec(c, depth + 1) {
                return Some(v);
            }
        }
        None
    }
    rec(e?, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Original small two-pair fixture: only the secondary series supplies categories.
    fn review6_axis_pairs() -> String {
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
<c:chart><c:plotArea>
<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>
<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>Primary</c:v></c:tx><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>5</c:v></c:pt></c:numLit></c:val></c:ser>
<c:axId val="10"/><c:axId val="20"/></c:barChart>
<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>
<c:ser><c:idx val="1"/><c:order val="1"/><c:tx><c:v>Secondary</c:v></c:tx><c:cat><c:numLit><c:formatCode>General</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>45292</c:v></c:pt></c:numLit></c:cat><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>8</c:v></c:pt></c:numLit></c:val></c:ser>
<c:axId val="30"/><c:axId val="40"/></c:barChart>
<c:catAx><c:axId val="10"/><c:axPos val="b"/><c:crossAx val="20"/></c:catAx>
<c:valAx><c:axId val="20"/><c:scaling><c:max val="50"/></c:scaling><c:axPos val="l"/><c:crossAx val="10"/></c:valAx>
<c:catAx><c:axId val="30"/><c:axPos val="t"/><c:crossAx val="40"/><c:numFmt formatCode="mmm-yy" sourceLinked="0"/></c:catAx>
<c:valAx><c:axId val="40"/><c:scaling><c:max val="1"/></c:scaling><c:axPos val="r"/><c:crossAx val="999"/><c:numFmt formatCode="0.00" sourceLinked="0"/></c:valAx>
<c:valAx><c:axId val="40"/><c:scaling><c:max val="90"/></c:scaling><c:axPos val="r"/><c:crossAx val="30"/><c:numFmt formatCode="0.000" sourceLinked="0"/></c:valAx>
</c:plotArea></c:chart></c:chartSpace>
"#.into()
    }

    #[test]
    fn review6_secondary_scaling_uses_the_group_owned_reciprocal_value_element() {
        let xml = review6_axis_pairs();
        let bad = r#"<c:valAx><c:axId val="40"/><c:scaling><c:max val="1"/></c:scaling><c:axPos val="r"/><c:crossAx val="999"/><c:numFmt formatCode="0.00" sourceLinked="0"/></c:valAx>"#;
        let good = r#"<c:valAx><c:axId val="40"/><c:scaling><c:max val="90"/></c:scaling><c:axPos val="r"/><c:crossAx val="30"/><c:numFmt formatCode="0.000" sourceLinked="0"/></c:valAx>"#;
        let category =
            r#"<c:catAx><c:axId val="30"/><c:axPos val="t"/><c:crossAx val="40"/><c:numFmt formatCode="mmm-yy" sourceLinked="0"/></c:catAx>"#;
        let foreign = r#"<c:catAx><c:axId val="999"/><c:axPos val="t"/><c:crossAx val="40"/></c:catAx>"#;
        for reverse in [false, true] {
            for stray_reciprocal in [false, true] {
                let fixture = if reverse { xml.replace(&format!("{bad}\n{good}"), &format!("{good}\n{bad}")) } else { xml.clone() };
                let fixture = if stray_reciprocal { fixture.replace(category, &format!("{foreign}{category}")) } else { fixture };
                let c = read_chart_xml(fixture.as_bytes()).unwrap();
                assert_eq!(c.y_max, Some(50.0));
                assert_eq!(c.series.iter().map(|s| s.secondary_axis).collect::<Vec<_>>(), [false, true]);
                assert_eq!(c.y2_max, Some(90.0), "reverse={reverse}, stray={stray_reciprocal}");
                assert_eq!(c.y2_format.as_deref(), Some("0.000"));
                assert_eq!(c.category_format.as_deref(), Some("mmm-yy"));
            }
        }
        // No unique group-owned pair: retain the old first-ID fallback, not a guessed pair.
        let ambiguous = xml.replace(good, &format!("{good}{}", good.replace(r#"val="90""#, r#"val="80""#)));
        let no_partner = xml.replace(category, "");
        let too_many_ids = xml.replace(
            r#"<c:axId val="30"/><c:axId val="40"/></c:barChart>"#,
            r#"<c:axId val="30"/><c:axId val="40"/><c:axId val="70"/></c:barChart>"#,
        );
        for fixture in [ambiguous, no_partner, too_many_ids] {
            assert_eq!(read_chart_xml(fixture.as_bytes()).unwrap().y2_max, Some(1.0), "unresolved evidence retains the historical fallback");
        }
        let repeated_id = xml.replace(
            r#"<c:axId val="30"/><c:axId val="40"/></c:barChart>"#,
            r#"<c:axId val="30"/><c:axId val="40"/><c:axId val="30"/></c:barChart>"#,
        );
        assert_eq!(read_chart_xml(repeated_id.as_bytes()).unwrap().y2_max, Some(90.0), "two unique IDs still define one pair");
    }

    #[test]
    fn review6_secondary_category_format_keeps_the_selected_value_element() {
        let xml = review6_axis_pairs();
        let bad = r#"<c:valAx><c:axId val="40"/><c:scaling><c:max val="1"/></c:scaling><c:axPos val="r"/><c:crossAx val="999"/><c:numFmt formatCode="0.00" sourceLinked="0"/></c:valAx>"#;
        let category =
            r#"<c:catAx><c:axId val="30"/><c:axPos val="t"/><c:crossAx val="40"/><c:numFmt formatCode="mmm-yy" sourceLinked="0"/></c:catAx>"#;
        let unrelated =
            r#"<c:catAx><c:axId val="30"/><c:axPos val="t"/><c:crossAx val="777"/><c:numFmt formatCode="0.00" sourceLinked="0"/></c:catAx>"#;
        let fixture = xml.replace(category, &format!("{unrelated}{category}"));
        // Removing the invalid value is a legitimate existing reciprocal-category control.
        for fixture in [fixture.clone(), fixture.replace(bad, "")] {
            let c = read_chart_xml(fixture.as_bytes()).unwrap();
            assert_eq!(c.category_format.as_deref(), Some("mmm-yy"));
            assert_eq!(c.category_label(0), "Jan-24");
            assert_eq!(c.y2_max, Some(90.0));
        }
    }

    #[test]
    fn review6_primary_used_group_avoids_an_invalid_last_duplicate_and_keeps_old_raw() {
        let xml = review6_axis_pairs();
        let bad_secondary = r#"<c:valAx><c:axId val="40"/><c:scaling><c:max val="1"/></c:scaling><c:axPos val="r"/><c:crossAx val="999"/><c:numFmt formatCode="0.00" sourceLinked="0"/></c:valAx>"#;
        let good = r#"<c:valAx><c:axId val="20"/><c:scaling><c:max val="50"/></c:scaling><c:axPos val="l"/><c:crossAx val="10"/></c:valAx>"#;
        let bad = r#"<c:valAx><c:axId val="20"/><c:scaling><c:max val="1"/></c:scaling><c:axPos val="l"/><c:crossAx val="999"/></c:valAx>"#;
        let xml = xml.replace(bad_secondary, "").replace(good, &format!("{good}{bad}"));
        for fixture in [xml.clone(), xml.replace(&format!("{good}{bad}"), &format!("{bad}{good}"))] {
            let c = read_chart_xml(fixture.as_bytes()).unwrap();
            assert_eq!(c.y_max, Some(50.0));
            assert_eq!(c.y2_max, Some(90.0));
            assert_eq!(c.series.iter().map(|s| s.secondary_axis).collect::<Vec<_>>(), [false, true]);
        }
        let xml = xml.replace("</c:chartSpace>", r#"<c:extLst><c:ext uri="urn:craft:original-axis-element"><own:preserved xmlns:own="urn:craft:original"/></c:ext></c:extLst></c:chartSpace>"#);
        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        for version in 0..=5 {
            let mut previous = read_chart(&root, MAX_CHART_POINTS, version).unwrap();
            // The exact earlier first/last-ID interpretations are preserved, not upgraded.
            assert_eq!(previous.y_max, Some(if version >= 5 { 1.0 } else { 50.0 }), "interpretation{version}");
            previous.raw = Some(xml.clone());
            let previous: Chart = serde_json::from_str(&serde_json::to_string(&previous).unwrap()).unwrap();
            let (written, kept) = crate::chart::write::chart_xml(&previous);
            assert!(kept, "unchanged interpretation{version} must retain raw XML");
            assert!(crate::xmlx::parse(&written).unwrap().find(Local("preserved")).is_some());
        }
    }

    #[test]
    fn point_count_charges_every_retained_vector() {
        let mut c = Chart { categories: vec!["a".into()], category_levels: vec![vec!["x".into(); 2], vec!["y".into(); 3]], ..Default::default() };
        let mut s = crate::chart::Series { values: vec![Some(1.0); 4], x_values: vec![None; 5], sizes: vec![None; 6], ..Default::default() };
        s.error_bars = Some(crate::chart::ErrorBars { plus: vec![None; 7], minus: vec![None; 8], ..Default::default() });
        c.series = vec![s];
        assert_eq!(point_count(&c), 1 + 1 + 5 + 4 + 5 + 6 + 15);
    }

    #[test]
    fn sparse_caches_share_one_chart_wide_budget() {
        // Every series declares 100000 points but writes one: without a chart-wide budget each
        // would expand to the full count.
        let ser = |n: usize| {
            format!(
                r#"<c:ser><c:idx val="{n}"/><c:val><c:numRef><c:numCache><c:ptCount val="100000"/><c:pt idx="0"><c:v>{n}</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#
            )
        };
        let sers: String = (0..5).map(ser).collect();
        let xml = format!(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/>{sers}</c:barChart></c:plotArea></c:chart></c:chartSpace>"#
        );
        let doc = crate::xmlx::parse(xml.as_bytes()).unwrap();
        let c = chart_within(&doc, 250_000).unwrap();
        let total: usize = c.series.iter().map(|s| s.values.len()).sum();
        assert!(total <= 250_000, "{total}");
        assert_eq!(c.series.len(), 5);
        assert_eq!(c.series[0].values.len(), 100_000);
        assert_eq!(c.series[0].values[0], Some(0.0));
        assert!(c.series[4].values.is_empty(), "the budget ran out before the last series");
        // A small chart is untouched by the default budget.
        let c = chart_from(&doc).unwrap();
        assert!(c.series.iter().all(|s| s.values.len() == 100_000));
    }

    #[test]
    fn minor_gridlines_are_read_and_written_back() {
        let mut c = Chart::sample(ChartType::Line);
        c.minor_gridlines = true;
        let back = read_chart_xml(&crate::chart::write::chart_xml(&c).0).unwrap();
        assert!(back.gridlines && back.minor_gridlines);
        c.minor_gridlines = false;
        assert!(!read_chart_xml(&crate::chart::write::chart_xml(&c).0).unwrap().minor_gridlines);
    }

    #[test]
    fn series_pattern_fills_are_read_and_written_back() {
        // A pattern-filled series and an empty one with an outline, as PowerPoint writes them.
        let xml = br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:spPr><a:pattFill prst="openDmnd"><a:fgClr><a:schemeClr val="accent4"/></a:fgClr><a:bgClr><a:schemeClr val="bg1"/></a:bgClr></a:pattFill></c:spPr><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>5</c:v></c:pt></c:numLit></c:val></c:ser><c:ser><c:idx val="1"/><c:spPr><a:noFill/><a:ln w="19050"><a:solidFill><a:schemeClr val="accent4"/></a:solidFill></a:ln></c:spPr><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>3</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        let c = read_chart_xml(xml).unwrap();
        let accent4 = crate::style::ColorRef::scheme(crate::color::SchemeSlot::Accent4);
        let pattern = crate::style::PatternFill {
            preset: "openDmnd".into(),
            fg: accent4.clone(),
            bg: crate::style::ColorRef::scheme(crate::color::SchemeSlot::Bg1),
        };
        assert_eq!(c.series[0].fill, Some(Fill::Pattern(pattern.clone())));
        assert_eq!(c.series[1].fill, Some(Fill::None));
        assert_eq!(c.series[1].line.as_ref().and_then(|l| l.fill.clone()), Some(Fill::solid(accent4)));
        // Written out from the model (an edited chart), the pattern survives.
        let mut edited = c.clone();
        edited.raw = None;
        edited.title = Some("Edited".into());
        let back = read_chart_xml(&crate::chart::write::chart_xml(&edited).0).unwrap();
        assert_eq!(back.series[0].fill, Some(Fill::Pattern(pattern)));
        assert_eq!(back.series[1].fill, Some(Fill::None));
    }

    #[test]
    fn log_base_is_read() {
        let xml = br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:lineChart><c:axId val="1"/><c:axId val="2"/></c:lineChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:logBase val="10"/><c:orientation val="minMax"/></c:scaling></c:valAx></c:plotArea></c:chart></c:chartSpace>"#;
        assert_eq!(read_chart_xml(xml).unwrap().y_log_base, Some(10.0));
    }

    #[test]
    fn error_bars_are_read() {
        let xml = br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:errBars><c:errBarType val="plus"/><c:errValType val="percentage"/><c:noEndCap val="1"/><c:val val="20"/></c:errBars><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>5</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        let c = read_chart_xml(xml).unwrap();
        let e = c.series[0].error_bars.clone().unwrap();
        assert_eq!((e.bar_type.as_str(), e.val_type.as_str(), e.value, e.end_cap), ("plus", "percentage", 20.0, false));
    }

    #[test]
    fn legend_position_and_overlay_are_read() {
        let legend = r#"<c:legend><c:legendPos val="r"/><c:overlay val="1"/></c:legend>"#;
        let xml = format!(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/></c:barChart></c:plotArea>{legend}</c:chart></c:chartSpace>"#
        );
        let c = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(c.legend.as_deref(), Some("r"));
        assert!(c.legend_overlay);
        let xml = br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/></c:barChart></c:plotArea><c:legend/></c:chart></c:chartSpace>"#;
        let c = read_chart_xml(xml).unwrap();
        assert_eq!(c.legend.as_deref(), Some("r"));
        assert!(!c.legend_overlay);
    }

    #[test]
    fn combo_chart_with_a_secondary_axis() {
        // Columns on axes 1/2, a line on axes 3/4 whose value axis is on the right.
        let xml = br##"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:lang val="ru-RU"/><c:chart><c:plotArea>
<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:tx><c:v>Spend</c:v></c:tx><c:cat><c:strLit><c:ptCount val="2"/><c:pt idx="0"><c:v>W1</c:v></c:pt><c:pt idx="1"><c:v>W2</c:v></c:pt></c:strLit></c:cat><c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>200000</c:v></c:pt><c:pt idx="1"><c:v>310000</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart>
<c:lineChart><c:grouping val="standard"/><c:ser><c:idx val="1"/><c:tx><c:v>Clicks</c:v></c:tx><c:spPr><a:ln w="28575"><a:solidFill><a:srgbClr val="00B050"/></a:solidFill></a:ln></c:spPr><c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>6000</c:v></c:pt><c:pt idx="1"><c:v>7400</c:v></c:pt></c:numLit></c:val></c:ser><c:marker val="1"/><c:axId val="3"/><c:axId val="4"/></c:lineChart>
<c:catAx><c:axId val="1"/><c:axPos val="b"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:axPos val="l"/><c:numFmt formatCode="#,##0" sourceLinked="1"/><c:crossAx val="1"/><c:majorUnit val="100000"/></c:valAx>
<c:valAx><c:axId val="4"/><c:scaling><c:max val="8000"/></c:scaling><c:axPos val="r"/><c:numFmt formatCode="0.0" sourceLinked="0"/><c:crossAx val="3"/></c:valAx><c:catAx><c:axId val="3"/><c:delete val="1"/><c:axPos val="b"/><c:crossAx val="4"/></c:catAx>
</c:plotArea></c:chart></c:chartSpace>"##;
        let c = read_chart_xml(xml).unwrap();
        assert_eq!(c.kind, ChartType::Combo);
        assert_eq!(c.series.len(), 2);
        assert!(!c.series[0].secondary_axis);
        assert!(c.series[1].secondary_axis);
        assert_eq!(c.series[1].kind, Some(ChartType::LineMarkers));
        assert!(c.series[1].line.as_ref().is_some_and(|l| l.width.is_some_and(|w| (w - 2.25).abs() < 0.01)));
        // Each value axis' number format, bounds and unit; the chart's language.
        assert_eq!(c.y_format.as_deref(), Some("#,##0"));
        assert_eq!((c.y_major, c.y2_max, c.y2_format.as_deref()), (Some(100000.0), Some(8000.0), Some("0.0")));
        assert_eq!(c.lang.as_deref(), Some("ru-RU"));
        // Group order doesn't decide: columns on the right-hand axis are the secondary series even
        // when their group comes first, and the line on the left is primary.
        let swapped = String::from_utf8_lossy(xml)
            .replace(
                "<c:axPos val=\"l\"/><c:numFmt formatCode=\"#,##0\"",
                "<c:axPos val=\"r\"/><c:crosses val=\"max\"/><c:numFmt formatCode=\"#,##0\"",
            )
            .replace("<c:axPos val=\"r\"/><c:numFmt formatCode=\"0.0\"", "<c:axPos val=\"l\"/><c:numFmt formatCode=\"0.0\"");
        let c2 = read_chart_xml(swapped.as_bytes()).unwrap();
        assert!(c2.series[0].secondary_axis && !c2.series[1].secondary_axis);
        assert_eq!((c2.y_format.as_deref(), c2.y2_format.as_deref()), (Some("0.0"), Some("#,##0")));
        // A chart of one type on one axis has no secondary series.
        let single = String::from_utf8_lossy(xml)
            .replace("<c:axId val=\"3\"/><c:axId val=\"4\"/></c:lineChart>", "<c:axId val=\"1\"/><c:axId val=\"2\"/></c:lineChart>");
        let c = read_chart_xml(single.as_bytes()).unwrap();
        assert!(c.series.iter().all(|s| !s.secondary_axis));
    }

    #[test]
    fn linked_axis_formats_come_from_the_data() {
        // No c:numFmt on the value axis, or a source-linked one: the series' own format.
        let xml = |ax_fmt: &str| {
            format!(
                r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:val><c:numRef><c:numCache><c:formatCode>#,##0</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>1200</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/>{ax_fmt}<c:axPos val="l"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#
            )
        };
        let fmt = |x: &str| read_chart_xml(xml(x).as_bytes()).unwrap().y_format;
        assert_eq!(fmt("").as_deref(), Some("#,##0"));
        assert_eq!(fmt(r#"<c:numFmt formatCode="General" sourceLinked="1"/>"#).as_deref(), Some("#,##0"));
        assert_eq!(fmt(r#"<c:numFmt formatCode="0.0" sourceLinked="0"/>"#).as_deref(), Some("0.0"));
    }

    #[test]
    fn text_colour_is_read_from_the_axes() {
        // LibreOffice's templates set light axis text for dark slides on the axes' c:txPr.
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:val><c:numRef><c:numCache><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:txPr><a:bodyPr/><a:p><a:pPr><a:defRPr sz="1000"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></a:defRPr></a:pPr></a:p></c:txPr></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#;
        let c = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(c.text_color, Some(crate::style::ColorRef::rgb(crate::color::Rgba::rgb(255, 255, 255))));
        assert!(!c.cat_reversed);
        let rev = read_chart_xml(
            xml.replace("<c:catAx><c:axId val=\"1\"/>", "<c:catAx><c:axId val=\"1\"/><c:scaling><c:orientation val=\"maxMin\"/></c:scaling>")
                .as_bytes(),
        )
        .unwrap();
        assert!(rev.cat_reversed, "maxMin reverses the category axis");
        // No c:gapWidth / c:overlap: the schema's 150 % and 0 %.
        assert_eq!((c.gap_width, c.overlap), (1.5, 0.0));
    }

    #[test]
    fn text_sizes_are_read_from_the_file() {
        // PowerPoint's new charts set the axes' and legend's text to 11.97 pt and the title's to
        // 18.62 pt; a chart that only sets its overall text size uses it everywhere.
        let ns =
            r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;
        let tx = |sz: u32| format!(r#"<c:txPr><a:bodyPr/><a:p><a:pPr><a:defRPr sz="{sz}"/></a:pPr></a:p></c:txPr>"#);
        let xml = format!(
            r#"<c:chartSpace {ns}><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:p><a:pPr><a:defRPr sz="1862"/></a:pPr><a:r><a:t>T</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:barDir val="col"/><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/>{}</c:catAx><c:valAx><c:axId val="2"/>{}</c:valAx></c:plotArea></c:chart></c:chartSpace>"#,
            tx(1197),
            tx(1197)
        );
        let c = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!((c.text_size, c.title_size), (Some(11.97), Some(18.62)));
        let xml = format!(
            r#"<c:chartSpace {ns}><c:chart><c:plotArea><c:barChart><c:barDir val="col"/></c:barChart></c:plotArea></c:chart>{}</c:chartSpace>"#,
            tx(1400)
        );
        let c = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!((c.text_size, c.title_size), (Some(14.0), None));
        let xml = format!(
            r#"<c:chartSpace {ns}><c:chart><c:plotArea><c:barChart><c:barDir val="col"/></c:barChart></c:plotArea></c:chart></c:chartSpace>"#
        );
        assert_eq!(read_chart_xml(xml.as_bytes()).unwrap().text_size, None, "no sizes: the renderer picks one");
    }

    #[test]
    fn hidden_labels_and_unfilled_gridlines_are_read() {
        let xml = |ax: &str| {
            format!(
                r#"<c:chartSpace xmlns:c="{NS_C}" xmlns:a="{NS_A}"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/>{ax}</c:valAx></c:plotArea></c:chart></c:chartSpace>"#
            )
        };
        let no_fill = r#"<c:spPr><a:ln w="9525"><a:noFill/></a:ln></c:spPr>"#;
        let c = read_chart_xml(xml(r#"<c:majorGridlines/><c:minorGridlines/><c:tickLblPos val="nextTo"/>"#).as_bytes()).unwrap();
        assert!(c.gridlines && c.minor_gridlines && !c.y_hidden);
        let c = read_chart_xml(
            xml(&format!(
                r#"<c:majorGridlines>{no_fill}</c:majorGridlines><c:minorGridlines>{no_fill}</c:minorGridlines><c:tickLblPos val="none"/>"#
            ))
            .as_bytes(),
        )
        .unwrap();
        assert!(!c.gridlines && !c.minor_gridlines && c.y_hidden);
    }

    #[test]
    fn trendlines_are_read() {
        let xml = br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:lineChart><c:grouping val="standard"/><c:ser><c:idx val="0"/><c:tx><c:v>Spend</c:v></c:tx>
<c:trendline><c:name>Fit &amp; forecast</c:name><c:trendlineType val="linear"/><c:forward val="1"/><c:backward val="0.5"/><c:intercept val="0"/><c:dispRSqr val="1"/><c:dispEq val="1"/></c:trendline>
<c:trendline><c:trendlineType val="poly"/><c:order val="9"/></c:trendline>
<c:trendline><c:trendlineType val="movingAvg"/><c:period val="3"/></c:trendline>
<c:trendline><c:trendlineType val="notAType"/></c:trendline>
<c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser></c:lineChart></c:plotArea></c:chart></c:chartSpace>"#;
        let c = read_chart_xml(xml).unwrap();
        let t = &c.series[0].trendlines;
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].name.as_deref(), Some("Fit & forecast"));
        assert_eq!(t[0].kind, TrendlineType::Linear);
        assert_eq!((t[0].forward, t[0].backward, t[0].intercept), (Some(1.0), Some(0.5), Some(0.0)));
        assert!(t[0].disp_eq && t[0].disp_r_sqr);
        assert_eq!((t[1].kind, t[1].order), (TrendlineType::Poly, Some(6)));
        assert_eq!((t[2].kind, t[2].period), (TrendlineType::MovingAvg, Some(3)));
    }

    const NS_C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
    const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

    fn chart(auto_deleted: &str, title: &str, series: usize) -> Chart {
        let val = r#"<c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val>"#;
        let ser: String = (0..series).map(|i| format!(r#"<c:ser><c:idx val="{i}"/><c:tx><c:v>S{i}</c:v></c:tx>{val}</c:ser>"#)).collect();
        let plot = format!(r#"<c:plotArea><c:barChart><c:barDir val="col"/>{ser}</c:barChart></c:plotArea>"#);
        let xml = format!(r#"<c:chartSpace xmlns:c="{NS_C}" xmlns:a="{NS_A}"><c:chart>{title}{auto_deleted}{plot}</c:chart></c:chartSpace>"#);
        read_chart_xml(xml.as_bytes()).unwrap()
    }

    #[test]
    fn single_series_gets_automatic_title() {
        assert_eq!(chart(r#"<c:autoTitleDeleted val="0"/>"#, "", 1).title.as_deref(), Some("S0"));
    }

    #[test]
    fn no_automatic_title_when_deleted_or_multi_series() {
        assert_eq!(chart(r#"<c:autoTitleDeleted val="1"/>"#, "", 1).title, None);
        assert_eq!(chart("", "", 1).title, None);
        assert_eq!(chart(r#"<c:autoTitleDeleted val="0"/>"#, "", 2).title, None);
    }

    #[test]
    fn explicit_title_wins() {
        let t = "<c:title><c:tx><c:rich><a:p><a:r><a:t>Mine</a:t></a:r></a:p></c:rich></c:tx></c:title>";
        assert_eq!(chart(r#"<c:autoTitleDeleted val="0"/>"#, t, 1).title.as_deref(), Some("Mine"));
    }

    const NS_CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

    fn vary_xml(vary: &str) -> String {
        let ser = r#"<c:ser><c:idx val="0"/><c:order val="0"/></c:ser>"#;
        let bar = format!(r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{vary}{ser}</c:barChart>"#);
        format!(r#"<c:chartSpace xmlns:c="{NS_CHART}"><c:chart><c:plotArea>{bar}</c:plotArea></c:chart></c:chartSpace>"#)
    }

    #[test]
    fn missing_vary_colors_means_vary_by_point() {
        assert!(read_chart_xml(vary_xml("").as_bytes()).is_some_and(|c| c.vary_colors));
    }

    #[test]
    fn explicit_vary_colors_is_respected() {
        assert!(read_chart_xml(vary_xml(r#"<c:varyColors val="0"/>"#).as_bytes()).is_some_and(|c| !c.vary_colors));
        assert!(read_chart_xml(vary_xml(r#"<c:varyColors val="1"/>"#).as_bytes()).is_some_and(|c| c.vary_colors));
    }
    /// A line chart over `cat` (a `c:cat` body) with the category axis element `ax` (a `c:dateAx`
    /// or `c:catAx` body) and `extra` after `c:chart`'s siblings (`c:date1904`).
    fn category_chart(cat: &str, ax: &str, extra: &str) -> Chart {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">{extra}<c:chart><c:plotArea><c:lineChart><c:ser><c:idx val="0"/><c:cat>{cat}</c:cat><c:val><c:numRef><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:lineChart>{ax}<c:valAx><c:axId val="2"/><c:axPos val="l"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#
        );
        read_chart_xml(xml.as_bytes()).unwrap()
    }

    fn num_cat(format_code: &str) -> String {
        let code = if format_code.is_empty() { String::new() } else { format!("<c:formatCode>{format_code}</c:formatCode>") };
        format!(
            r#"<c:numRef><c:numCache>{code}<c:ptCount val="2"/><c:pt idx="0"><c:v>45292</c:v></c:pt><c:pt idx="1"><c:v>45323</c:v></c:pt></c:numCache></c:numRef>"#
        )
    }

    #[test]
    fn date_categories_use_the_cell_format_and_the_axis_format() {
        // category cache format, axis element, Excel shows (first category, second category)
        let own = r#"<c:dateAx><c:axId val="1"/><c:numFmt formatCode="mmm-yy" sourceLinked="0"/></c:dateAx>"#;
        let linked = r#"<c:dateAx><c:axId val="1"/><c:numFmt formatCode="General" sourceLinked="1"/></c:dateAx>"#;
        let bare = r#"<c:dateAx><c:axId val="1"/></c:dateAx>"#;
        let general = r#"<c:dateAx><c:axId val="1"/><c:numFmt formatCode="General" sourceLinked="0"/></c:dateAx>"#;
        for (cell, ax, excel) in [
            ("m/d/yyyy", own, ("Jan-24", "Feb-24")),
            ("", own, ("Jan-24", "Feb-24")),
            ("m/d/yyyy", linked, ("1/1/2024", "2/1/2024")),
            ("m/d/yyyy", bare, ("1/1/2024", "2/1/2024")),
            ("mmmm d", linked, ("January 1", "February 1")),
            // an axis that says General shows the serial numbers, whatever the cells say
            ("m/d/yyyy", general, ("45292", "45323")),
            // nothing says a date: the cached text, as before
            ("", bare, ("45292", "45323")),
            ("General", linked, ("45292", "45323")),
            ("", linked, ("45292", "45323")),
        ] {
            let c = category_chart(&num_cat(cell), ax, "");
            assert_eq!((c.category_label(0).as_str(), c.category_label(1).as_str()), excel, "cell {cell:?}, axis {ax}");
            // the cached categories are kept as the file has them
            assert_eq!(c.categories, ["45292", "45323"]);
        }
        // a category index past the data has no label
        assert_eq!(category_chart(&num_cat("m/d/yyyy"), bare, "").category_label(5), "");
    }

    #[test]
    fn the_category_format_comes_from_the_series_that_gave_the_categories() {
        // The first series has multi-level text categories, the second numeric dates.
        let xml = format!(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:lineChart><c:ser><c:idx val="0"/><c:cat><c:multiLvlStrRef><c:multiLvlStrCache><c:ptCount val="1"/><c:lvl><c:pt idx="0"><c:v>2024</c:v></c:pt></c:lvl></c:multiLvlStrCache></c:multiLvlStrRef></c:cat><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser><c:ser><c:idx val="1"/><c:cat>{}</c:cat><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:lineChart><c:dateAx><c:axId val="1"/></c:dateAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#,
            num_cat("m/d/yyyy")
        );
        let c = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(c.categories, ["2024"]);
        assert_eq!(c.category_format, None);
        assert_eq!(c.category_label(0), "2024");
    }

    #[test]
    fn category_formats_follow_the_retained_series_and_its_plot_axis() {
        let skipped = format!(r#"<c:ser><c:idx val="1000"/><c:cat>{}</c:cat></c:ser>"#, num_cat("m/d/yyyy"));
        let over_budget = format!("{}{}", r#"<c:ser><c:idx val="0"/></c:ser>"#.repeat(1000), skipped);
        let text = r#"<c:strLit><c:ptCount val="1"/><c:pt idx="0"><c:v>2024</c:v></c:pt></c:strLit>"#;
        for (first, second, expected) in [(String::new(), num_cat("m/d/yyyy"), "Jan-24"), (over_budget, text.to_string(), "2024")] {
            let xml = format!(
                r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart>{first}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:lineChart><c:ser><c:idx val="0"/><c:cat>{second}</c:cat></c:ser><c:axId val="3"/><c:axId val="4"/></c:lineChart><c:catAx><c:axId val="1"/><c:numFmt formatCode="0.00" sourceLinked="0"/></c:catAx><c:valAx><c:axId val="2"/><c:axPos val="l"/></c:valAx><c:dateAx><c:axId val="3"/><c:numFmt formatCode="mmm-yy" sourceLinked="0"/></c:dateAx><c:valAx><c:axId val="4"/><c:axPos val="r"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#
            );
            let c = read_chart_xml(xml.as_bytes()).unwrap();
            assert_eq!(c.category_label(0), expected);
        }
    }

    #[test]
    fn scatter_category_format_uses_the_horizontal_axis_regardless_of_id_order() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:scatterChart><c:ser><c:idx val="0"/><c:xVal>{}</c:xVal></c:ser><c:axId val="4"/><c:axId val="3"/></c:scatterChart><c:valAx><c:axId val="4"/><c:axPos val="l"/><c:numFmt formatCode="0.00" sourceLinked="0"/></c:valAx><c:valAx><c:axId val="3"/><c:axPos val="b"/><c:numFmt formatCode="mmm-yy" sourceLinked="0"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#,
            num_cat("m/d/yyyy")
        );
        let c = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(c.category_label(0), "Jan-24");
    }

    #[test]
    fn text_categories_ignore_the_axis_number_format() {
        let strs = r#"<c:strRef><c:strCache><c:ptCount val="2"/><c:pt idx="0"><c:v>2024</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strCache></c:strRef>"#;
        let ax = r#"<c:catAx><c:axId val="1"/><c:numFmt formatCode="0.00" sourceLinked="0"/></c:catAx>"#;
        let c = category_chart(strs, ax, "");
        assert_eq!(c.category_format, None);
        assert_eq!((c.category_label(0).as_str(), c.category_label(1).as_str()), ("2024", "Q2"));
    }

    #[test]
    fn the_1904_date_system_is_read_from_the_chart() {
        let cat = r#"<c:numRef><c:numCache><c:formatCode>m/d/yyyy</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>43830</c:v></c:pt></c:numCache></c:numRef>"#;
        let ax = r#"<c:dateAx><c:axId val="1"/></c:dateAx>"#;
        // Excel shows 1/1/2024 for 43830 in a 1904 workbook, 12/31/2019 in a 1900 one.
        for (extra, date_1904, excel) in [
            ("", false, "12/31/2019"),
            (r#"<c:date1904 val="0"/>"#, false, "12/31/2019"),
            (r#"<c:date1904 val="1"/>"#, true, "1/1/2024"),
            ("<c:date1904/>", true, "1/1/2024"),
        ] {
            let c = category_chart(cat, ax, extra);
            assert_eq!((c.date_1904, c.category_label(0).as_str()), (date_1904, excel), "{extra}");
        }
    }

    #[test]
    fn data_table_first_slice_angle_and_pivot_source_roundtrip() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{NS_C}"><c:pivotSource><c:name>[Book.xlsx]Sheet1!PivotTable1</c:name><c:fmtId val="0"/></c:pivotSource><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>5</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart><c:dTable><c:showHorzBorder/><c:showVertBorder val="0"/><c:showOutline val="1"/><c:showKeys/></c:dTable></c:plotArea></c:chart></c:chartSpace>"#
        );
        let c = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(c.pivot_source.as_deref(), Some("[Book.xlsx]Sheet1!PivotTable1"));
        assert_eq!(c.data_table, Some(ChartDataTable { show_keys: true, horizontal_border: true, vertical_border: false, outline: true }));
        let mut edited = c.clone();
        edited.raw = None;
        let (written, kept) = crate::chart::write::chart_xml(&edited);
        assert!(!kept);
        let back = read_chart_xml(&written).unwrap();
        assert_eq!(back.pivot_source, c.pivot_source);
        assert_eq!(back.data_table, c.data_table);

        let pie_xml = format!(
            r#"<c:chartSpace xmlns:c="{NS_C}"><c:chart><c:plotArea><c:pieChart><c:ser><c:idx val="0"/><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>5</c:v></c:pt></c:numLit></c:val></c:ser><c:firstSliceAng val="450"/></c:pieChart></c:plotArea></c:chart></c:chartSpace>"#
        );
        let mut pie = read_chart_xml(pie_xml.as_bytes()).unwrap();
        assert_eq!(pie.first_slice_angle, 90);
        pie.raw = None;
        let (pie_written, _) = crate::chart::write::chart_xml(&pie);
        assert_eq!(read_chart_xml(&pie_written).unwrap().first_slice_angle, 90);
    }

    // Original synthetic XML for ECMA-376-1 §21.2.2 chart metadata regressions.
    fn metadata_chart(plot_area: &str) -> String {
        format!(
            r#"<c:chartSpace xmlns:c="{NS_C}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea>{plot_area}</c:plotArea></c:chart></c:chartSpace>"#
        )
    }

    fn same_kind_chart_xml() -> String {
        metadata_chart(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>
<c:ser><c:idx val="1"/><c:order val="1"/><c:tx><c:v>Secondary</c:v></c:tx><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>7</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="3"/><c:axId val="4"/></c:barChart>
<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>
<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>Primary</c:v></c:tx><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>2</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:barChart>
<c:catAx><c:axId val="3"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="b"/><c:crossAx val="4"/></c:catAx>
<c:valAx><c:axId val="4"/><c:scaling><c:max val="90"/></c:scaling><c:axPos val="r"/><c:crossAx val="3"/></c:valAx>
<c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="b"/><c:crossAx val="2"/></c:catAx>
<c:valAx><c:axId val="2"/><c:scaling><c:max val="10"/></c:scaling><c:axPos val="l"/><c:crossAx val="1"/></c:valAx>"#,
        )
    }

    #[test]
    fn same_kind_groups_keep_their_series_types_and_axis_ownership() {
        let xml = same_kind_chart_xml();
        let c = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(c.kind, ChartType::Column);
        let primary = c.series.iter().find(|s| s.name == "Primary").unwrap();
        let secondary = c.series.iter().find(|s| s.name == "Secondary").unwrap();
        assert_eq!(primary.kind, Some(ChartType::Column));
        assert_eq!(secondary.kind, primary.kind);
        assert!(!primary.secondary_axis && secondary.secondary_axis);
        assert_eq!((c.y_max, c.y2_max), (Some(10.0), Some(90.0)));
    }

    fn line_marker_chart_xml() -> String {
        metadata_chart(
            r#"<c:lineChart><c:grouping val="standard"/>
<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>Hidden</c:v></c:tx><c:marker><c:symbol val="none"/></c:marker></c:ser>
<c:ser><c:idx val="1"/><c:order val="1"/><c:tx><c:v>Inherited</c:v></c:tx></c:ser><c:marker val="1"/></c:lineChart>
<c:lineChart><c:grouping val="standard"/>
<c:ser><c:idx val="2"/><c:order val="2"/><c:tx><c:v>Explicit</c:v></c:tx><c:marker><c:symbol val="diamond"/></c:marker></c:ser>
<c:ser><c:idx val="3"/><c:order val="3"/><c:tx><c:v>Unmarked</c:v></c:tx></c:ser><c:marker val="0"/></c:lineChart>"#,
        )
    }

    #[test]
    fn line_marker_overrides_do_not_change_other_series() {
        let xml = line_marker_chart_xml();
        let c = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(c.series.iter().map(|s| s.marker.as_deref()).collect::<Vec<_>>(), [Some("none"), None, Some("diamond"), None]);
        assert_eq!(
            c.series.iter().map(|s| c.series_kind(s)).collect::<Vec<_>>(),
            [ChartType::Line, ChartType::LineMarkers, ChartType::LineMarkers, ChartType::Line]
        );
        // Our writer uses a marker-enabled line group with per-series "none" symbols.
        // Those overrides must still read back as an ordinary line chart.
        let plain = Chart::sample(ChartType::Line);
        let back = read_chart_xml(&crate::chart::write::chart_xml(&plain).0).unwrap();
        assert_eq!(back.kind, plain.kind);
    }

    #[test]
    fn series_order_keeps_retained_metadata_attached() {
        let scatter = |idx: u32, order: u32, name: &str, value: u32| {
            format!(
                r#"<c:ser><c:idx val="{idx}"/><c:order val="{order}"/>
<c:tx><c:strRef><c:f>{name}!$A$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{name}</c:v></c:pt></c:strCache></c:strRef></c:tx>
<c:spPr><a:noFill/><a:ln w="12700"><a:solidFill><a:srgbClr val="123456"/></a:solidFill></a:ln></c:spPr>
<c:marker><c:symbol val="diamond"/></c:marker><c:dLbls><c:numFmt formatCode="0.0" sourceLinked="0"/><c:showVal val="1"/></c:dLbls>
<c:trendline><c:name>{name} trend</c:name><c:trendlineType val="linear"/></c:trendline>
<c:errBars><c:errDir val="y"/><c:errBarType val="both"/><c:errValType val="cust"/><c:plus><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>3</c:v></c:pt></c:numLit></c:plus><c:minus><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>4</c:v></c:pt></c:numLit></c:minus></c:errBars>
<c:xVal><c:numRef><c:f>{name}!$B$1</c:f><c:numCache><c:formatCode>0.0</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>{value}</c:v></c:pt></c:numCache></c:numRef></c:xVal>
<c:yVal><c:numRef><c:f>{name}!$C$1</c:f><c:numCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{value}</c:v></c:pt></c:numCache></c:numRef></c:yVal><c:smooth val="1"/></c:ser>"#
            )
        };
        let bubble = |idx: u32, order: &str, name: &str, value: u32| {
            format!(
                r#"<c:ser><c:idx val="{idx}"/>{order}
<c:tx><c:v>{name}</c:v></c:tx><c:spPr><a:solidFill><a:srgbClr val="ABCDEF"/></a:solidFill></c:spPr>
<c:xVal><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>{value}</c:v></c:pt></c:numLit></c:xVal>
<c:yVal><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>{value}</c:v></c:pt></c:numLit></c:yVal>
<c:bubbleSize><c:numRef><c:f>{name}!$D$1</c:f><c:numCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{value}</c:v></c:pt></c:numCache></c:numRef></c:bubbleSize></c:ser>"#
            )
        };
        let a = scatter(0, 2, "Alpha", 11);
        let b = scatter(1, 0, "Beta", 22);
        let c = bubble(2, r#"<c:order val="0"/>"#, "Gamma", 33);
        let d = bubble(3, "", "Delta", 44);
        let xml = metadata_chart(&format!(
            r#"<c:scatterChart><c:scatterStyle val="lineMarker"/>{a}{b}</c:scatterChart><c:bubbleChart>{c}{d}</c:bubbleChart>"#
        ));
        let source_order = xml.replace(r#"<c:order val="2"/>"#, "").replace(r#"<c:order val="0"/>"#, "");
        let ordered = crate::xmlx::parse(xml.as_bytes()).unwrap();
        let source_order = crate::xmlx::parse(source_order.as_bytes()).unwrap();
        // The small budget is exhausted in source order: sorting must move the retained
        // records, not give a previously truncated series a new chance to decode its cache.
        for budget in [MAX_CHART_POINTS, 7] {
            let before = chart_within(&source_order, budget).unwrap();
            let after = chart_within(&ordered, budget).unwrap();
            assert_eq!(
                after.series,
                [before.series[1].clone(), before.series[2].clone(), before.series[0].clone(), before.series[3].clone()],
                "the cross-group tie stays stable, with every series field at budget {budget}"
            );
            assert_eq!(after.categories, before.categories);
            assert_eq!(after.category_format, before.category_format);
        }
    }

    #[test]
    fn primary_axis_titles_follow_the_pair_instead_of_axis_or_empty_group_order() {
        let axis = |tag: &str, id: u32, cross: u32, pos: &str, title: &str, size: Option<u32>, max: Option<u32>| {
            let text = size
                .map(|v| format!(r#"<c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz="{v}"/></a:pPr></a:p></c:txPr>"#))
                .unwrap_or_default();
            let max = max.map(|v| format!(r#"<c:max val="{v}"/>"#)).unwrap_or_default();
            format!(
                r#"<c:{tag}><c:axId val="{id}"/><c:scaling><c:orientation val="minMax"/>{max}</c:scaling><c:axPos val="{pos}"/>
<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{title}</a:t></a:r></a:p></c:rich></c:tx></c:title>{text}<c:crossAx val="{cross}"/></c:{tag}>"#
            )
        };
        for (scatter, empty_group) in [(false, false), (false, true), (true, false)] {
            let (group, settings, cat, value) = if scatter {
                ("scatterChart", r#"<c:scatterStyle val="lineMarker"/>"#, "valAx", "yVal")
            } else {
                ("barChart", r#"<c:barDir val="col"/><c:grouping val="clustered"/>"#, "dateAx", "val")
            };
            let series = |idx: u32, title: &str| {
                format!(
                    r#"<c:ser><c:idx val="{idx}"/><c:order val="{idx}"/><c:tx><c:v>{title}</c:v></c:tx><c:{value}><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>8</c:v></c:pt></c:numLit></c:{value}></c:ser>"#
                )
            };
            let mut plot = String::new();
            if empty_group {
                plot.push_str(r#"<c:barChart><c:barDir val="col"/><c:axId val="50"/><c:axId val="60"/></c:barChart>"#);
            }
            for (idx, title, cat_id, val_id) in [(1, "Secondary", 30, 40), (0, "Primary", 10, 20)] {
                let ser = series(idx, title);
                plot.push_str(&format!(r#"<c:{group}>{settings}{ser}<c:axId val="{cat_id}"/><c:axId val="{val_id}"/></c:{group}>"#));
            }
            if empty_group {
                plot.push_str(&axis("catAx", 50, 60, "b", "Empty category", None, None));
                plot.push_str(&axis("valAx", 60, 50, "l", "Empty value", None, Some(1)));
            }
            plot.push_str(&axis(cat, 30, 40, if scatter { "t" } else { "b" }, "Secondary category", Some(2200), None));
            plot.push_str(&axis("valAx", 40, 30, "r", "Secondary value", None, Some(90)));
            plot.push_str(&axis(cat, 10, 20, "b", "Primary category", Some(1100), None));
            plot.push_str(&axis("valAx", 20, 10, "l", "Primary value", None, Some(10)));
            let c = read_chart_xml(metadata_chart(&plot).as_bytes()).unwrap();
            assert_eq!(c.axis_titles, (Some("Primary category".into()), Some("Primary value".into())), "scatter={scatter}, empty={empty_group}");
            assert_eq!(c.text_size, Some(11.0), "the primary pair also supplies axis text styles");
            assert_eq!(c.y2_max, Some(90.0), "an unused axis must not supply the secondary scale");
            assert!(c.series.iter().find(|s| s.name == "Primary").is_some_and(|s| !s.secondary_axis));
        }
    }

    #[test]
    fn raw_xml_preservation_distinguishes_legacy_models_from_current_metadata_edits() {
        let xml = same_kind_chart_xml()
            .replace("<c:plotArea>", r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:axId val="50"/><c:axId val="60"/></c:barChart>"#)
            .replacen("<c:catAx>", r#"<c:catAx><c:axId val="50"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="b"/><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Empty category</a:t></a:r></a:p></c:rich></c:tx></c:title><c:crossAx val="60"/></c:catAx><c:valAx><c:axId val="60"/><c:scaling><c:max val="1"/></c:scaling><c:axPos val="l"/><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Empty value</a:t></a:r></a:p></c:rich></c:tx></c:title><c:crossAx val="50"/></c:valAx><c:catAx>"#, 1)
            .replace(
            "</c:chartSpace>",
            r#"<c:extLst><c:ext uri="urn:craft:test:original-chart"><own:preserved xmlns:own="urn:craft:test" value="legacy-kept"/></c:ext></c:extLst></c:chartSpace>"#,
        );
        // An original synthetic native-save model matching the former reader: source series
        // order, no same-kind overrides or secondary scale, and no provenance field.
        let legacy: Chart = serde_json::from_value(serde_json::json!({
            "kind": "column", "title": null, "legend": null, "gridlines": false,
            "gapWidth": 1.5, "overlap": 0.0, "varyColors": true, "yMax": 1.0,
            "axisTitles": ["Empty category", "Empty value"],
            "series": [
                { "name": "Secondary", "values": [7.0], "kind": null, "secondaryAxis": false },
                { "name": "Primary", "values": [2.0], "kind": null, "secondaryAxis": false }
            ],
            "raw": xml
        }))
        .unwrap();
        let (written, kept) = crate::chart::write::chart_xml(&legacy);
        assert!(kept, "an unchanged legacy save retains its unmodelled XML");
        let doc = crate::xmlx::parse(&written).unwrap();
        assert_eq!(doc.find(Local("preserved")).and_then(|e| e.attr_local("value")), Some("legacy-kept"));

        // The former line reader made marker visibility a group decision and collapsed
        // explicit "none" into omission; a legacy save must still match that projection.
        let legacy_line: Chart = serde_json::from_value(serde_json::json!({
            "kind": "line", "title": null, "legend": null, "gridlines": false, "varyColors": true,
            "series": [
                { "name": "Hidden" }, { "name": "Inherited" },
                { "name": "Explicit", "marker": "diamond" }, { "name": "Unmarked" }
            ],
            "raw": line_marker_chart_xml()
        }))
        .unwrap();
        assert!(crate::chart::write::chart_xml(&legacy_line).1, "legacy group marker and symbol semantics remain comparable");

        let mut edited = legacy.clone();
        edited.axis_titles.0 = Some("Edited category".into());
        let (written, kept) = crate::chart::write::chart_xml(&edited);
        assert!(!kept, "legacy provenance does not hide an actual title edit");
        assert_eq!(read_chart_xml(&written).unwrap().axis_titles.0, edited.axis_titles.0);

        let imported = read_chart_xml(legacy.raw.as_ref().unwrap().as_bytes()).unwrap();
        let reopened: Chart = serde_json::from_str(&serde_json::to_string(&imported).unwrap()).unwrap();
        // A current-provenance chart genuinely edited into the old reader's representation
        // has identical modelled values to `legacy`, but must not take its raw fastpath.
        let mut edited_into_legacy = legacy.clone();
        edited_into_legacy.raw_metadata_version = reopened.raw_metadata_version;
        let (written, kept) = crate::chart::write::chart_xml(&edited_into_legacy);
        assert!(!kept, "current metadata edits cannot be mistaken for unchanged legacy state");
        let back = read_chart_xml(&written).unwrap();
        assert!(back.series.iter().all(|s| !s.secondary_axis), "the edited primary-axis assignment is written");
    }

    #[test]
    fn review_6093532941_title_edit_keeps_same_kind_secondary_axis_and_scale() {
        let xml = same_kind_chart_xml().replace(
            r#"<c:valAx><c:axId val="4"/><c:scaling><c:max val="90"/></c:scaling><c:axPos val="r"/><c:crossAx val="3"/></c:valAx>"#,
            r#"<c:valAx><c:axId val="4"/><c:scaling><c:max val="90"/><c:min val="2"/></c:scaling><c:delete val="1"/><c:axPos val="r"/><c:numFmt formatCode="0.00%" sourceLinked="0"/><c:crossAx val="3"/><c:majorUnit val="5"/></c:valAx>"#,
        );
        let mut edited = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(edited.kind, ChartType::Column);
        assert_eq!((edited.y2_min, edited.y2_max, edited.y2_major), (Some(2.0), Some(90.0), Some(5.0)));
        assert_eq!((edited.y2_format.as_deref(), edited.y2_hidden), (Some("0.00%"), true));
        edited.title = Some("Only the chart title changed".into());
        let (written, kept) = crate::chart::write::chart_xml(&edited);
        assert!(!kept, "the title edit must take the XML rebuilding route");
        let back = read_chart_xml(&written).unwrap();
        assert_eq!(back.title, edited.title);
        assert!(!back.series.iter().find(|s| s.name == "Primary").unwrap().secondary_axis);
        assert!(back.series.iter().find(|s| s.name == "Secondary").unwrap().secondary_axis, "a title edit must not move the secondary series");
        assert_eq!((back.y2_min, back.y2_max, back.y2_major), (edited.y2_min, edited.y2_max, edited.y2_major));
        assert_eq!((back.y2_format.as_deref(), back.y2_hidden), (edited.y2_format.as_deref(), edited.y2_hidden));
        assert_eq!(back.y_max, edited.y_max, "the primary scale remains independent");

        let doc = crate::xmlx::parse(&written).unwrap();
        let plot = doc.path(&[Local("chart"), Local("plotArea")]).unwrap();
        let groups = plot.children(Local("barChart")).collect::<Vec<_>>();
        assert_eq!(groups.len(), 2, "same type on different axis pairs needs distinct plot groups");
        let pair = |name: &str| {
            let group = groups.iter().find(|g| g.children(Local("ser")).any(|s| rich_text(s).as_deref() == Some(name))).unwrap();
            group.children(Local("axId")).map(|id| id.attr_local("val").unwrap().to_string()).collect::<Vec<_>>()
        };
        let (primary, secondary) = (pair("Primary"), pair("Secondary"));
        assert_eq!((primary.len(), secondary.len()), (2, 2));
        assert!(primary.iter().all(|id| !secondary.contains(id)), "the secondary pair owns distinct axis IDs");
        for pair in [&primary, &secondary] {
            for (i, id) in pair.iter().enumerate() {
                let axis = plot
                    .elements()
                    .find(|a| matches!(a.local(), "catAx" | "valAx") && a.child(Local("axId")).and_then(|v| v.attr_local("val")) == Some(id.as_str()))
                    .unwrap();
                assert_eq!(axis.child(Local("crossAx")).and_then(|v| v.attr_local("val")), Some(pair[1 - i].as_str()));
            }
        }
    }

    #[test]
    fn review_6093532941_series_kind_edit_rebuilds_without_chart_kind_change() {
        let xml = same_kind_chart_xml().replace(r#"<c:axId val="3"/><c:axId val="4"/>"#, r#"<c:axId val="1"/><c:axId val="2"/>"#);
        let mut edited = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(edited.kind, ChartType::Column);
        assert!(edited.series.iter().all(|s| !s.secondary_axis));
        edited.series.iter_mut().find(|s| s.name == "Primary").unwrap().kind = Some(ChartType::Line);
        let (written, kept) = crate::chart::write::chart_xml(&edited);
        assert!(!kept, "an explicit series-type edit must rebuild the unchanged chart-wide type");
        let back = read_chart_xml(&written).unwrap();
        assert_eq!(back.series.iter().find(|s| s.name == "Primary").unwrap().kind, Some(ChartType::Line));
        assert_eq!(back.series.iter().find(|s| s.name == "Secondary").unwrap().kind, Some(ChartType::Column));

        // The former reader flattens this group to Line when any marker is explicitly none.
        // The current reader retains the omitted marker as LineMarkers, producing Combo.
        let xml = metadata_chart(
            r#"<c:lineChart><c:grouping val="standard"/>
<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>Edited</c:v></c:tx><c:marker><c:symbol val="none"/></c:marker><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>2</c:v></c:pt></c:numLit></c:val></c:ser>
<c:ser><c:idx val="1"/><c:order val="1"/><c:tx><c:v>Inherited</c:v></c:tx><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>7</c:v></c:pt></c:numLit></c:val></c:ser><c:marker val="1"/></c:lineChart>"#,
        );
        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        assert_eq!(chart_from(&root).unwrap().kind, ChartType::Combo);
        let mut legacy = legacy_chart_from(&root).unwrap();
        assert_eq!((legacy.kind, legacy.raw_metadata_version), (ChartType::Line, 0));
        legacy.raw = Some(xml);
        legacy.series[0].kind = Some(ChartType::Area);
        let (written, kept) = crate::chart::write::chart_xml(&legacy);
        assert!(!kept);
        let back = read_chart_xml(&written).unwrap();
        assert_eq!(
            back.series.iter().find(|s| s.name == "Edited").unwrap().kind,
            Some(ChartType::Area),
            "legacy provenance must not turn an unchanged chart-wide type into an override"
        );
        assert_eq!(back.series.iter().find(|s| s.name == "Inherited").unwrap().kind, Some(ChartType::Line));
    }

    #[test]
    fn review_6093532941_series_kind_without_raw_remains_authoritative() {
        for kind in [ChartType::Area, ChartType::Bar, ChartType::PercentColumn] {
            let chart = Chart {
                kind: ChartType::Column,
                categories: vec!["Category".into()],
                series: vec![Series { name: "Explicit type".into(), kind: Some(kind), values: vec![Some(7.0)], ..Default::default() }],
                ..Default::default()
            };
            assert!(chart.raw.is_none());
            let (written, kept) = crate::chart::write::chart_xml(&chart);
            assert!(!kept);
            let back = read_chart_xml(&written).unwrap();
            assert_eq!(back.kind, kind, "without a raw baseline there is no chart-wide edit to override the explicit series type");
            assert_eq!(back.series[0].kind, Some(kind));
            assert_eq!((back.series[0].name.as_str(), &back.series[0].values), (chart.series[0].name.as_str(), &chart.series[0].values));

            let doc = crate::xmlx::parse(&written).unwrap();
            let plot = doc.path(&[Local("chart"), Local("plotArea")]).unwrap();
            let category = plot.child(Local("catAx")).unwrap();
            let value = plot.child(Local("valAx")).unwrap();
            assert_eq!(category.child(Local("axPos")).and_then(|a| a.attr_local("val")), Some(if kind.horizontal() { "l" } else { "b" }));
            assert_eq!(value.child(Local("axPos")).and_then(|a| a.attr_local("val")), Some(if kind.horizontal() { "b" } else { "l" }));
            let format = value.child(Local("numFmt")).unwrap();
            assert_eq!(format.attr_local("formatCode"), Some(if kind.percent() { "0%" } else { "General" }));
            assert_eq!(format.attr_local("sourceLinked"), Some("1"), "the format is the effective type's default, not an explicit override");
        }
    }

    #[test]
    fn review_6093532941_chart_kind_edit_overrides_imported_series_kinds() {
        let xml = same_kind_chart_xml()
            .replacen(r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>"#, r#"<c:areaChart><c:grouping val="standard"/>"#, 1)
            .replacen("</c:barChart>", "</c:areaChart>", 1);
        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        for legacy in [false, true] {
            let mut chart = if legacy { legacy_chart_from(&root).unwrap() } else { chart_from(&root).unwrap() };
            chart.raw = Some(xml.clone());
            assert_eq!(chart.kind, ChartType::Combo);
            assert!(chart.series.iter().any(|s| s.kind == Some(ChartType::Column)));
            assert!(chart.series.iter().any(|s| s.kind == Some(ChartType::Area)));
            let names_and_values = chart.series.iter().map(|s| (s.name.clone(), s.values.clone())).collect::<Vec<_>>();
            chart.kind = ChartType::Line;
            let (written, kept) = crate::chart::write::chart_xml(&chart);
            assert!(!kept);
            let back = read_chart_xml(&written).unwrap();
            assert_eq!(back.kind, ChartType::Line, "an actual chart-wide edit wins for legacy={legacy}");
            assert!(back.series.iter().all(|s| s.kind == Some(ChartType::Line)), "imported overrides must not undo the chart-wide edit");
            assert_eq!(back.series.iter().map(|s| (s.name.clone(), s.values.clone())).collect::<Vec<_>>(), names_and_values);
        }
    }

    #[test]
    fn review_6094076812_deleting_primary_keeps_the_surviving_secondary_axis() {
        for name in ["Primary", "Secondary"] {
            let mut chart = read_chart_xml(same_kind_chart_xml().as_bytes()).unwrap();
            chart.series.retain(|s| s.name == name);
            let (written, kept) = crate::chart::write::chart_xml(&chart);
            assert!(!kept);
            let back = read_chart_xml(&written).unwrap();
            assert_eq!(back.series.len(), 1);
            assert_eq!(back.series[0].name, name);
            assert_eq!(back.series[0].secondary_axis, name == "Secondary");
            assert_eq!(back.y_max, Some(10.0), "the primary scale must not adopt the surviving secondary scale");
            if name == "Secondary" {
                assert_eq!(back.y2_max, Some(90.0));
                assert_eq!(back.series[0].values, vec![Some(7.0)]);
            } else {
                assert_eq!(back.series[0].values, vec![Some(2.0)]);
            }
        }
        // Published version1 called the only used value axis primary. Keep that exact
        // native-save projection, even though new imports retain the unused primary pair.
        let xml = metadata_chart(
            r#"<c:barChart><c:barDir val="col"/>
<c:ser><c:tx><c:v>Secondary</c:v></c:tx></c:ser><c:axId val="3"/><c:axId val="4"/></c:barChart>
<c:catAx><c:axId val="1"/><c:axPos val="b"/><c:crossAx val="2"/></c:catAx>
<c:valAx><c:axId val="2"/><c:scaling><c:max val="10"/></c:scaling><c:axPos val="l"/><c:crossAx val="1"/></c:valAx>
<c:catAx><c:axId val="3"/><c:axPos val="b"/><c:crossAx val="4"/></c:catAx>
<c:valAx><c:axId val="4"/><c:scaling><c:max val="90"/></c:scaling><c:axPos val="r"/><c:crossAx val="3"/></c:valAx>"#,
        );
        let mut old = serde_json::to_value(read_chart_xml(xml.as_bytes()).unwrap()).unwrap();
        old["rawMetadataVersion"] = serde_json::json!(1);
        for series in old["series"].as_array_mut().unwrap() {
            series.as_object_mut().unwrap().remove("sourceIdx");
            series.as_object_mut().unwrap().remove("sourceGroup");
        }
        old["series"][0]["secondaryAxis"] = serde_json::json!(false);
        old["yMax"] = serde_json::json!(90.0);
        old["y2Max"] = serde_json::Value::Null;
        let saved: Chart = serde_json::from_value(old).unwrap();
        assert!(crate::chart::write::chart_xml(&saved).1, "unchanged version1 axis metadata keeps raw XML");
        let mut current = saved;
        current.raw_metadata_version = 2;
        assert!(!crate::chart::write::chart_xml(&current).1, "current ownership edits are not legacy state");
    }

    fn review_line_chart(grouping: &str, visible: bool, series: &[(&str, Option<&str>)]) -> String {
        let mut plot = format!(r#"<c:lineChart><c:grouping val="{grouping}"/>"#);
        for (i, (name, marker)) in series.iter().enumerate() {
            let marker = marker.map(|m| format!(r#"<c:marker><c:symbol val="{m}"/></c:marker>"#)).unwrap_or_default();
            plot.push_str(&format!(
                r#"<c:ser><c:idx val="{i}"/><c:order val="{i}"/><c:tx><c:v>{name}</c:v></c:tx>{marker}<c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>7</c:v></c:pt></c:numLit></c:val></c:ser>"#
            ));
        }
        plot.push_str(&format!(r#"<c:marker val="{}"/></c:lineChart>"#, u8::from(visible)));
        metadata_chart(&plot)
    }

    #[test]
    fn review_6094076812_chart_wide_line_edits_override_only_the_old_visibility() {
        for (visible, symbol, target) in [(false, "none", ChartType::LineMarkers), (true, "circle", ChartType::Line)] {
            let xml = review_line_chart("standard", visible, &[("Edited", Some(symbol))]);
            let mut chart = read_chart_xml(xml.as_bytes()).unwrap();
            assert_ne!(chart.kind, target);
            chart.kind = target;
            let (written, kept) = crate::chart::write::chart_xml(&chart);
            assert!(!kept);
            let back = read_chart_xml(&written).unwrap();
            assert_eq!(back.kind, target, "a real chart-wide edit wins over the imported symbol {symbol}");
            assert_eq!(back.series[0].values, vec![Some(7.0)]);
        }
        // An explicit per-series none is deliberate when there was no chart-wide edit.
        let mut unchanged_kind = read_chart_xml(line_marker_chart_xml().as_bytes()).unwrap();
        unchanged_kind.title = Some("Title only".into());
        let back = read_chart_xml(&crate::chart::write::chart_xml(&unchanged_kind).0).unwrap();
        assert_eq!(back.series.iter().map(|s| s.marker.as_deref()).collect::<Vec<_>>(), [Some("none"), None, Some("diamond"), None]);
        assert_eq!(back.series.iter().map(|s| s.kind).collect::<Vec<_>>(), unchanged_kind.series.iter().map(|s| s.kind).collect::<Vec<_>>());

        let chart = Chart {
            kind: ChartType::LineMarkers,
            series: vec![
                Series { name: "Hidden".into(), marker: Some("none".into()), kind: Some(ChartType::LineMarkers), ..Default::default() },
                Series { name: "Circle".into(), marker: Some("circle".into()), kind: Some(ChartType::LineMarkers), ..Default::default() },
            ],
            ..Default::default()
        };
        let back = read_chart_xml(&crate::chart::write::chart_xml(&chart).0).unwrap();
        assert_eq!(back.series[0].marker.as_deref(), Some("none"), "do not globally discard explicit none");
        assert_eq!(back.series[1].marker.as_deref(), Some("circle"));
    }

    #[test]
    fn review_6094076812_combo_axes_follow_each_emitted_plot_and_explicit_formats() {
        for explicit in [false, true] {
            for line_first in [false, true] {
                let mut chart = Chart {
                    kind: ChartType::Combo,
                    categories: vec!["Category".into()],
                    series: vec![
                        Series { name: "Primary bar".into(), kind: Some(ChartType::Bar), values: vec![Some(2.0)], ..Default::default() },
                        Series {
                            name: "Secondary percent bar".into(),
                            kind: Some(ChartType::PercentBar),
                            secondary_axis: true,
                            values: vec![Some(7.0)],
                            ..Default::default()
                        },
                    ],
                    y_format: explicit.then(|| "0.00".into()),
                    y2_format: explicit.then(|| "0.000%".into()),
                    ..Default::default()
                };
                if line_first {
                    chart.series.insert(
                        0,
                        Series { name: "Primary line".into(), kind: Some(ChartType::Line), values: vec![Some(3.0)], ..Default::default() },
                    );
                    chart.series.insert(
                        1,
                        Series {
                            name: "Secondary line".into(),
                            kind: Some(ChartType::Line),
                            secondary_axis: true,
                            values: vec![Some(4.0)],
                            ..Default::default()
                        },
                    );
                }
                let (written, _) = crate::chart::write::chart_xml(&chart);
                let doc = crate::xmlx::parse(&written).unwrap();
                let plot = doc.path(&[Local("chart"), Local("plotArea")]).unwrap();
                for (name, cat_pos, val_pos, code) in [
                    ("Primary bar", "l", "b", if explicit { "0.00" } else { "General" }),
                    ("Secondary percent bar", "r", "t", if explicit { "0.000%" } else { "0%" }),
                ] {
                    let group =
                        plot.children(Local("barChart")).find(|g| g.children(Local("ser")).any(|s| rich_text(s).as_deref() == Some(name))).unwrap();
                    let ids = group.children(Local("axId")).map(|a| a.attr_local("val").unwrap()).collect::<Vec<_>>();
                    assert_eq!(ids.len(), 2);
                    for (tag, id, pos) in [("catAx", ids[0], cat_pos), ("valAx", ids[1], val_pos)] {
                        let axis = plot.children(Local(tag)).find(|a| a.child(Local("axId")).and_then(|v| v.attr_local("val")) == Some(id)).unwrap();
                        assert_eq!(axis.child(Local("axPos")).and_then(|a| a.attr_local("val")), Some(pos));
                        if tag == "valAx" {
                            let format = axis.child(Local("numFmt")).unwrap();
                            assert_eq!(format.attr_local("formatCode"), Some(code));
                            assert_eq!(format.attr_local("sourceLinked"), Some(if explicit { "0" } else { "1" }));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn review_6094076812_title_only_plain_line_keeps_omitted_and_explicit_none_distinct() {
        let xml = review_line_chart("standard", false, &[("Omitted", None), ("Explicit none", Some("none"))]);
        let mut chart = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(chart.kind, ChartType::Line);
        chart.title = Some("New title".into());
        let (written, kept) = crate::chart::write::chart_xml(&chart);
        assert!(!kept);
        let back = read_chart_xml(&written).unwrap();
        assert_eq!(back.kind, ChartType::Line);
        assert_eq!(back.title, chart.title);
        assert_eq!(back.series.iter().map(|s| s.marker.as_deref()).collect::<Vec<_>>(), [None, Some("none")]);
        let doc = crate::xmlx::parse(&written).unwrap();
        let plot = doc.path(&[Local("chart"), Local("plotArea")]).unwrap();
        assert!(plot.children(Local("lineChart")).all(|g| g.child(Local("marker")).and_then(|m| m.attr_local("val")) == Some("0")));
    }

    #[test]
    fn review_6094076812_stacked_marker_visibility_survives_independently_of_symbol_and_kind() {
        for grouping in ["stacked", "percentStacked"] {
            for visible in [false, true] {
                let xml = review_line_chart(grouping, visible, &[("Inherited", None), ("Hidden", Some("none")), ("Circle", Some("circle"))]);
                let chart = read_chart_xml(xml.as_bytes()).unwrap();
                assert!(chart.series.iter().all(|s| s.kind == Some(ChartType::StackedLine)));
                let model = serde_json::to_value(&chart).unwrap();
                for (i, expected) in [visible, false, true].into_iter().enumerate() {
                    assert_eq!(
                        model["series"][i].get("markerVisible"),
                        Some(&serde_json::json!(expected)),
                        "{grouping}, visible={visible}, series={i}"
                    );
                }
                // Simulate the published version1 native-save shape, which had no markerVisible.
                let mut old = model.clone();
                old["rawMetadataVersion"] = serde_json::json!(1);
                for series in old["series"].as_array_mut().unwrap() {
                    series.as_object_mut().unwrap().remove("markerVisible");
                    series.as_object_mut().unwrap().remove("sourceIdx");
                    series.as_object_mut().unwrap().remove("sourceGroup");
                }
                let old: Chart = serde_json::from_value(old).unwrap();
                assert!(crate::chart::write::chart_xml(&old).1, "unchanged version1 saves retain their raw part");

                let mut edited = chart.clone();
                edited.title = Some("Title only".into());
                let (written, kept) = crate::chart::write::chart_xml(&edited);
                assert!(!kept);
                let back = read_chart_xml(&written).unwrap();
                assert_eq!(back.series.iter().map(|s| s.marker.as_deref()).collect::<Vec<_>>(), [None, Some("none"), Some("circle")]);
                let back = serde_json::to_value(&back).unwrap();
                for (i, expected) in [visible, false, true].into_iter().enumerate() {
                    assert_eq!(back["series"][i]["markerVisible"], serde_json::json!(expected));
                }
                // Changing current metadata must rebuild, even though its raw XML is unchanged.
                let mut changed = model;
                changed["series"][0]["markerVisible"] = serde_json::json!(!visible);
                let changed: Chart = serde_json::from_value(changed).unwrap();
                let (written, kept) = crate::chart::write::chart_xml(&changed);
                assert!(!kept);
                let back = serde_json::to_value(read_chart_xml(&written).unwrap()).unwrap();
                assert_eq!(back["series"][0]["markerVisible"], serde_json::json!(!visible));
            }
        }
    }

    #[test]
    fn current_line_marker_defaults_distinguish_bare_false_and_absent() {
        // Show Marker uses CT_Boolean: a present element without val defaults to true.
        for grouping in ["standard", "stacked", "percentStacked"] {
            for (flag, visible) in [("<c:marker/>", true), ("<c:marker val=\"true\"/>", true), ("<c:marker val=\"0\"/>", false), ("", false)] {
                let xml = review_line_chart(grouping, true, &[("Inherited", None)]).replace("<c:marker val=\"1\"/>", flag);
                let mut chart = read_chart_xml(xml.as_bytes()).unwrap();
                let expected = if grouping == "standard" {
                    assert_eq!(chart.kind, if visible { ChartType::LineMarkers } else { ChartType::Line });
                    None
                } else {
                    Some(serde_json::json!(visible))
                };
                assert_eq!(serde_json::to_value(&chart).unwrap()["series"][0].get("markerVisible"), expected.as_ref());
                assert_eq!(chart.series[0].marker, None);
                chart.title = Some("Title only".into());
                let (written, kept) = crate::chart::write::chart_xml(&chart);
                assert!(!kept);
                let back = read_chart_xml(&written).unwrap();
                assert_eq!(back.kind, chart.kind);
                assert_eq!(back.series[0].marker, None);
                assert_eq!(serde_json::to_value(back).unwrap()["series"][0].get("markerVisible"), expected.as_ref());
            }
        }
    }

    #[test]
    fn review_6094537368_series_kind_edits_do_not_inherit_stacked_visibility() {
        for (visible, symbol, target, expected) in [
            (false, None, ChartType::LineMarkers, ChartType::LineMarkers),
            (true, None, ChartType::Line, ChartType::Line),
            (false, Some("none"), ChartType::LineMarkers, ChartType::Line),
            (true, Some("circle"), ChartType::Line, ChartType::LineMarkers),
        ] {
            let xml = review_line_chart("stacked", visible, &[("Edited", symbol)]);
            let mut chart = read_chart_xml(xml.as_bytes()).unwrap();
            assert_eq!(chart.kind, ChartType::StackedLine);
            chart.series[0].kind = Some(target);
            let (written, kept) = crate::chart::write::chart_xml(&chart);
            assert!(!kept);
            let back = read_chart_xml(&written).unwrap();
            assert_eq!(back.series[0].kind, Some(expected), "visible={visible}, symbol={symbol:?}, target={target:?}");
            assert_eq!(back.series[0].values, vec![Some(7.0)]);
            assert_eq!(back.series[0].marker.as_deref(), symbol, "explicit symbols still take precedence");
        }
    }

    #[test]
    fn review_6094537368_same_kind_secondary_axes_drive_shared_helpers() {
        let xml = same_kind_chart_xml()
            .replace(r#"<c:max val="90"/>"#, r#"<c:min val="2"/><c:max val="90"/>"#)
            .replace(r#"<c:max val="10"/>"#, r#"<c:min val="0"/><c:max val="10"/>"#)
            .replace(r#"<c:axPos val="r"/>"#, r#"<c:axPos val="r"/><c:numFmt formatCode="0%" sourceLinked="0"/><c:majorUnit val="5"/>"#)
            .replace(r#"<c:axPos val="l"/>"#, r#"<c:axPos val="l"/><c:numFmt formatCode="0.0" sourceLinked="0"/><c:majorUnit val="2"/>"#);
        let mut chart = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(chart.kind, ChartType::Column);
        assert!(!chart.on_secondary(&chart.series[0]));
        assert!(chart.on_secondary(&chart.series[1]));
        assert_eq!(chart.axis_range(false), Some((0.0, 10.0)));
        assert_eq!(chart.axis_range(true), Some((0.0, 7.0)), "secondary range is data extent, not explicit scale bounds");
        assert_eq!(chart.axis_scale(false, 5), Some((0.0, 10.0, 2.0)));
        assert_eq!(chart.axis_scale(true, 5), Some((2.0, 90.0, 5.0)));
        assert_eq!(chart.data_label(&chart.series[0], 0.5), "0.5");
        assert_eq!(chart.data_label(&chart.series[1], 0.5), "50%");
        chart.series[1].number_format = Some("0.00".into());
        assert_eq!(chart.data_label(&chart.series[1], 0.5), "0.50", "series format wins over the axis");
        chart.y_min = None;
        chart.y_max = None;
        assert_eq!(chart.axis_range(false), Some((0.0, 2.0)), "secondary values do not enlarge the inferred primary extent");
        chart.series.retain(|s| s.secondary_axis);
        assert_eq!(chart.axis_range(false), None, "there is no populated primary axis");
        assert_eq!(chart.axis_scale(false, 5), None);
        assert_eq!(chart.axis_range(true), Some((0.0, 7.0)));

        for kind in [ChartType::Column, ChartType::StackedColumn, ChartType::PercentColumn, ChartType::Waterfall] {
            let single = Chart::sample(kind);
            assert_eq!(single.axis_range(false), Some(single.value_range()), "unchanged single-axis aggregate for {kind:?}");
            assert_eq!(single.axis_range(true), None);
            assert!(single.axis_scale(false, 5).is_some());
            assert_eq!(single.axis_scale(true, 5), None);
        }
        let empty = Chart { series: vec![], ..Default::default() };
        assert_eq!(empty.axis_range(false), Some((0.0, 0.0)), "the existing empty primary default remains");
        assert_eq!(empty.axis_scale(true, 5), None);
    }

    fn review_source_ids_chart(second_id: u32) -> String {
        metadata_chart(&format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>
<c:ser><c:idx val="0"/><c:order val="1"/><c:tx><c:v>A</c:v></c:tx><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>2</c:v></c:pt></c:numLit></c:val></c:ser>
<c:ser><c:idx val="{second_id}"/><c:order val="0"/><c:tx><c:strRef><c:f>Data!$Z$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>B</c:v></c:pt></c:strCache></c:strRef></c:tx><c:val><c:numRef><c:f>Data!$Z$2:$Z$2</c:f><c:numCache><c:ptCount val="1"/><c:pt idx="0"><c:v>7</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
</c:barChart>"#
        ))
    }

    fn review_written_ids(bytes: &[u8]) -> Vec<(String, u32, u32)> {
        let root = crate::xmlx::parse(bytes).unwrap();
        root.path(&[Local("chart"), Local("plotArea")])
            .unwrap()
            .elements()
            .filter(|g| g.local().ends_with("Chart"))
            .flat_map(|g| g.children(Local("ser")))
            .map(|s| {
                let value = |name| s.child(Local(name)).and_then(|v| v.attr_local("val")).unwrap().parse().unwrap();
                (rich_text(s).unwrap(), value("idx"), value("order"))
            })
            .collect()
    }

    #[test]
    fn review_6094537368_source_ids_survive_reorder_new_series_and_duplicate_ids() {
        for second_id in [1, u32::MAX] {
            let xml = review_source_ids_chart(second_id);
            let mut chart = read_chart_xml(xml.as_bytes()).unwrap();
            assert_eq!(chart.series.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["B", "A"]);
            chart.title = Some("Title only".into());
            let (written, kept) = crate::chart::write::chart_xml(&chart);
            assert!(!kept);
            assert_eq!(review_written_ids(&written), [("B".into(), second_id, 0), ("A".into(), 0, 1)]);

            let mut duplicate = chart.series[0].clone();
            duplicate.name = "Duplicate B".into();
            chart.series.push(duplicate);
            chart.series.insert(0, Series { name: "New".into(), values: vec![Some(13.0)], ..Default::default() });
            let written = crate::chart::write::chart_xml(&chart).0;
            let (new_id, duplicate_id) = if second_id == 1 { (2, 3) } else { (1, 2) };
            assert_eq!(
                review_written_ids(&written),
                [("New".into(), new_id, 0), ("B".into(), second_id, 1), ("A".into(), 0, 2), ("Duplicate B".into(), duplicate_id, 3),]
            );
            let back = read_chart_xml(&written).unwrap();
            assert_eq!(
                back.series.iter().map(|s| s.values.clone()).collect::<Vec<_>>(),
                [vec![Some(13.0)], vec![Some(7.0)], vec![Some(2.0)], vec![Some(7.0)]]
            );
            let mut default_layout = chart.clone();
            for s in &mut default_layout.series {
                s.refs = Default::default();
            }
            let default_written = crate::chart::write::chart_xml(&default_layout).0;
            assert_eq!(review_written_ids(&default_written), review_written_ids(&written));
            let default_back = read_chart_xml(&default_written).unwrap();
            assert_eq!(default_back.series[0].refs.values.as_deref(), Some("Sheet1!$B$2:$B$2"));
            assert!(
                back.series[0].refs.name.is_none() && back.series[0].refs.values.is_none(),
                "new literal series do not guess a column in a mixed workbook layout"
            );
            assert_eq!(back.series[1].refs.name.as_deref(), Some("Data!$Z$1"));
            assert_eq!(back.series[1].refs.values.as_deref(), Some("Data!$Z$2:$Z$2"));
            assert_eq!(default_back.series[2].refs.name.as_deref(), Some("Sheet1!$D$1"));
            assert_eq!(
                default_back.series[2].refs.values.as_deref(),
                Some("Sheet1!$D$2:$D$2"),
                "formula columns follow display position, not source ID"
            );
            assert!(
                back.series[2].refs.name.is_none() && back.series[2].refs.values.is_none(),
                "the original literal A remains literal in the mixed layout"
            );
            assert_eq!(back.series[3].refs.values.as_deref(), Some("Data!$Z$2:$Z$2"));
        }
        for invalid in [None, Some("-1"), Some("1.5"), Some("4294967296"), Some("bad")] {
            let idx = invalid.map(|v| format!(r#"<c:idx val="{v}"/>"#)).unwrap_or_default();
            let xml = review_source_ids_chart(1)
                .replacen(r#"<c:idx val="1"/>"#, &idx, 1)
                .replace("</c:barChart>", r#"<c:ser><c:idx val="1"/><c:order val="2"/><c:tx><c:v>C</c:v></c:tx></c:ser></c:barChart>"#);
            let mut chart = read_chart_xml(xml.as_bytes()).unwrap();
            chart.title = Some("Rebuild invalid ID".into());
            let written = crate::chart::write::chart_xml(&chart).0;
            assert_eq!(review_written_ids(&written), [("B".into(), 2, 0), ("A".into(), 0, 1), ("C".into(), 1, 2)], "invalid={invalid:?}");
        }
    }

    #[test]
    fn review_6094537368_source_ids_preserve_old_raw_projections_and_real_id_edits() {
        let xml = review_source_ids_chart(1).replace("</c:chartSpace>", r#"<c:extLst><c:ext uri="urn:craft:original-series-identity"><own:preserved xmlns:own="urn:craft:original"/></c:ext></c:extLst></c:chartSpace>"#);
        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        for version in [0, 1, 2] {
            let mut previous = read_chart(&root, MAX_CHART_POINTS, version).unwrap();
            previous.raw = Some(xml.clone());
            let previous: Chart = serde_json::from_str(&serde_json::to_string(&previous).unwrap()).unwrap();
            let (written, kept) = crate::chart::write::chart_xml(&previous);
            assert!(kept, "unchanged saved interpretation{version} preserves raw XML");
            assert!(crate::xmlx::parse(&written).unwrap().find(Local("preserved")).is_some());

            // An actual new identity edit is not silently stripped on an older model.
            let mut edited = serde_json::to_value(&previous).unwrap();
            edited["series"][0]["sourceIdx"] = serde_json::json!(17);
            let edited: Chart = serde_json::from_value(edited).unwrap();
            let (written, kept) = crate::chart::write::chart_xml(&edited);
            assert!(!kept, "a new ID edit must rebuild even with saved interpretation{version}");
            assert_eq!(review_written_ids(&written)[0].1, 17);
        }
        let current = read_chart_xml(xml.as_bytes()).unwrap();
        let current: Chart = serde_json::from_str(&serde_json::to_string(&current).unwrap()).unwrap();
        assert!(crate::chart::write::chart_xml(&current).1, "unchanged current identities also retain raw XML");
    }

    #[test]
    fn review_6094537368_reciprocal_axes_keep_duplicate_partner_and_candidate_order() {
        for tag in ["catAx", "dateAx", "valAx"] {
            for valid_first in [false, true] {
                for first_pair in [true, false] {
                    let partner = |cross| format!(r#"<c:{tag}><c:axId val="10"/><c:axPos val="b"/><c:crossAx val="{cross}"/></c:{tag}>"#);
                    let valid = partner(if first_pair { 20 } else { 999 });
                    let invalid = partner(999);
                    let partners = if valid_first { valid + &invalid } else { invalid + &valid };
                    let xml = metadata_chart(&format!(
                        r#"<c:barChart><c:barDir val="col"/><c:ser><c:tx><c:v>Secondary</c:v></c:tx></c:ser><c:axId val="30"/><c:axId val="40"/></c:barChart>
<c:valAx><c:axId val="70"/><c:axPos val="l"/><c:crossAx val="80"/></c:valAx>
<c:valAx><c:axId val="20"/><c:scaling><c:max val="10"/></c:scaling><c:axPos val="l"/><c:crossAx val="10"/></c:valAx>
<c:valAx><c:axId val="60"/><c:scaling><c:max val="60"/></c:scaling><c:axPos val="l"/><c:crossAx val="50"/></c:valAx>{partners}
<c:catAx><c:axId val="50"/><c:axPos val="b"/><c:crossAx val="60"/></c:catAx>
<c:catAx><c:axId val="30"/><c:axPos val="t"/><c:crossAx val="40"/></c:catAx>
<c:valAx><c:axId val="40"/><c:scaling><c:max val="90"/></c:scaling><c:axPos val="r"/><c:crossAx val="30"/></c:valAx>"#
                    ));
                    let chart = read_chart_xml(xml.as_bytes()).unwrap();
                    assert_eq!(chart.y_max, Some(if first_pair { 10.0 } else { 60.0 }), "tag={tag}, valid_first={valid_first}");
                    assert_eq!(chart.y2_max, Some(90.0));
                    assert!(chart.series[0].secondary_axis);
                }
            }
        }
    }

    #[test]
    fn review_6094777682_mixed_references_keep_literals_and_intentional_shared_ranges() {
        // Original two-series worksheet/cache fixture: sorting moves the literal before B's range.
        let xml = metadata_chart(
            r#"<c:barChart><c:barDir val="col"/>
<c:ser><c:idx val="17"/><c:order val="1"/><c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:pt idx="0"><c:v>Referenced</c:v></c:pt></c:strCache></c:strRef></c:tx><c:val><c:numRef><c:f>Sheet1!$B$2:$B$2</c:f><c:numCache><c:pt idx="0"><c:v>7</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
<c:ser><c:idx val="9"/><c:order val="0"/><c:tx><c:v>Literal</c:v></c:tx><c:val><c:numLit><c:pt idx="0"><c:v>2</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart>"#,
        );
        let mut chart = read_chart_xml(xml.as_bytes()).unwrap();
        chart.title = Some("Rebuild after sorting".into());
        let mut shared = chart.series[1].clone();
        shared.name = "Intentionally shared".into();
        chart.series.push(shared);
        let (written, kept) = crate::chart::write::chart_xml(&chart);
        assert!(!kept);
        let back = read_chart_xml(&written).unwrap();
        assert_eq!(back.series.iter().map(|s| s.values.clone()).collect::<Vec<_>>(), [vec![Some(2.0)], vec![Some(7.0)], vec![Some(7.0)]]);
        assert!(
            back.series[0].refs.name.is_none() && back.series[0].refs.values.is_none(),
            "a literal must not alias the populated B column after sorting"
        );
        for index in [1, 2] {
            assert_eq!(back.series[index].refs.name, chart.series[index].refs.name);
            assert_eq!(back.series[index].refs.values, chart.series[index].refs.values, "intentionally shared references are not reassigned");
        }
        assert_eq!(back.series[0].refs.categories.as_deref(), Some("Sheet1!$A$2:$A$2"), "the stable common-A category policy is unchanged");
        assert_eq!(review_written_ids(&written), [("Literal".into(), 9, 0), ("Referenced".into(), 17, 1), ("Intentionally shared".into(), 0, 2)]);
        let root = crate::xmlx::parse(&written).unwrap();
        let literal = root.path(&[Local("chart"), Local("plotArea"), Local("barChart"), Local("ser")]).unwrap();
        assert_eq!(literal.path(&[Local("tx"), Local("v")]).unwrap().deep_text(), "Literal");
        assert!(literal.path(&[Local("val"), Local("numLit")]).is_some());
        assert!(literal.child(Local("val")).unwrap().find(Local("f")).is_none());

        for kind in [ChartType::Column, ChartType::Scatter, ChartType::Bubble] {
            let mut c = Chart { kind, categories: vec!["1.5".into()], category_format: Some("0.0".into()), ..Default::default() };
            c.series.push(Series {
                name: "Referenced".into(),
                values: vec![Some(8.0)],
                refs: crate::chart::SeriesRefs { values: Some("Data!$Z$2:$Z$2".into()), ..Default::default() },
                ..Default::default()
            });
            c.series.push(Series {
                name: "Literal".into(),
                values: vec![Some(3.0)],
                x_values: vec![Some(1.5)],
                sizes: vec![Some(4.0)],
                ..Default::default()
            });
            let back = read_chart_xml(&crate::chart::write::chart_xml(&c).0).unwrap();
            assert_eq!(back.series[0].refs.values, c.series[0].refs.values);
            assert!(
                back.series[1].refs.name.is_none() && back.series[1].refs.values.is_none() && back.series[1].refs.sizes.is_none(),
                "{kind:?}: missing positional fields use caches, not fabricated ranges"
            );
            assert_eq!(back.series[1].refs.categories.as_deref(), Some("Sheet1!$A$2:$A$2"));
            assert_eq!(back.series[1].values, c.series[1].values);
            if kind == ChartType::Column {
                assert_eq!(back.category_format.as_deref(), Some("0.0"));
            } else {
                assert_eq!(back.series[1].x_values, c.series[1].x_values);
                if kind == ChartType::Bubble {
                    assert_eq!(back.series[1].sizes, c.series[1].sizes);
                }
            }
        }
    }

    #[test]
    fn review_6094777682_model_kinds_agree_with_rebuilt_plots_and_chart_wide_edits() {
        let make = |kind| Chart {
            kind: ChartType::Column,
            categories: vec!["Category".into()],
            series: vec![
                Series { values: vec![Some(7.0)], kind: Some(kind), ..Default::default() },
                Series { values: vec![Some(8.0)], kind: Some(kind), ..Default::default() },
            ],
            ..Default::default()
        };
        for (kind, extent) in [(ChartType::Column, 8.0), (ChartType::StackedColumn, 15.0)] {
            let chart = make(kind);
            assert_eq!(chart.series_kind(&chart.series[0]), kind);
            assert_eq!(chart.axis_range(false), Some((0.0, extent)));
            assert_eq!(chart.value_range(), (0.0, extent));
            assert!(chart.axis_scale(false, 5).unwrap().1 >= extent);
            let back = read_chart_xml(&crate::chart::write::chart_xml(&chart).0).unwrap();
            assert_eq!(back.kind, kind);
            assert_eq!(back.axis_range(false), chart.axis_range(false));
        }
        let xml = same_kind_chart_xml();
        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        for version in 0..=6 {
            let mut chart = read_chart(&root, MAX_CHART_POINTS, version).unwrap();
            chart.raw = Some(xml.clone());
            for s in &mut chart.series {
                s.kind = Some(ChartType::StackedColumn);
                s.secondary_axis = false;
            }
            assert_eq!(chart.series_kind(&chart.series[0]), ChartType::StackedColumn, "version{version}: explicit edit on unchanged raw chart kind");
            assert_eq!(chart.value_range(), (0.0, 10.0), "the imported explicit primary bound remains authoritative");
            chart.y_max = None;
            assert_eq!(
                chart.value_range(),
                (0.0, if version >= 5 { 7.0 } else { 9.0 }),
                "version{version}: retained source groups remain independent"
            );
            if version >= 5 {
                // Moving plots onto one axis does not itself merge imported stack membership.
                let group = chart.series[0].source_group;
                for series in &mut chart.series {
                    series.source_group = group;
                }
                assert_eq!(chart.value_range(), (0.0, 9.0), "an explicit same-group edit still sums the stack");
            }
            chart.kind = ChartType::Bar;
            assert_eq!(chart.series_kind(&chart.series[0]), ChartType::Bar, "version{version}: a real chart-wide edit overrides retained kinds");
            assert_eq!(read_chart_xml(&crate::chart::write::chart_xml(&chart).0).unwrap().kind, ChartType::Bar);
        }
        let mut combo = make(ChartType::Line);
        combo.kind = ChartType::Combo;
        combo.series[1].kind = None;
        assert_eq!(combo.series_kind(&combo.series[1]), ChartType::Line, "missing-kind fallback follows the first plot, like the writer");
        for unsupported in [ChartType::Pie, ChartType::Doughnut, ChartType::Sunburst, ChartType::Scatter, ChartType::Bubble] {
            combo.series[1].kind = Some(unsupported);
            assert_eq!(combo.series_kind(&combo.series[1]), ChartType::Column, "existing unsupported mixed plot fallback: {unsupported:?}");
        }
    }

    #[test]
    fn review_6094777682_secondary_percent_axes_use_percent_labels_without_extra_headroom() {
        for horizontal in [false, true] {
            let xml =
                same_kind_chart_xml().replace("clustered", "percentStacked").replace(r#"<c:max val="10"/>"#, "").replace(r#"<c:max val="90"/>"#, "");
            let xml = if horizontal { xml.replace(r#"<c:barDir val="col"/>"#, r#"<c:barDir val="bar"/>"#) } else { xml };
            let mut chart = read_chart_xml(xml.as_bytes()).unwrap();
            for secondary in [false, true] {
                assert_eq!(chart.axis_range(secondary), Some((0.0, 1.0)));
                assert_eq!(chart.axis_scale(secondary, 5), Some((0.0, 1.0, 0.2)), "horizontal={horizontal}, secondary={secondary}");
                assert_eq!(chart.axis_label(secondary, 0.5), "50%");
            }
            chart.y2_format = Some("General".into());
            assert_eq!(chart.axis_label(true, 0.5), "0.5", "an explicit secondary General remains authoritative");
            chart.y2_format = Some("0.00".into());
            assert_eq!(chart.axis_label(true, 0.5), "0.50");
            chart.y2_min = Some(2.0);
            chart.y2_max = Some(90.0);
            chart.y2_major = Some(5.0);
            assert_eq!(chart.axis_scale(true, 5), Some((2.0, 90.0, 5.0)), "explicit bounds and unit win");
        }
    }

    fn review_duplicate_axis_xml(vertical_last: bool) -> String {
        let bottom = r#"<c:valAx><c:axId val="10"/><c:axPos val="b"/><c:crossAx val="20"/></c:valAx>"#;
        let vertical = r#"<c:valAx><c:axId val="10"/><c:axPos val="r"/><c:crossAx val="999"/></c:valAx>"#;
        let partners = if vertical_last { format!("{bottom}{vertical}") } else { format!("{vertical}{bottom}") };
        metadata_chart(&format!(
            r#"<c:barChart><c:barDir val="col"/><c:ser><c:tx><c:v>Secondary</c:v></c:tx><c:val><c:numLit><c:pt idx="0"><c:v>7</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="30"/><c:axId val="40"/></c:barChart>
<c:valAx><c:axId val="20"/><c:axPos val="l"/><c:scaling><c:max val="10"/></c:scaling><c:crossAx val="10"/></c:valAx>{partners}
<c:catAx><c:axId val="30"/><c:axPos val="t"/><c:crossAx val="40"/></c:catAx><c:valAx><c:axId val="40"/><c:axPos val="r"/><c:scaling><c:max val="90"/></c:scaling><c:crossAx val="30"/></c:valAx>"#
        ))
    }

    fn review_duplicate_category_xml() -> String {
        metadata_chart(
            r#"<c:barChart><c:barDir val="col"/><c:ser/><c:axId val="10"/><c:axId val="20"/></c:barChart>
<c:catAx><c:axId val="10"/><c:axPos val="b"/><c:title><c:tx><c:v>Unrelated duplicate</c:v></c:tx></c:title><c:crossAx val="999"/></c:catAx>
<c:catAx><c:axId val="10"/><c:axPos val="b"/><c:title><c:tx><c:v>Reciprocal category</c:v></c:tx></c:title><c:crossAx val="20"/></c:catAx>
<c:valAx><c:axId val="20"/><c:axPos val="l"/><c:crossAx val="10"/></c:valAx>"#,
        )
    }

    #[test]
    fn review_6094777682_reciprocal_axes_use_the_partners_own_position_and_pair() {
        for vertical_last in [false, true] {
            let chart = read_chart_xml(review_duplicate_axis_xml(vertical_last).as_bytes()).unwrap();
            assert_eq!(chart.y_max, Some(10.0), "the valid bottom partner is independent of another axis sharing its ID");
            assert_eq!(chart.y2_max, Some(90.0));
            assert!(chart.series[0].secondary_axis);
        }
        let xml = review_duplicate_category_xml();
        assert_eq!(read_chart_xml(xml.as_bytes()).unwrap().axis_titles.0.as_deref(), Some("Reciprocal category"));
        let fallback = xml.replace(r#"<c:crossAx val="20"/>"#, r#"<c:crossAx val="998"/>"#);
        assert_eq!(
            read_chart_xml(fallback.as_bytes()).unwrap().axis_titles.0.as_deref(),
            Some("Unrelated duplicate"),
            "keep the original ID-only fallback when no reciprocal partner exists"
        );
    }

    #[test]
    fn review_6094777682_axis_interpretations_preserve_old_raw_and_real_metadata_edits() {
        for (category, fixture) in [(false, review_duplicate_axis_xml(true)), (true, review_duplicate_category_xml())] {
            let xml = fixture.replace("</c:chartSpace>", r#"<c:extLst><c:ext uri="urn:craft:original-axis-fixture"><own:preserved xmlns:own="urn:craft:original"/></c:ext></c:extLst></c:chartSpace>"#);
            let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
            for version in 0..=6 {
                let mut chart = read_chart(&root, MAX_CHART_POINTS, version).unwrap();
                chart.raw = Some(xml.clone());
                let chart: Chart = serde_json::from_str(&serde_json::to_string(&chart).unwrap()).unwrap();
                let (written, kept) = crate::chart::write::chart_xml(&chart);
                assert!(kept, "unchanged native interpretation{version} keeps the original malformed-duplicate XML");
                assert!(crate::xmlx::parse(&written).unwrap().find(Local("preserved")).is_some());
                if !category && (version == 2 || version == 3) {
                    assert_eq!(chart.y_max, Some(90.0), "retain the exact old last-ID-map interpretation");
                }
                if category {
                    assert_eq!(
                        chart.axis_titles.0.as_deref(),
                        Some(if version < 4 { "Unrelated duplicate" } else { "Reciprocal category" }),
                        "keep the native version's category interpretation"
                    );
                }
                let mut edit = chart.clone();
                edit.series[0].secondary_axis = !edit.series[0].secondary_axis;
                let (written, kept) = crate::chart::write::chart_xml(&edit);
                assert!(!kept, "interpretation{version} must not discard an actual axis ownership edit");
                assert_eq!(read_chart_xml(&written).unwrap().series[0].secondary_axis, edit.series[0].secondary_axis);
            }
            let current = read_chart_xml(xml.as_bytes()).unwrap();
            assert_eq!(current.raw_metadata_version, 10);
            if !category {
                assert_eq!(current.y_max, Some(10.0));
            }
        }
    }

    #[test]
    fn review_6094777682_capped_groups_keep_sorted_metadata_and_source_ids_attached() {
        let mut plot = String::from(r#"<c:barChart><c:barDir val="col"/>"#);
        for id in 0..=1000 {
            plot.push_str(&format!(r#"<c:ser><c:idx val="{id}"/><c:order val="{}"/><c:tx><c:v>Primary{id}</c:v></c:tx><c:val><c:numLit><c:pt idx="0"><c:v>{id}</c:v></c:pt></c:numLit></c:val></c:ser>"#, id + 1));
        }
        plot.push_str(r#"<c:axId val="10"/><c:axId val="20"/></c:barChart><c:barChart><c:barDir val="col"/><c:ser><c:idx val="9000"/><c:order val="0"/><c:tx><c:v>Later secondary</c:v></c:tx><c:val><c:numLit><c:pt idx="0"><c:v>123</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="30"/><c:axId val="40"/></c:barChart><c:catAx><c:axId val="10"/><c:axPos val="b"/><c:crossAx val="20"/></c:catAx><c:valAx><c:axId val="20"/><c:axPos val="l"/><c:crossAx val="10"/></c:valAx><c:catAx><c:axId val="30"/><c:axPos val="t"/><c:crossAx val="40"/></c:catAx><c:valAx><c:axId val="40"/><c:axPos val="r"/><c:crossAx val="30"/></c:valAx>"#);
        let mut chart = read_chart_xml(metadata_chart(&plot).as_bytes()).unwrap();
        assert_eq!(chart.series.len(), 1001, "retain the existing per-group cap and the later group's record");
        let first = chart.series[0].clone();
        assert_eq!(
            (first.name.as_str(), first.values.as_slice(), first.kind, first.secondary_axis),
            ("Later secondary", [Some(123.0)].as_slice(), Some(ChartType::Column), true)
        );
        assert_eq!(serde_json::to_value(&first).unwrap()["sourceIdx"], 9000);
        assert!(chart.series.iter().all(|s| s.name != "Primary1000"));
        assert_eq!(chart.series.last().unwrap().values, [Some(999.0)]);
        chart.title = Some("Rebuild capped records".into());
        let written = crate::chart::write::chart_xml(&chart).0;
        let ids = review_written_ids(&written);
        assert_eq!(ids.len(), 1001);
        assert!(ids.iter().any(|(name, id, order)| name == "Later secondary" && *id == 9000 && *order == 0));
        assert!(ids.iter().all(|(_, id, _)| *id != 1000));
        let back = read_chart_xml(&written).unwrap();
        assert_eq!(back.series[0].values, first.values);
        assert!(back.series[0].secondary_axis);
    }

    #[test]
    fn effective_kind_context_projects_ambiguous_raw_once_per_operation() {
        let xml = same_kind_chart_xml();
        let mut chart = read_chart_xml(xml.as_bytes()).unwrap();
        for s in &mut chart.series {
            s.kind = Some(ChartType::StackedColumn);
        }
        let reset = || RAW_PROJECTIONS.with(|count| count.set(0));
        let count = || RAW_PROJECTIONS.with(std::cell::Cell::get);
        reset();
        let kinds = chart.series_kinds();
        for _ in 0..5 {
            for s in &chart.series {
                assert_eq!(kinds.kind(s), ChartType::StackedColumn);
            }
        }
        assert_eq!(count(), 1, "the reusable context resolves the ambiguous raw baseline only once");
        reset();
        let _ = chart.axis_range(false);
        assert_eq!(count(), 1);
        reset();
        let _ = chart.axis_scale(true, 5);
        assert_eq!(count(), 1);
        reset();
        let _ = chart.value_range();
        assert_eq!(count(), 1);
        reset();
        let _ = crate::chart::write::chart_xml(&chart);
        assert_eq!(count(), 1, "writer reuses its existing projection instead of reparsing for the policy");

        chart.raw = None;
        reset();
        assert_eq!(chart.series_kind(&chart.series[0]), ChartType::StackedColumn);
        let _ = chart.axis_scale(true, 5);
        assert_eq!(count(), 0, "ordinary new charts have no original XML to parse");
        chart.raw = Some(xml);
        chart.kind = ChartType::Combo;
        reset();
        let _ = chart.value_range();
        assert_eq!(count(), 0, "Combo has no chart-wide override");
        chart.kind = ChartType::StackedColumn;
        reset();
        let _ = chart.axis_range(false);
        assert_eq!(count(), 0, "matching retained kinds need no raw-kind decision");
    }

    #[test]
    fn review_6095125890_selected_value_axis_keeps_the_exact_duplicate_element_metadata() {
        // Original synthetic pair: the first duplicate is not the reciprocal primary axis.
        let plot = r#"<c:barChart><c:barDir val="col"/><c:ser><c:tx><c:v>Secondary</c:v></c:tx><c:val><c:numLit><c:pt idx="0"><c:v>7</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="30"/><c:axId val="40"/></c:barChart>
<c:valAx><c:axId val="20"/><c:axPos val="r"/><c:scaling><c:min val="0"/><c:max val="1"/></c:scaling><c:numFmt formatCode="0.00" sourceLinked="0"/><c:delete val="1"/><c:title><c:tx><c:v>Wrong value</c:v></c:tx></c:title><c:txPr><a:p><a:pPr><a:defRPr sz="1000"/></a:pPr></a:p></c:txPr><c:crossAx val="999"/></c:valAx>
<c:valAx><c:axId val="20"/><c:axPos val="l"/><c:scaling><c:min val="2"/><c:max val="10"/></c:scaling><c:majorUnit val="2"/><c:numFmt formatCode="0%" sourceLinked="0"/><c:delete val="0"/><c:title><c:tx><c:v>Selected value</c:v></c:tx></c:title><c:txPr><a:p><a:pPr><a:defRPr sz="1600"/></a:pPr></a:p></c:txPr><c:crossAx val="10"/></c:valAx>
<c:catAx><c:axId val="10"/><c:axPos val="b"/><c:title><c:tx><c:v>Selected category</c:v></c:tx></c:title><c:crossAx val="20"/></c:catAx>
<c:catAx><c:axId val="30"/><c:axPos val="t"/><c:crossAx val="40"/></c:catAx><c:valAx><c:axId val="40"/><c:axPos val="r"/><c:scaling><c:max val="90"/></c:scaling><c:crossAx val="30"/></c:valAx>"#;
        let xml = metadata_chart(plot).replace("</c:chartSpace>", r#"<c:extLst><c:ext uri="urn:craft:selected-axis-original"><own:preserved xmlns:own="urn:craft:original"/></c:ext></c:extLst></c:chartSpace>"#);
        let current = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!((current.y_min, current.y_max, current.y_major), (Some(2.0), Some(10.0), Some(2.0)));
        assert_eq!(current.y_format.as_deref(), Some("0%"));
        assert!(!current.y_hidden, "visibility belongs to the same selected element");
        assert_eq!(current.axis_titles, (Some("Selected category".into()), Some("Selected value".into())));
        assert_eq!(current.text_size, Some(16.0), "text style belongs to the selected element too");
        assert!(current.series[0].secondary_axis);
        assert_eq!(current.y2_max, Some(90.0));

        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        for version in 0..=4 {
            let mut previous = read_chart(&root, MAX_CHART_POINTS, version).unwrap();
            assert_eq!(previous.y_max, Some(if version < 2 { 90.0 } else { 1.0 }), "exact historical interpretation{version}");
            assert!(serde_json::to_value(&previous.series[0]).unwrap().get("sourceGroup").is_none());
            previous.raw = Some(xml.clone());
            assert!(crate::chart::write::chart_xml(&previous).1, "old native interpretation{version} must keep its raw part");
        }
        assert!(crate::chart::write::chart_xml(&current).1, "unchanged current model keeps raw too");
        let mut edited = current;
        edited.y_max = Some(12.0);
        let (written, kept) = crate::chart::write::chart_xml(&edited);
        assert!(!kept, "an actual metadata edit must rebuild");
        assert_eq!(read_chart_xml(&written).unwrap().y_max, Some(12.0));
    }

    #[test]
    fn review_6095125890_numeric_categories_use_the_reciprocal_supplied_group_format() {
        let plot = r#"<c:barChart><c:barDir val="col"/><c:ser><c:cat><c:numLit><c:formatCode>General</c:formatCode><c:pt idx="0"><c:v>45292</c:v></c:pt></c:numLit></c:cat><c:val><c:numLit><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser><c:axId val="10"/><c:axId val="20"/></c:barChart>
<c:catAx><c:axId val="10"/><c:axPos val="b"/><c:title><c:tx><c:v>Unrelated</c:v></c:tx></c:title><c:numFmt formatCode="0.00" sourceLinked="0"/><c:crossAx val="999"/></c:catAx>
<c:catAx><c:axId val="10"/><c:axPos val="b"/><c:title><c:tx><c:v>Reciprocal</c:v></c:tx></c:title><c:numFmt formatCode="mmm-yy" sourceLinked="0"/><c:crossAx val="20"/></c:catAx>
<c:valAx><c:axId val="20"/><c:axPos val="l"/><c:crossAx val="10"/></c:valAx>"#;
        let xml = metadata_chart(plot);
        let mut current = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(current.axis_titles.0.as_deref(), Some("Reciprocal"));
        assert_eq!(current.category_format.as_deref(), Some("mmm-yy"));
        assert_eq!(current.category_label(0), "Jan-24");
        assert_eq!(current.categories, ["45292"]);
        current.title = Some("Rebuilt date categories".into());
        let back = read_chart_xml(&crate::chart::write::chart_xml(&current).0).unwrap();
        assert_eq!(back.category_label(0), current.category_label(0));
        assert_eq!(back.categories, current.categories);

        let fallback = xml.replace(r#"<c:crossAx val="20"/>"#, r#"<c:crossAx val="998"/>"#);
        let fallback = read_chart_xml(fallback.as_bytes()).unwrap();
        assert_eq!(fallback.category_format.as_deref(), Some("0.00"), "no reciprocal partner retains the ID-only fallback");
        assert_eq!(fallback.category_label(0), "45292.00");
        let explicit_general = xml.replace(r#"formatCode="mmm-yy" sourceLinked="0""#, r#"formatCode="General" sourceLinked="0""#);
        assert_eq!(read_chart_xml(explicit_general.as_bytes()).unwrap().category_label(0), "45292");
        let linked = xml
            .replace("<c:formatCode>General</c:formatCode>", "<c:formatCode>0.0</c:formatCode>")
            .replace(r#"formatCode="mmm-yy" sourceLinked="0""#, r#"formatCode="mmm-yy" sourceLinked="1""#);
        assert_eq!(
            read_chart_xml(linked.as_bytes()).unwrap().category_label(0),
            "45292.0",
            "source-linked format still comes from the supplied cache"
        );
        let text = xml.replace(
            "<c:numLit><c:formatCode>General</c:formatCode><c:pt idx=\"0\"><c:v>45292</c:v></c:pt></c:numLit>",
            "<c:strLit><c:pt idx=\"0\"><c:v>45292</c:v></c:pt></c:strLit>",
        );
        let text = read_chart_xml(text.as_bytes()).unwrap();
        assert_eq!(text.category_format, None);
        assert_eq!(text.category_label(0), "45292");

        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        for version in 0..=4 {
            let mut previous = read_chart(&root, MAX_CHART_POINTS, version).unwrap();
            assert_eq!(previous.category_format.as_deref(), Some("0.00"), "retain the exact older numeric category interpretation{version}");
            previous.raw = Some(xml.clone());
            assert!(crate::chart::write::chart_xml(&previous).1);
        }
    }

    #[test]
    fn review_6095125890_source_groups_survive_title_edits_without_cross_group_stacking_or_loss() {
        for grouping in ["clustered", "stacked"] {
            let mut plot = String::new();
            for group in 0..2 {
                plot.push_str(&format!(r#"<c:barChart><c:barDir val="col"/><c:grouping val="{grouping}"/>"#));
                for offset in 0..600 {
                    let id = group * 600 + offset;
                    // Interleave display order: group membership must travel with the whole record.
                    let order = offset * 2 + group;
                    plot.push_str(&format!(r#"<c:ser><c:idx val="{id}"/><c:order val="{order}"/><c:tx><c:v>Group{group}-{offset}</c:v></c:tx><c:val><c:numLit><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser>"#));
                }
                plot.push_str(r#"<c:axId val="10"/><c:axId val="20"/></c:barChart>"#);
            }
            plot.push_str(r#"<c:catAx><c:axId val="10"/><c:axPos val="b"/><c:crossAx val="20"/></c:catAx><c:valAx><c:axId val="20"/><c:axPos val="l"/><c:crossAx val="10"/></c:valAx>"#);
            let mut current = read_chart_xml(metadata_chart(&plot).as_bytes()).unwrap();
            assert_eq!(current.series.len(), 1200);
            let membership = |chart: &Chart| {
                let records = serde_json::to_value(&chart.series).unwrap();
                let group0 = records[0].get("sourceGroup").cloned().expect("current imports retain a source group");
                let group1 = records[1].get("sourceGroup").cloned().expect("both groups retain membership");
                assert_ne!(group0, group1);
                for (i, record) in records.as_array().unwrap().iter().enumerate() {
                    assert_eq!(record.get("sourceGroup"), Some(if i % 2 == 0 { &group0 } else { &group1 }));
                }
            };
            let names: Vec<_> = current.series.iter().map(|s| s.name.clone()).collect();
            current.title = Some("Title-only rebuild".into());
            let (written, kept) = crate::chart::write::chart_xml(&current);
            assert!(!kept);
            let back = read_chart_xml(&written).unwrap();
            assert_eq!(back.series.len(), 1200, "{grouping}: no retained source group may exceed the reader cap on rebuild");
            membership(&current);
            assert_eq!(back.series.iter().map(|s| s.name.clone()).collect::<Vec<_>>(), names);
            assert_eq!(
                back.series.iter().map(|s| s.values.clone()).collect::<Vec<_>>(),
                current.series.iter().map(|s| s.values.clone()).collect::<Vec<_>>()
            );
            assert_eq!(
                back.series.iter().map(|s| serde_json::to_value(s).unwrap()["sourceIdx"].clone()).collect::<Vec<_>>(),
                current.series.iter().map(|s| serde_json::to_value(s).unwrap()["sourceIdx"].clone()).collect::<Vec<_>>()
            );
            membership(&back); // Numeric tokens may renumber; this checks the membership relation.
            if grouping == "stacked" {
                assert_eq!(current.axis_range(false), Some((0.0, 600.0)), "the two independent source stacks do not sum together");
                assert_eq!(back.axis_range(false), current.axis_range(false));
            }
        }
    }

    // Original synthetic pairs: numeric IDs are local references, not element identities.
    fn review_6095656137_group(category: u32, value: u32, name: &str, format: &str) -> String {
        format!(
            r#"<c:barChart><c:barDir val="col"/><c:ser><c:tx><c:v>{name}</c:v></c:tx><c:val><c:numRef><c:f>{name}!B1</c:f><c:numCache><c:formatCode>{format}</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>7</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:axId val="{category}"/><c:axId val="{value}"/></c:barChart>"#
        )
    }

    fn review_6095656137_axis(kind: &str, id: u32, cross: u32, pos: &str, max: u32, format: &str) -> String {
        let min = if kind == "valAx" { 2 } else { 0 };
        format!(
            r#"<c:{kind}><c:axId val="{id}"/><c:scaling><c:max val="{max}"/><c:min val="{min}"/></c:scaling><c:axPos val="{pos}"/>{format}<c:title><c:tx><c:v>Axis{id}-{cross}</c:v></c:tx></c:title><c:crossAx val="{cross}"/></c:{kind}>"#
        )
    }

    #[test]
    fn review_6095656137_shared_value_ids_keep_distinct_group_ownership_and_scales() {
        let groups = format!("{}{}", review_6095656137_group(10, 40, "Left", "0%"), review_6095656137_group(30, 40, "Right", "0.00"));
        let left = review_6095656137_axis("valAx", 40, 10, "l", 50, r#"<c:numFmt formatCode="0%" sourceLinked="0"/>"#);
        let right = review_6095656137_axis("valAx", 40, 30, "r", 90, r#"<c:numFmt formatCode="0.00" sourceLinked="0"/>"#);
        let cats = format!("{}{}", review_6095656137_axis("catAx", 10, 40, "b", 1, ""), review_6095656137_axis("catAx", 30, 40, "t", 1, ""));
        for axes in [format!("{left}{right}{cats}"), format!("{cats}{right}{left}")] {
            let xml = metadata_chart(&format!("{groups}{axes}"));
            let chart = read_chart_xml(xml.as_bytes()).unwrap();
            assert_eq!(chart.series.iter().map(|s| (s.name.as_str(), s.secondary_axis)).collect::<Vec<_>>(), [("Left", false), ("Right", true)]);
            assert_eq!((chart.y_min, chart.y_max, chart.y2_min, chart.y2_max), (Some(2.0), Some(50.0), Some(2.0), Some(90.0)));
            assert_eq!((chart.y_format.as_deref(), chart.y2_format.as_deref()), (Some("0%"), Some("0.00")));
            assert_eq!(chart.axis_titles.1.as_deref(), Some("Axis40-10"));
            assert!(crate::chart::write::chart_xml(&chart).1);
            let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
            for version in 0..=6 {
                let mut previous = read_chart(&root, MAX_CHART_POINTS, version).unwrap();
                if version == 6 {
                    assert_eq!(previous.series.iter().map(|s| s.secondary_axis).collect::<Vec<_>>(), [false, false]);
                    assert_eq!(previous.y2_max, None);
                }
                previous.raw = Some(xml.clone());
                let previous: Chart = serde_json::from_str(&serde_json::to_string(&previous).unwrap()).unwrap();
                let (written, kept) = crate::chart::write::chart_xml(&previous);
                assert!(kept, "unchanged saved interpretation{version} must retain its raw part");
                assert_eq!(crate::xmlx::parse(&written).unwrap().to_xml(), root.to_xml());
            }
        }
        // An unused primary fallback is not proof that either populated group owns it.
        // The unresolved left group keeps ID fallback; the uniquely resolved right is secondary.
        let invalid_left = left.replace(r#"<c:crossAx val="10"/>"#, r#"<c:crossAx val="777"/>"#);
        let unused = format!("{}{}", review_6095656137_axis("valAx", 40, 999, "l", 70, ""), review_6095656137_axis("catAx", 999, 40, "b", 1, ""));
        let xml = metadata_chart(&format!("{groups}{invalid_left}{right}{cats}{unused}"));
        let chart = read_chart_xml(xml.as_bytes()).unwrap();
        assert_eq!(chart.y_max, Some(70.0));
        assert_eq!(
            chart.series.iter().map(|s| s.secondary_axis).collect::<Vec<_>>(),
            [false, true],
            "the unresolved group keeps ID fallback while the resolved right group is secondary"
        );
        assert_eq!(chart.y2_max, Some(90.0));
        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        let old8 = read_chart(&root, MAX_CHART_POINTS, 8).unwrap();
        assert_eq!(old8.series.iter().map(|s| s.secondary_axis).collect::<Vec<_>>(), [false, false]);
        assert_eq!(old8.y2_max, None, "saved8 retains its original ownership interpretation");
    }

    #[test]
    fn review_6095656137_scatter_direction_uses_the_actual_pair_not_a_stray_duplicate() {
        let primary = review_6095656137_group(1, 2, "Primary", "0");
        let scatter = r#"<c:scatterChart><c:ser><c:tx><c:v>Scatter</c:v></c:tx><c:xVal><c:numLit><c:formatCode>General</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>30</c:v></c:pt></c:numLit></c:xVal><c:yVal><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>7</c:v></c:pt></c:numLit></c:yVal></c:ser><c:axId val="10"/><c:axId val="20"/></c:scatterChart>"#;
        let primary_axes = format!("{}{}", review_6095656137_axis("catAx", 1, 2, "b", 1, ""), review_6095656137_axis("valAx", 2, 1, "l", 50, ""));
        let x = review_6095656137_axis("valAx", 10, 20, "b", 12, r#"<c:numFmt formatCode="0.0" sourceLinked="0"/>"#);
        let y = review_6095656137_axis("valAx", 20, 10, "r", 90, "");
        let stray = review_6095656137_axis("valAx", 10, 999, "l", 999, r#"<c:numFmt formatCode="0%" sourceLinked="0"/>"#);
        let foreign = review_6095656137_axis("catAx", 999, 10, "b", 1, "");
        for (axes, primary_resolves) in
            [(format!("{x}{y}{stray}"), true), (format!("{stray}{y}{x}"), true), (format!("{x}{foreign}{stray}{y}"), false)]
        {
            let xml = metadata_chart(&format!("{primary}{scatter}{primary_axes}{axes}"));
            let chart = read_chart_xml(xml.as_bytes()).unwrap();
            assert_eq!(chart.series.iter().map(|s| s.secondary_axis).collect::<Vec<_>>(), [false, true]);
            assert_eq!(
                (chart.y_max, chart.y2_max),
                (Some(50.0), Some(90.0)),
                "secondary scale belongs to Y, not the horizontal X sharing an ID with a stray"
            );
            assert_eq!(chart.category_format.as_deref(), Some("0.0"));
            assert_eq!(chart.category_label(0), "30.0");
            assert_eq!(chart.series[1].x_values, [Some(30.0)]);
            let pure_xml = metadata_chart(&format!("{scatter}{axes}"));
            let pure = read_chart_xml(pure_xml.as_bytes()).unwrap();
            if primary_resolves {
                assert!(!pure.series[0].secondary_axis);
                assert_eq!(pure.y_max, Some(90.0));
                assert_eq!(pure.axis_titles, (Some("Axis10-20".into()), Some("Axis20-10".into())));
                assert_eq!(pure.category_format.as_deref(), Some("0.0"));
                assert_eq!(pure.category_label(0), "30.0");
            } else {
                // A valid unused primary pair remains the anchor, as when primary series
                // are deleted. Its resolved overlay still owns the distinct right Y element.
                assert_eq!((pure.series[0].secondary_axis, pure.y_max, pure.y2_max), (true, Some(999.0), Some(90.0)));
                let root = crate::xmlx::parse(pure_xml.as_bytes()).unwrap();
                let old = read_chart(&root, MAX_CHART_POINTS, 6).unwrap();
                assert_eq!((old.series[0].secondary_axis, old.y_max, old.y2_max), (false, Some(999.0), None));
                let old8 = read_chart(&root, MAX_CHART_POINTS, 8).unwrap();
                assert_eq!((old8.series[0].secondary_axis, old8.y_max, old8.y2_max), (false, Some(999.0), None));
            }
        }
        // Ambiguous duplicate elements and an unresolved pair keep the ID-only fallback.
        for axes in [format!("{x}{y}{y}{stray}"), format!("{x}{stray}")] {
            let xml = metadata_chart(&format!("{primary}{scatter}{primary_axes}{axes}"));
            let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
            let old = read_chart(&root, MAX_CHART_POINTS, 6).unwrap();
            let current = read_chart_xml(xml.as_bytes()).unwrap();
            assert_eq!(
                current.series.iter().map(|s| s.secondary_axis).collect::<Vec<_>>(),
                old.series.iter().map(|s| s.secondary_axis).collect::<Vec<_>>()
            );
            assert_eq!((current.y2_max, current.category_format), (old.y2_max, old.category_format));
        }
        // Two horizontal value candidates are equally eligible, not an arbitrarily picked Y.
        let both_horizontal = y.replace(r#"<c:axPos val="r"/>"#, r#"<c:axPos val="t"/>"#);
        let xml = metadata_chart(&format!("{primary}{scatter}{primary_axes}{x}{both_horizontal}"));
        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        assert_eq!(read_chart_xml(xml.as_bytes()).unwrap().y2_max, read_chart(&root, MAX_CHART_POINTS, 6).unwrap().y2_max);
    }

    #[test]
    fn review_6095656137_source_linked_formats_follow_resolved_owner_elements() {
        let foreign = review_6095656137_group(999, 40, "Foreign", "0%");
        let left = review_6095656137_group(10, 40, "Left", "0.00");
        let right = review_6095656137_group(30, 40, "Right", "0.000");
        let cats = format!("{}{}", review_6095656137_axis("catAx", 10, 40, "b", 1, ""), review_6095656137_axis("catAx", 30, 40, "t", 1, ""));
        for format in ["", r#"<c:numFmt formatCode="General" sourceLinked="1"/>"#] {
            let l = review_6095656137_axis("valAx", 40, 10, "l", 50, format);
            let r = review_6095656137_axis("valAx", 40, 30, "r", 90, format);
            for (groups, flags) in
                [(format!("{foreign}{left}{right}"), vec![false, false, true]), (format!("{right}{foreign}{left}"), vec![true, false, false])]
            {
                for axes in [format!("{l}{r}{cats}"), format!("{cats}{r}{l}")] {
                    let xml = metadata_chart(&format!("{groups}{axes}"));
                    let chart = read_chart_xml(xml.as_bytes()).unwrap();
                    assert_eq!((chart.y_format.as_deref(), chart.y2_format.as_deref()), (Some("0.00"), Some("0.000")));
                    assert_eq!(chart.series.iter().map(|s| s.secondary_axis).collect::<Vec<_>>(), flags);
                    assert_eq!((chart.y_max, chart.y2_max), (Some(50.0), Some(90.0)));
                    if !format.is_empty() {
                        let explicit = xml.replace(format, r#"<c:numFmt formatCode="0.0E+00" sourceLinked="0"/>"#);
                        let explicit = read_chart_xml(explicit.as_bytes()).unwrap();
                        assert_eq!((explicit.y_format.as_deref(), explicit.y2_format.as_deref()), (Some("0.0E+00"), Some("0.0E+00")));
                    }
                }
            }
        }
        // A selected fallback with no resolved owner retains its historical source-ID format.
        let xml = metadata_chart(&format!("{foreign}{}", review_6095656137_axis("valAx", 40, 999, "l", 50, "")));
        assert_eq!(read_chart_xml(xml.as_bytes()).unwrap().y_format.as_deref(), Some("0%"));
        // Multiple resolved owners still use the first source-order cache on that element.
        let alias = review_6095656137_group(10, 40, "LaterLeft", "0.0000");
        let xml = metadata_chart(&format!(
            "{left}{alias}{right}{cats}{}{}",
            review_6095656137_axis("valAx", 40, 10, "l", 50, ""),
            review_6095656137_axis("valAx", 40, 30, "r", 90, "")
        ));
        assert_eq!(read_chart_xml(xml.as_bytes()).unwrap().y_format.as_deref(), Some("0.00"));
    }

    #[test]
    fn review_6095873677_ambiguous_scatter_and_bubble_y_keep_the_id_fallback() {
        let primary = review_6095656137_group(1, 2, "Primary", "0");
        let primary_axes = format!("{}{}", review_6095656137_axis("catAx", 1, 2, "b", 1, ""), review_6095656137_axis("valAx", 2, 1, "l", 50, ""));
        let x = review_6095656137_axis("valAx", 10, 20, "b", 12, r#"<c:numFmt formatCode="0.0" sourceLinked="0"/>"#);
        let top_y = review_6095656137_axis("valAx", 20, 10, "t", 70, r#"<c:numFmt formatCode="0.00" sourceLinked="0"/>"#);
        let right_y = review_6095656137_axis("valAx", 20, 10, "r", 90, r#"<c:numFmt formatCode="0.000" sourceLinked="0"/>"#);
        for kind in ["scatterChart", "bubbleChart"] {
            let size = if kind == "bubbleChart" {
                r#"<c:bubbleSize><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:bubbleSize>"#
            } else {
                ""
            };
            let group = format!(
                r#"<c:{kind}><c:ser><c:tx><c:v>Overlay</c:v></c:tx><c:xVal><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>30</c:v></c:pt></c:numLit></c:xVal><c:yVal><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>7</c:v></c:pt></c:numLit></c:yVal>{size}</c:ser><c:axId val="10"/><c:axId val="20"/></c:{kind}>"#
            );
            for (axes, fallback_max) in [(format!("{x}{top_y}{right_y}"), 70.0), (format!("{right_y}{top_y}{x}"), 12.0)] {
                let xml = metadata_chart(&format!("{primary}{group}{primary_axes}{axes}"));
                let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
                let current = read_chart_xml(xml.as_bytes()).unwrap();
                assert_eq!(current.y2_max, Some(fallback_max), "{kind}: an ambiguous Y must not reverse-resolve horizontal X");
                assert_eq!(current.series.iter().map(|s| s.secondary_axis).collect::<Vec<_>>(), [false, true]);
                let mut fallback = read_chart(&root, MAX_CHART_POINTS, 6).unwrap();
                // Version6's direction cannot be invented from the remaining horizontal partner.
                // Compare every typed field, normalizing only raw/projection provenance.
                fallback.raw_metadata_version = current.raw_metadata_version;
                fallback.raw = current.raw.clone();
                assert_eq!(serde_json::to_value(&current).unwrap(), serde_json::to_value(&fallback).unwrap());
            }
        }
        // Horizontal bar charts legitimately have a bottom value axis and a left category.
        let bar = review_6095656137_group(10, 20, "Horizontal", "0").replace(r#"barDir val="col""#, r#"barDir val="bar""#);
        let plot = format!("{bar}{}{}", review_6095656137_axis("catAx", 10, 20, "l", 1, ""), review_6095656137_axis("valAx", 20, 10, "b", 90, ""));
        let chart = read_chart_xml(metadata_chart(&plot).as_bytes()).unwrap();
        assert_eq!(chart.kind, ChartType::Bar);
        assert_eq!(chart.y_max, Some(90.0));
        assert!(!chart.series[0].secondary_axis);
        assert_eq!(chart.axis_titles, (Some("Axis10-20".into()), Some("Axis20-10".into())));
    }

    #[test]
    fn review_6095873677_secondary_fallback_excludes_the_resolved_primary_scatter() {
        let primary = r#"<c:scatterChart><c:ser><c:order val="0"/><c:tx><c:v>Primary scatter</c:v></c:tx><c:xVal><c:numLit><c:pt idx="0"><c:v>30</c:v></c:pt></c:numLit></c:xVal><c:yVal><c:numLit><c:pt idx="0"><c:v>7</c:v></c:pt></c:numLit></c:yVal></c:ser><c:axId val="10"/><c:axId val="20"/></c:scatterChart>"#;
        let secondary = review_6095656137_group(30, 40, "Unresolved secondary", "0.000").replace("<c:ser>", r#"<c:ser><c:order val="1"/>"#);
        let axes = format!(
            "{}{}{}{}",
            review_6095656137_axis("valAx", 10, 20, "b", 12, ""),
            review_6095656137_axis("valAx", 20, 10, "l", 50, r#"<c:numFmt formatCode="0.00" sourceLinked="0"/>"#),
            review_6095656137_axis("valAx", 10, 999, "l", 999, ""),
            review_6095656137_axis("valAx", 40, 888, "r", 90, r#"<c:numFmt formatCode="0.000" sourceLinked="0"/>"#)
        );
        for groups in [format!("{primary}{secondary}"), format!("{secondary}{primary}")] {
            let chart = read_chart_xml(metadata_chart(&format!("{groups}{axes}")).as_bytes()).unwrap();
            assert_eq!((chart.y_max, chart.y2_max), (Some(50.0), Some(90.0)), "the primary group's corrupt ID-only X role must not supply y2");
            assert_eq!(
                chart.series.iter().map(|s| (s.name.as_str(), s.secondary_axis)).collect::<Vec<_>>(),
                [("Primary scatter", false), ("Unresolved secondary", true)]
            );
            assert_eq!((chart.y_format.as_deref(), chart.y2_format.as_deref()), (Some("0.00"), Some("0.000")));
            assert_eq!(chart.axis_titles.1.as_deref(), Some("Axis20-10"));
        }
    }

    #[test]
    fn review_6095873677_axis_formats_use_boolean_source_linking_and_its_default() {
        let left = review_6095656137_group(10, 40, "Left", "0.00")
            .replace("<c:val>", r#"<c:cat><c:numLit><c:formatCode>0.0</c:formatCode><c:pt idx="0"><c:v>30</c:v></c:pt></c:numLit></c:cat><c:val>"#);
        let right = review_6095656137_group(30, 40, "Right", "0.000");
        let fixture = |attributes: Option<&str>, own: &str| {
            let fmt = attributes.map(|a| format!(r#"<c:numFmt formatCode="{own}" {a}/>"#)).unwrap_or_default();
            metadata_chart(&format!(
                "{left}{right}{}{}{}{}",
                review_6095656137_axis("catAx", 10, 40, "b", 1, &fmt),
                review_6095656137_axis("valAx", 40, 10, "l", 50, &fmt),
                review_6095656137_axis("catAx", 30, 40, "t", 1, &fmt),
                review_6095656137_axis("valAx", 40, 30, "r", 90, &fmt)
            ))
        };
        for attribute in [Some(r#"sourceLinked="true""#), Some(""), Some(r#"sourceLinked="1""#), None] {
            let chart = read_chart_xml(fixture(attribute, "General").as_bytes()).unwrap();
            assert_eq!(
                (chart.y_format.as_deref(), chart.y2_format.as_deref()),
                (Some("0.00"), Some("0.000")),
                "{attribute:?}: use each actual owner's value-cache format"
            );
            assert_eq!(chart.category_format.as_deref(), Some("0.0"));
            assert_eq!(chart.category_label(0), "30.0");
        }
        for attribute in [r#"sourceLinked="false""#, r#"sourceLinked="0""#] {
            for own in ["General", "0.0000"] {
                let chart = read_chart_xml(fixture(Some(attribute), own).as_bytes()).unwrap();
                assert_eq!((chart.y_format.as_deref(), chart.y2_format.as_deref()), (Some(own), Some(own)));
                assert_eq!(chart.category_format.as_deref(), Some(own));
                assert_eq!(chart.category_label(0), if own == "General" { "30" } else { "30.0000" });
            }
        }
        let xml = fixture(Some(""), "General").replace(
            "</c:chartSpace>",
            r#"<c:extLst><c:ext uri="urn:craft:linked-original"><own:preserved xmlns:own="urn:craft:original"/></c:ext></c:extLst></c:chartSpace>"#,
        );
        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        for version in 0..=7 {
            let mut previous = read_chart(&root, MAX_CHART_POINTS, version).unwrap();
            assert_eq!(previous.y_format.as_deref(), Some("General"), "old{version} literal value-axis decoding");
            assert_eq!(previous.category_format.as_deref(), Some("General"), "old{version} explicit General category decoding");
            previous.raw = Some(xml.clone());
            let previous: Chart = serde_json::from_str(&serde_json::to_string(&previous).unwrap()).unwrap();
            let (written, kept) = crate::chart::write::chart_xml(&previous);
            assert!(kept, "unchanged old{version} native comparison keeps raw XML");
            assert_eq!(crate::xmlx::parse(&written).unwrap().to_xml(), root.to_xml());
        }
    }

    #[test]
    fn review_6095873677_label_linking_keeps_series_group_source_precedence() {
        let fixture = |series: Option<&str>, group: Option<&str>| {
            let labels = |attribute: Option<&str>, format: &str| {
                attribute.map(|a| format!(r#"<c:dLbls><c:numFmt formatCode="{format}" {a}/></c:dLbls>"#)).unwrap_or_default()
            };
            let series_labels = labels(series, "0%");
            let group_labels = labels(group, "0.0000");
            let plot = review_6095656137_group(10, 20, "Labels", "0.00")
                .replace("<c:val>", &format!("{series_labels}<c:val>"))
                .replace("</c:ser>", &format!("</c:ser>{group_labels}"));
            metadata_chart(&plot)
        };
        for attribute in [r#"sourceLinked="true""#, "", r#"sourceLinked="1""#] {
            let chart = read_chart_xml(fixture(Some(attribute), Some(attribute)).as_bytes()).unwrap();
            assert_eq!(chart.series[0].number_format.as_deref(), Some("0.00"), "linked labels use the source when neither label layer is explicit");
            let group_wins = read_chart_xml(fixture(Some(attribute), Some(r#"sourceLinked="0""#)).as_bytes()).unwrap();
            assert_eq!(group_wins.series[0].number_format.as_deref(), Some("0.0000"), "an explicit group label still precedes the source");
        }
        for attribute in [r#"sourceLinked="false""#, r#"sourceLinked="0""#] {
            let chart = read_chart_xml(fixture(Some(attribute), Some(r#"sourceLinked="0""#)).as_bytes()).unwrap();
            assert_eq!(chart.series[0].number_format.as_deref(), Some("0%"), "explicit series labels precede the group");
        }
        let absent = read_chart_xml(fixture(None, Some(r#"sourceLinked="0""#)).as_bytes()).unwrap();
        assert_eq!(absent.series[0].number_format.as_deref(), Some("0.0000"));
        let xml = fixture(Some(r#"sourceLinked="true""#), Some(r#"sourceLinked="true""#));
        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        for version in 0..=7 {
            assert_eq!(
                read_chart(&root, MAX_CHART_POINTS, version).unwrap().series[0].number_format.as_deref(),
                Some("0%"),
                "old{version} keeps its literal-one-only label behavior"
            );
        }
    }

    #[test]
    fn review_6096118299_resolved_secondary_survives_an_unowned_primary_anchor() {
        let scatter = r#"<c:scatterChart><c:ser><c:tx><c:v>Overlay</c:v></c:tx><c:xVal><c:numLit><c:formatCode>General</c:formatCode><c:pt idx="0"><c:v>30</c:v></c:pt></c:numLit></c:xVal><c:yVal><c:numLit><c:pt idx="0"><c:v>7</c:v></c:pt></c:numLit></c:yVal></c:ser><c:axId val="10"/><c:axId val="20"/></c:scatterChart>"#;
        let x = review_6095656137_axis("valAx", 10, 20, "b", 12, r#"<c:numFmt formatCode="0.0" sourceLinked="0"/>"#);
        let y = review_6095656137_axis("valAx", 20, 10, "r", 90, r#"<c:numFmt formatCode="0.000" sourceLinked="0"/>"#);
        // Both the reported duplicate-X anchor and an anchor sharing Y's ID are distinct
        // elements from the resolved group. Numeric equality cannot establish ownership.
        for anchor_id in [10, 20] {
            let category = review_6095656137_axis("catAx", 999, anchor_id, "b", 1, "");
            let anchor = review_6095656137_axis("valAx", anchor_id, 999, "l", 50, r#"<c:numFmt formatCode="0.00" sourceLinked="0"/>"#);
            for axes in [format!("{x}{y}{category}{anchor}"), format!("{category}{y}{x}{anchor}")] {
                let xml = metadata_chart(&format!("{scatter}{axes}"));
                let chart = read_chart_xml(xml.as_bytes()).unwrap();
                assert!(chart.series[0].secondary_axis, "resolved Y belongs to the secondary plot, not the unused primary anchor{anchor_id}");
                assert_eq!((chart.y_min, chart.y_max, chart.y2_min, chart.y2_max), (Some(2.0), Some(50.0), Some(2.0), Some(90.0)));
                assert_eq!((chart.y_format.as_deref(), chart.y2_format.as_deref()), (Some("0.00"), Some("0.000")));
                assert_eq!(chart.axis_titles.1, Some(format!("Axis{anchor_id}-999")), "keep the valid unused primary pair's existing selection");
                assert_eq!(chart.category_format.as_deref(), Some("0.0"));
                assert_eq!(chart.category_label(0), "30.0");
                let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
                for version in 0..=8 {
                    let mut previous = read_chart(&root, MAX_CHART_POINTS, version).unwrap();
                    if version == 8 {
                        assert!(!previous.series[0].secondary_axis, "saved8 retains its former ID fallback");
                        assert_eq!(previous.y2_max, None);
                    }
                    previous.raw = Some(xml.clone());
                    let previous: Chart = serde_json::from_str(&serde_json::to_string(&previous).unwrap()).unwrap();
                    let (written, kept) = crate::chart::write::chart_xml(&previous);
                    assert!(kept, "old{version} native projection must retain the original part");
                    assert_eq!(crate::xmlx::parse(&written).unwrap().to_xml(), root.to_xml());
                }
                // Ambiguous Y has no resolved owner: keep the old ID-only classification.
                let ambiguous = metadata_chart(&format!("{scatter}{y}{axes}"));
                let ambiguous = read_chart_xml(ambiguous.as_bytes()).unwrap();
                assert!(!ambiguous.series[0].secondary_axis);
                assert_eq!((ambiguous.y_max, ambiguous.y2_max), (Some(50.0), None));
                // When a populated group actually owns the anchor, exact element comparison
                // already distinguishes it from the overlay, even with a shared numeric ID.
                let primary = review_6095656137_group(999, anchor_id, "Primary", "0.00");
                let owned = read_chart_xml(metadata_chart(&format!("{primary}{scatter}{axes}")).as_bytes()).unwrap();
                assert_eq!(
                    owned.series.iter().map(|s| (s.name.as_str(), s.secondary_axis)).collect::<Vec<_>>(),
                    [("Primary", false), ("Overlay", true)]
                );
                assert_eq!((owned.y_max, owned.y2_max), (Some(50.0), Some(90.0)));
            }
        }
    }

    #[test]
    fn review_6096118299_linked_labels_use_literal_and_cached_numeric_formats() {
        let fixture = |literal: bool, series: Option<(&str, &str)>, group: Option<(&str, &str)>| {
            let labels = |setting: Option<(&str, &str)>| {
                setting.map(|(code, attr)| format!(r#"<c:dLbls><c:numFmt formatCode="{code}" {attr}/></c:dLbls>"#)).unwrap_or_default()
            };
            let data = r#"<c:formatCode>0%</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>0.5</c:v></c:pt>"#;
            let values = if literal {
                format!("<c:numLit>{data}</c:numLit>")
            } else {
                format!("<c:numRef><c:f>Sheet1!$B$2</c:f><c:numCache>{data}</c:numCache></c:numRef>")
            };
            metadata_chart(&format!(
                r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>
                <c:ser><c:idx val="0"/><c:order val="0"/>{}<c:val>{values}</c:val></c:ser>{}</c:barChart>"#,
                labels(series),
                labels(group)
            ))
        };
        for literal in [true, false] {
            for attr in [r#"sourceLinked="true""#, r#"sourceLinked="1""#, ""] {
                let xml = fixture(literal, Some(("0%", attr)), None);
                let chart = read_chart_xml(xml.as_bytes()).unwrap();
                assert_eq!(chart.series[0].number_format.as_deref(), Some("0%"), "literal={literal}, {attr}");
                assert_eq!(chart.data_label(&chart.series[0], 0.5), "50%");
                let group = fixture(literal, Some(("0%", attr)), Some(("0.0000", r#"sourceLinked="false""#)));
                assert_eq!(read_chart_xml(group.as_bytes()).unwrap().series[0].number_format.as_deref(), Some("0.0000"));
            }
            for attr in [r#"sourceLinked="false""#, r#"sourceLinked="0""#] {
                let xml = fixture(literal, Some(("0.0", attr)), Some(("0.0000", r#"sourceLinked="0""#)));
                assert_eq!(read_chart_xml(xml.as_bytes()).unwrap().series[0].number_format.as_deref(), Some("0.0"));
            }
            assert_eq!(
                read_chart_xml(fixture(literal, None, None).as_bytes()).unwrap().series[0].number_format.as_deref(),
                Some("0%"),
                "without a local label format the numeric source still supplies its format"
            );
        }
        // Published interpretation8 did not read literal source formats for linked labels.
        let xml = fixture(true, Some(("0%", r#"sourceLinked="true""#)), None);
        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        let mut previous = read_chart(&root, MAX_CHART_POINTS, 8).unwrap();
        assert_eq!(previous.series[0].number_format, None);
        previous.raw = Some(xml);
        let previous: Chart = serde_json::from_str(&serde_json::to_string(&previous).unwrap()).unwrap();
        let (written, kept) = crate::chart::write::chart_xml(&previous);
        assert!(kept, "unchanged interpretation8 native saves retain their raw chart");
        assert_eq!(crate::xmlx::parse(&written).unwrap().to_xml(), root.to_xml());
    }

    #[test]
    fn review_6096118299_linked_category_general_is_not_an_absent_source_format() {
        let fixture = |literal: bool, source_format: Option<&str>, attribute: &str| {
            let code = source_format.map(|f| format!("<c:formatCode>{f}</c:formatCode>")).unwrap_or_default();
            let points = format!(r#"{code}<c:ptCount val="1"/><c:pt idx="0"><c:v>45292</c:v></c:pt>"#);
            let categories = if literal {
                format!("<c:numLit>{points}</c:numLit>")
            } else {
                format!("<c:numRef><c:f>Sheet1!$A$2</c:f><c:numCache>{points}</c:numCache></c:numRef>")
            };
            let axis_format = format!(r#"<c:numFmt formatCode="mmm-yy" {attribute}/>"#);
            metadata_chart(&format!(
                r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:order val="0"/>
                <c:cat>{categories}</c:cat><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val>
                </c:ser><c:axId val="10"/><c:axId val="20"/></c:barChart>{}{}"#,
                review_6095656137_axis("catAx", 10, 20, "b", 1, &axis_format),
                review_6095656137_axis("valAx", 20, 10, "l", 2, "")
            ))
        };
        for literal in [true, false] {
            for attr in [r#"sourceLinked="true""#, r#"sourceLinked="1""#, ""] {
                let chart = read_chart_xml(fixture(literal, Some("General"), attr).as_bytes()).unwrap();
                assert_eq!(chart.category_format, None, "literal={literal}, {attr}: explicit source General wins before normalization");
                assert_eq!(chart.category_label(0), "45292", "a linked numeric source must not become a date");
                let absent = read_chart_xml(fixture(literal, None, attr).as_bytes()).unwrap();
                assert_eq!(absent.category_format.as_deref(), Some("mmm-yy"), "only an absent source format falls back to the stored axis code");
                assert_eq!(absent.category_label(0), "Jan-24");
                let numeric = read_chart_xml(fixture(literal, Some("0.00"), attr).as_bytes()).unwrap();
                assert_eq!(numeric.category_label(0), "45292.00");
            }
            for attr in [r#"sourceLinked="false""#, r#"sourceLinked="0""#] {
                let chart = read_chart_xml(fixture(literal, Some("General"), attr).as_bytes()).unwrap();
                assert_eq!(chart.category_format.as_deref(), Some("mmm-yy"), "explicit unlinked axis format still wins");
                assert_eq!(chart.category_label(0), "Jan-24");
                let xml = fixture(literal, Some("0.00"), attr).replace("formatCode=\"mmm-yy\"", "formatCode=\"General\"");
                let general = read_chart_xml(xml.as_bytes()).unwrap();
                assert_eq!(general.category_format.as_deref(), Some("General"), "explicit unlinked General retains its existing representation");
                assert_eq!(general.category_label(0), "45292");
            }
        }
        // Keep the published interpretation8 format choice and unchanged native raw XML.
        let xml = fixture(true, Some("General"), r#"sourceLinked="true""#);
        let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
        let mut previous = read_chart(&root, MAX_CHART_POINTS, 8).unwrap();
        assert_eq!(previous.category_format.as_deref(), Some("mmm-yy"));
        previous.raw = Some(xml);
        let previous: Chart = serde_json::from_str(&serde_json::to_string(&previous).unwrap()).unwrap();
        let (written, kept) = crate::chart::write::chart_xml(&previous);
        assert!(kept, "unchanged interpretation8 source-General saves retain their raw chart");
        assert_eq!(crate::xmlx::parse(&written).unwrap().to_xml(), root.to_xml());
    }

    #[test]
    fn review_6096226526_exact_secondary_owner_precedes_unresolved_source_groups() {
        let unresolved = review_6095656137_group(30, 40, "Unresolved", "0%");
        let scatter = r#"<c:scatterChart><c:ser><c:tx><c:v>Resolved</c:v></c:tx><c:xVal><c:numLit><c:formatCode>General</c:formatCode><c:pt idx="0"><c:v>30</c:v></c:pt></c:numLit></c:xVal><c:yVal><c:numLit><c:pt idx="0"><c:v>7</c:v></c:pt></c:numLit></c:yVal></c:ser><c:axId val="10"/><c:axId val="20"/></c:scatterChart>"#;
        let x = review_6095656137_axis("valAx", 10, 20, "b", 12, r#"<c:numFmt formatCode="0.0" sourceLinked="0"/>"#);
        let y = review_6095656137_axis("valAx", 20, 10, "r", 90, r#"<c:numFmt formatCode="0.000" sourceLinked="0"/>"#)
            .replace("</c:valAx>", r#"<c:majorUnit val="5"/></c:valAx>"#);
        let stray = review_6095656137_axis("valAx", 40, 888, "r", 70, r#"<c:numFmt formatCode="0%" sourceLinked="0"/>"#);
        let anchor = review_6095656137_axis("valAx", 10, 999, "l", 50, r#"<c:numFmt formatCode="0.00" sourceLinked="0"/>"#);
        let category = review_6095656137_axis("catAx", 999, 10, "b", 1, "");
        let axes = format!("{x}{y}{stray}{category}{anchor}");
        for (groups, old_y2) in [(format!("{unresolved}{scatter}"), 70.0), (format!("{scatter}{unresolved}"), 90.0)] {
            let xml = metadata_chart(&format!("{groups}{axes}"));
            let chart = read_chart_xml(xml.as_bytes()).unwrap();
            assert_eq!((chart.y_max, chart.y2_max), (Some(50.0), Some(90.0)), "the later exact owner must outrank an earlier unresolved group");
            assert!(chart.series.iter().all(|s| s.secondary_axis));
            assert_eq!(chart.y2_format.as_deref(), Some("0.000"));
            assert_eq!(chart.y2_major, Some(5.0));
            assert_eq!(chart.axis_titles.1.as_deref(), Some("Axis10-999"), "the unused primary anchor stays selected");
            let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
            let mut old9 = read_chart(&root, MAX_CHART_POINTS, 9).unwrap();
            assert_eq!(old9.y2_max, Some(old_y2), "saved9 keeps its source-order fallback when primary ownership is unresolved");
            old9.raw = Some(xml.clone());
            let old9: Chart = serde_json::from_str(&serde_json::to_string(&old9).unwrap()).unwrap();
            let (written, kept) = crate::chart::write::chart_xml(&old9);
            assert!(kept, "old9 native comparison retains the exact original part");
            assert_eq!(crate::xmlx::parse(&written).unwrap().to_xml(), root.to_xml());
        }
        // With no resolved secondary group, the existing ID-only fallback remains useful.
        let only_unresolved = read_chart_xml(metadata_chart(&format!("{unresolved}{axes}")).as_bytes()).unwrap();
        assert_eq!((only_unresolved.y_max, only_unresolved.y2_max), (Some(50.0), Some(70.0)));
        assert!(only_unresolved.series[0].secondary_axis);
        assert_eq!(only_unresolved.y2_format.as_deref(), Some("0%"));
        // A populated primary owner keeps the already-correct resolved-secondary preference.
        let primary = review_6095656137_group(999, 10, "Primary", "0.00");
        let owned = read_chart_xml(metadata_chart(&format!("{primary}{unresolved}{scatter}{axes}")).as_bytes()).unwrap();
        assert_eq!(
            owned.series.iter().map(|s| (s.name.as_str(), s.secondary_axis)).collect::<Vec<_>>(),
            [("Primary", false), ("Unresolved", true), ("Resolved", true)]
        );
        assert_eq!((owned.y_max, owned.y2_max, owned.y2_major), (Some(50.0), Some(90.0), Some(5.0)));
    }

    #[test]
    fn review_6096226526_linked_formats_do_not_borrow_another_resolved_axis() {
        let scatter = r#"<c:scatterChart><c:ser><c:tx><c:v>Overlay</c:v></c:tx><c:xVal><c:numLit><c:pt idx="0"><c:v>30</c:v></c:pt></c:numLit></c:xVal><c:yVal><c:numRef><c:f>Overlay!B1</c:f><c:numCache><c:formatCode>0%</c:formatCode><c:pt idx="0"><c:v>0.5</c:v></c:pt></c:numCache></c:numRef></c:yVal></c:ser><c:axId val="10"/><c:axId val="20"/></c:scatterChart>"#;
        let x = review_6095656137_axis("valAx", 10, 20, "b", 12, "");
        let y = review_6095656137_axis("valAx", 20, 10, "r", 90, "");
        let category = review_6095656137_axis("catAx", 999, 20, "b", 1, "");
        let anchor = review_6095656137_axis("valAx", 20, 999, "l", 50, r#"<c:numFmt formatCode="0.00" sourceLinked="true"/>"#);
        for axes in [format!("{x}{y}{category}{anchor}"), format!("{category}{y}{x}{anchor}")] {
            let xml = metadata_chart(&format!("{scatter}{axes}"));
            let current = read_chart_xml(xml.as_bytes()).unwrap();
            assert_eq!((current.y_format.as_deref(), current.y2_format.as_deref()), (Some("0.00"), Some("0%")));
            assert_eq!((current.axis_label(false, 0.5), current.axis_label(true, 0.5)), (String::from("0.50"), String::from("50%")));
            assert_eq!(current.series[0].values, [Some(0.5)]);
            // Unresolved groups still have the established numeric-ID fallback.
            let unresolved = review_6095656137_group(888, 20, "Unresolved", "0.000");
            let fallback = read_chart_xml(metadata_chart(&format!("{unresolved}{scatter}{axes}")).as_bytes()).unwrap();
            assert_eq!(fallback.y_format.as_deref(), Some("0.000"));
            assert_eq!(fallback.y2_format.as_deref(), Some("0%"));
            // A real owner takes priority over an earlier unresolved ID alias.
            let owner = review_6095656137_group(999, 20, "Owner", "0.0000");
            let owned = read_chart_xml(metadata_chart(&format!("{unresolved}{owner}{scatter}{axes}")).as_bytes()).unwrap();
            assert_eq!(owned.y_format.as_deref(), Some("0.0000"));
            let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
            let mut old = read_chart(&root, MAX_CHART_POINTS, 9).unwrap();
            assert_eq!(old.y_format.as_deref(), Some("0%"), "interpretation9 retains its former numeric-ID format fallback");
            old.raw = Some(xml);
            let old: Chart = serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
            let (written, kept) = crate::chart::write::chart_xml(&old);
            assert!(kept, "unchanged interpretation9 native chart keeps its original XML");
            assert_eq!(crate::xmlx::parse(&written).unwrap().to_xml(), root.to_xml());
        }
    }

    #[test]
    fn review_6096226526_linked_value_axes_use_literal_and_cached_source_formats() {
        for scatter in [false, true] {
            for literal in [false, true] {
                let points = r#"<c:formatCode>0%</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>0.5</c:v></c:pt>"#;
                let data = if literal {
                    format!("<c:numLit>{points}</c:numLit>")
                } else {
                    format!("<c:numRef><c:f>Sheet1!$B$2</c:f><c:numCache>{points}</c:numCache></c:numRef>")
                };
                let group = if scatter {
                    format!(
                        r#"<c:scatterChart><c:ser><c:xVal><c:numLit><c:pt idx="0"><c:v>30</c:v></c:pt></c:numLit></c:xVal><c:yVal>{data}</c:yVal></c:ser><c:axId val="10"/><c:axId val="20"/></c:scatterChart>"#
                    )
                } else {
                    format!(
                        r#"<c:barChart><c:barDir val="col"/><c:ser><c:val>{data}</c:val></c:ser><c:axId val="10"/><c:axId val="20"/></c:barChart>"#
                    )
                };
                let category = review_6095656137_axis(if scatter { "valAx" } else { "catAx" }, 10, 20, "b", 30, "");
                for attr in [r#"sourceLinked="true""#, r#"sourceLinked="1""#, ""] {
                    let fmt = format!(r#"<c:numFmt formatCode="0.000" {attr}/>"#);
                    let y = review_6095656137_axis("valAx", 20, 10, "l", 1, &fmt);
                    let xml = metadata_chart(&format!("{group}{category}{y}"));
                    let chart = read_chart_xml(xml.as_bytes()).unwrap();
                    assert_eq!(chart.y_format.as_deref(), Some("0%"), "scatter={scatter}, literal={literal}, {attr}");
                    assert_eq!(chart.axis_label(false, 0.5), "50%");
                    assert_eq!(chart.series[0].values, [Some(0.5)]);
                    let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
                    let mut old = read_chart(&root, MAX_CHART_POINTS, 9).unwrap();
                    assert_eq!(old.y_format.as_deref(), Some(if literal { "0.000" } else { "0%" }));
                    old.raw = Some(xml);
                    let old: Chart = serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
                    let (written, kept) = crate::chart::write::chart_xml(&old);
                    assert!(kept);
                    assert_eq!(crate::xmlx::parse(&written).unwrap().to_xml(), root.to_xml());
                }
                for attr in [r#"sourceLinked="false""#, r#"sourceLinked="0""#] {
                    let fmt = format!(r#"<c:numFmt formatCode="0.000" {attr}/>"#);
                    let y = review_6095656137_axis("valAx", 20, 10, "l", 1, &fmt);
                    let chart = read_chart_xml(metadata_chart(&format!("{group}{category}{y}")).as_bytes()).unwrap();
                    assert_eq!(chart.y_format.as_deref(), Some("0.000"));
                    assert_eq!(chart.axis_label(false, 0.5), "0.500");
                }
            }
        }
    }

    #[test]
    fn review_6096226526_source_general_overrides_stored_value_axis_format() {
        for literal in [false, true] {
            for percent in [false, true] {
                for source in [Some("General"), Some("0%"), Some(""), None] {
                    let code = source.map(|s| format!("<c:formatCode>{s}</c:formatCode>")).unwrap_or_default();
                    let points = format!(r#"{code}<c:pt idx="0"><c:v>0.5</c:v></c:pt>"#);
                    let data = if literal {
                        format!("<c:numLit>{points}</c:numLit>")
                    } else {
                        format!("<c:numRef><c:f>Sheet1!$B$2</c:f><c:numCache>{points}</c:numCache></c:numRef>")
                    };
                    let grouping = if percent { "percentStacked" } else { "clustered" };
                    let group = format!(
                        r#"<c:barChart><c:barDir val="col"/><c:grouping val="{grouping}"/><c:ser><c:val>{data}</c:val></c:ser><c:axId val="10"/><c:axId val="20"/></c:barChart>"#
                    );
                    let cat = review_6095656137_axis("catAx", 10, 20, "b", 1, "");
                    for attr in [r#"sourceLinked="true""#, r#"sourceLinked="1""#, "", r#"sourceLinked="false""#, r#"sourceLinked="0""#] {
                        let fmt = format!(r#"<c:numFmt formatCode="0.000" {attr}/>"#);
                        let y = review_6095656137_axis("valAx", 20, 10, "l", 1, &fmt);
                        let xml = metadata_chart(&format!("{group}{cat}{y}"));
                        let chart = read_chart_xml(xml.as_bytes()).unwrap();
                        let linked = !matches!(attr, r#"sourceLinked="false""# | r#"sourceLinked="0""#);
                        let expected = if linked { source.filter(|s| !s.is_empty()).unwrap_or("0.000") } else { "0.000" };
                        assert_eq!(chart.y_format.as_deref(), Some(expected), "literal={literal}, percent={percent}, {source:?}, {attr}");
                        assert_eq!(
                            chart.axis_label(false, 0.5),
                            match expected {
                                "General" => "0.5",
                                "0%" => "50%",
                                _ => "0.500",
                            }
                        );
                        if source == Some("General") && linked {
                            let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
                            let mut old = read_chart(&root, MAX_CHART_POINTS, 9).unwrap();
                            assert_eq!(old.y_format.as_deref(), Some("0.000"));
                            old.raw = Some(xml);
                            let old: Chart = serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
                            let (written, kept) = crate::chart::write::chart_xml(&old);
                            assert!(kept);
                            assert_eq!(crate::xmlx::parse(&written).unwrap().to_xml(), root.to_xml());
                        }
                    }
                    // No local numFmt is also source-linked; General must not activate
                    // the percent-axis default merely because it normalizes like no format.
                    let y = review_6095656137_axis("valAx", 20, 10, "l", 1, "");
                    let chart = read_chart_xml(metadata_chart(&format!("{group}{cat}{y}")).as_bytes()).unwrap();
                    let expected = source.filter(|s| !s.is_empty());
                    assert_eq!(chart.y_format.as_deref(), expected);
                    assert_eq!(chart.axis_label(false, 0.5), if expected == Some("0%") || (expected.is_none() && percent) { "50%" } else { "0.5" });
                }
            }
        }
    }

    #[test]
    fn review_6096226526_empty_axis_source_format_does_not_hide_a_later_usable_cache() {
        for first_format in [Some(""), None, Some("General")] {
            let first_code = first_format.map(|code| format!("<c:formatCode>{code}</c:formatCode>")).unwrap_or_default();
            let first = format!(
                r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>First</c:v></c:tx><c:val><c:numRef><c:f>Sheet1!$B$2</c:f><c:numCache>{first_code}<c:pt idx="0"><c:v>0.25</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#
            );
            let second = r#"<c:ser><c:idx val="1"/><c:order val="1"/><c:tx><c:v>Second</c:v></c:tx><c:val><c:numRef><c:f>Sheet1!$C$2</c:f><c:numCache><c:formatCode>0%</c:formatCode><c:pt idx="0"><c:v>0.5</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#;
            let group = format!(r#"<c:barChart><c:barDir val="col"/>{first}{second}<c:axId val="10"/><c:axId val="20"/></c:barChart>"#);
            let category = review_6095656137_axis("catAx", 10, 20, "b", 1, "");
            let value = review_6095656137_axis("valAx", 20, 10, "l", 1, r#"<c:numFmt formatCode="0.000" sourceLinked="true"/>"#);
            let xml = metadata_chart(&format!("{group}{category}{value}"));
            let chart = read_chart_xml(xml.as_bytes()).unwrap();
            let expected = if first_format == Some("General") { "General" } else { "0%" };
            assert_eq!(chart.y_format.as_deref(), Some(expected), "first source format {first_format:?}");
            assert_eq!(chart.axis_label(false, 0.5), if expected == "General" { "0.5" } else { "50%" });
            assert_eq!(chart.series.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
            assert_eq!(chart.series.iter().map(|s| s.values.clone()).collect::<Vec<_>>(), [vec![Some(0.25)], vec![Some(0.5)]]);
            assert!(chart.series.iter().all(|s| !s.secondary_axis), "both caches supply the same exact primary axis");
            let explicit = xml.replace(r#"sourceLinked="true""#, r#"sourceLinked="false""#);
            assert_eq!(read_chart_xml(explicit.as_bytes()).unwrap().y_format.as_deref(), Some("0.000"));
            // Missing formatCode already continued historically; an empty element and
            // explicit General both triggered interpretation9's stored-format fallback.
            let root = crate::xmlx::parse(xml.as_bytes()).unwrap();
            let mut old = read_chart(&root, MAX_CHART_POINTS, 9).unwrap();
            assert_eq!(old.y_format.as_deref(), Some(if first_format.is_none() { "0%" } else { "0.000" }));
            old.raw = Some(xml);
            let old: Chart = serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
            let (written, kept) = crate::chart::write::chart_xml(&old);
            assert!(kept, "interpretation9's existing typed choice still retains its original native part");
            assert_eq!(crate::xmlx::parse(&written).unwrap().to_xml(), root.to_xml());
        }
    }
}
