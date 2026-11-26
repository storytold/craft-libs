//! Charts drawn as Word draws them: text in its stated colour, weight and size. Synthetic chart
//! parts written here; ported from storytold/wordcraft#4507.

use super::*;
use craft_wordmodel::fill::Fill;

/// The texts a chart draws, with their rectangles and turn (degrees clockwise).
pub(super) fn chart_texts(items: &[GraphicItem]) -> Vec<(String, [f32; 4], f32)> {
    items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Text { text, rect, angle, .. } => Some((text.clone(), *rect, *angle)),
            _ => None,
        })
        .collect()
}

pub(super) fn whole_chart(chart: &str) -> El {
    let x = format!(r#"<c:chartSpace {NS}><c:chart>{chart}</c:chart></c:chartSpace>"#);
    crate::xml::parse(x.as_bytes()).unwrap()
}

/// A chart's title, legend and axis labels take the colour their own text properties state
/// (a dark chart style states light text); text that states none takes the chart's `c:txPr`
/// colour, and the default grey without one. All of it used to be the default grey, which
/// left the text of a dark chart unreadable.
#[test]
fn chart_text_takes_the_colour_its_text_properties_state() {
    let fill = |c: &str| format!(r#"<a:solidFill><a:srgbClr val="{c}"/></a:solidFill>"#);
    let tx_pr = |c: &str| format!(r#"<c:txPr><a:bodyPr/><a:p><a:pPr><a:defRPr>{}</a:defRPr></a:pPr></a:p></c:txPr>"#, fill(c));
    let chart = |own: bool, chart_default: Option<&str>| {
        let rpr = if own { format!("<a:rPr>{}</a:rPr>", fill("FFFFFF")) } else { "<a:rPr/>".to_string() };
        let title = format!(r#"<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r>{rpr}<a:t>Revenue</a:t></a:r></a:p></c:rich></c:tx></c:title>"#);
        let (cat, val, legend) = if own { (tx_pr("FF0000"), tx_pr("0000FF"), tx_pr("00FF00")) } else { Default::default() };
        let plot = format!(
            r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/>{cat}</c:catAx><c:valAx><c:axId val="2"/>{val}</c:valAx></c:plotArea>"#,
            ser(0, "North", None, &["Q1", "Q2"], &["4", "8"])
        );
        let x = format!(
            r#"<c:chartSpace {NS}><c:chart>{title}{plot}<c:legend><c:legendPos val="b"/>{legend}</c:legend></c:chart>{}</c:chartSpace>"#,
            chart_default.map(tx_pr).unwrap_or_default()
        );
        chart_items(&crate::xml::parse(x.as_bytes()).unwrap(), &theme(), 300.0, 200.0)
    };
    let colour = |items: &[GraphicItem], s: &str| {
        items.iter().find_map(|it| match it {
            GraphicItem::Text { text, color, .. } if text == s => Some(*color),
            _ => None,
        })
    };
    let own = chart(true, Some("123456"));
    assert_eq!(colour(&own, "Revenue"), Some(Rgb(0xFF, 0xFF, 0xFF)), "title");
    assert_eq!(colour(&own, "Q1"), Some(Rgb(0xFF, 0x00, 0x00)), "category axis");
    assert_eq!(colour(&own, "8"), Some(Rgb(0x00, 0x00, 0xFF)), "value axis");
    assert_eq!(colour(&own, "North"), Some(Rgb(0x00, 0xFF, 0x00)), "legend");
    let inherited = chart(false, Some("123456"));
    for s in ["Revenue", "Q1", "8", "North"] {
        assert_eq!(colour(&inherited, s), Some(Rgb(0x12, 0x34, 0x56)), "{s} takes the chart's colour");
    }
    let plain = chart(false, None);
    for s in ["Revenue", "Q1", "8", "North"] {
        assert_eq!(colour(&plain, s), Some(TEXT), "{s} keeps the default grey");
    }
}

/// Bold comes from the text properties: a title that turns bold off is plain, axis labels whose
/// `c:txPr` says bold are bold, and labels that say nothing are plain.
#[test]
fn chart_text_is_bold_as_its_text_properties_say() {
    let bold_tx = r#"<c:txPr><a:bodyPr/><a:p><a:pPr><a:defRPr b="1"/></a:pPr></a:p></c:txPr>"#;
    let title =
        r#"<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:pPr><a:defRPr b="0"/></a:pPr><a:r><a:t>Plain title</a:t></a:r></a:p></c:rich></c:tx></c:title>"#;
    let plot = format!(
        r#"{title}<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/>{bold_tx}</c:valAx></c:plotArea>"#,
        ser(0, "S", None, &["A", "B"], &["4", "8"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let bold = |s: &str| {
        items.iter().find_map(|it| match it {
            GraphicItem::Text { text, bold, .. } if text == s => Some(*bold),
            _ => None,
        })
    };
    assert_eq!(bold("Plain title"), Some(false));
    assert_eq!(bold("8"), Some(true), "value axis labels");
    assert_eq!(bold("A"), Some(false), "category axis labels");
}

/// The title's size comes from its text properties (`sz`): a stated size is larger or smaller
/// than the default as it says.
#[test]
fn the_title_takes_its_stated_size() {
    let title = |rpr: &str| {
        let plot = format!(
            r#"<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:pPr><a:defRPr {rpr}/></a:pPr><a:r><a:t>Sized</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:barDir val="col"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#,
            ser(0, "S", None, &["A"], &["1"]),
        );
        chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0).iter().find_map(|it| match it {
            GraphicItem::Text { text, size, .. } if text == "Sized" => Some(*size),
            _ => None,
        })
    };
    let (default, big, small) = (title(""), title(r#"sz="2400""#), title(r#"sz="1000""#));
    assert!(big > default && small < default, "{small:?} < {default:?} < {big:?}");
}

/// A combination chart: columns and a line in one plot area, on the same axes (Word's "Clustered
/// Column - Line"). Both are drawn, and the value axis takes in the line's values too. Only the
/// first chart type used to be drawn.
#[test]
fn a_combination_chart_draws_its_columns_and_its_line_on_the_shared_axes() {
    let cats = ["A", "B", "C"];
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:lineChart><c:grouping val="standard"/>{}<c:axId val="1"/><c:axId val="2"/></c:lineChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#,
        ser(0, "Costs", Some("FF0000"), &cats, &["1", "2", "3"]),
        ser(1, "Benefits", Some("00FF00"), &cats, &["2", "2", "2"]),
        ser(2, "Total", Some("0000FF"), &cats, &["3", "7", "12"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    assert_eq!(filled(&items).len(), 6, "two column series of three");
    let blue_line = items.iter().any(|it| matches!(it, GraphicItem::Path { fill: None, stroke: Some(c), .. } if *c == Rgb(0, 0, 0xFF)));
    assert!(blue_line, "the line series is drawn");
    let ticks: Vec<f64> = chart_texts(&items).iter().filter_map(|(t, _, _)| t.parse().ok()).collect();
    assert!(ticks.iter().any(|t| *t >= 12.0), "the value axis reaches the line's 12: {ticks:?}");
}

/// A combination's chart types share one cell budget: once the first type has spent it, the
/// next one's series are not parsed at all.
#[test]
fn combination_chart_types_share_one_cell_budget() {
    let sers: String = (0..20)
        .map(|i| format!(r#"<c:ser><c:idx val="{i}"/><c:val><c:numRef><c:numCache><c:ptCount val="10000"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#))
        .collect();
    let ty = |name: &str| crate::xml::parse(format!(r#"<c:{name} {NS}>{sers}</c:{name}>"#).as_bytes()).unwrap();
    let mut budget = MAX_CELLS;
    let bars = read_series(&ty("barChart"), false, &theme(), &mut budget);
    assert!(bars.len() < 20, "the first type stops at the budget: {}", bars.len());
    assert_eq!(budget, 0);
    assert!(read_series(&ty("lineChart"), false, &theme(), &mut budget).is_empty(), "nothing is left for the line");
}

/// A 100% stacked column chart combined with a line: only the columns become shares of their
/// category's total; the line keeps its values and doesn't count in the totals.
#[test]
fn only_the_stacked_chart_type_becomes_percentages() {
    let cats = ["A", "B"];
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="percentStacked"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:lineChart><c:grouping val="standard"/>{}<c:axId val="1"/><c:axId val="2"/></c:lineChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#,
        ser(0, "Left", None, &cats, &["1", "3"]),
        ser(1, "Right", None, &cats, &["3", "1"]),
        ser(2, "Trend", None, &cats, &["10", "20"]),
    );
    let chart = Chart::parse(&whole_chart(&plot), &theme()).expect("a chart");
    let values = |j: usize| chart.series[j].values.clone();
    assert_eq!(values(0), vec![Some(0.25), Some(0.75)], "column shares of the columns' totals");
    assert_eq!(values(1), vec![Some(0.75), Some(0.25)]);
    assert_eq!(values(2), vec![Some(10.0), Some(20.0)], "the line keeps its values");
}

/// A 100% stacked column chart combined with a line on the same axes: the line keeps its values,
/// so the value axis takes them in rather than stopping at 100%, and the line is drawn at them,
/// not pinned to the top of the plot.
#[test]
fn a_percent_stacked_combination_scales_its_axis_to_the_line() {
    let cats = ["A", "B"];
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="percentStacked"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:lineChart><c:grouping val="standard"/>{}<c:axId val="1"/><c:axId val="2"/></c:lineChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#,
        ser(0, "Left", Some("FF0000"), &cats, &["1", "3"]),
        ser(1, "Right", Some("00FF00"), &cats, &["3", "1"]),
        ser(2, "Trend", Some("0000FF"), &cats, &["10", "20"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let line_ys: Vec<f32> = items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Path { segs, fill: None, stroke: Some(c), .. } if *c == Rgb(0, 0, 0xFF) => Some(segs),
            _ => None,
        })
        .flat_map(|segs| {
            segs.iter().filter_map(|s| match s {
                PathSeg::Move(_, y) | PathSeg::Line(_, y) => Some(*y),
                _ => None,
            })
        })
        .collect();
    assert_eq!(line_ys.len(), 2, "two line points: {line_ys:?}");
    assert!((line_ys[0] - line_ys[1]).abs() > 10.0, "10 and 20 at different heights, not both at the top: {line_ys:?}");
    let ticks: Vec<String> = chart_texts(&items).into_iter().map(|(t, _, _)| t).filter(|t| t.ends_with('%')).collect();
    assert!(
        ticks.iter().any(|t| t.trim_end_matches('%').parse::<f64>().is_ok_and(|v| v >= 2000.0)),
        "the axis reaches the line's 20 (2000%): {ticks:?}"
    );
}

/// A line chart that shows markers (`c:marker val="1"` of the chart type) draws one per point in
/// the series' marker colour, the automatic marker where the series names no symbol; a series
/// whose symbol is `none` draws none.
#[test]
fn line_markers_are_drawn_unless_the_series_turns_them_off() {
    let marked = |symbol: &str| {
        let ser = format!(
            r#"<c:ser><c:idx val="1"/><c:order val="0"/><c:spPr><a:ln><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></a:ln></c:spPr><c:marker>{symbol}<c:size val="5"/><c:spPr><a:solidFill><a:srgbClr val="8EA9DB"/></a:solidFill></c:spPr></c:marker><c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt><c:pt idx="2"><c:v>C</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>3</c:v></c:pt><c:pt idx="2"><c:v>2</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#
        );
        let plot = format!(
            r#"<c:plotArea><c:lineChart><c:grouping val="standard"/>{ser}<c:marker val="1"/><c:axId val="1"/><c:axId val="2"/></c:lineChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#
        );
        let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
        items
            .iter()
            .filter(
                |it| matches!(it, GraphicItem::Path { fill: Some(c), .. } | GraphicItem::Shape { fill: Some(c), .. } if *c == Rgb(0x8E, 0xA9, 0xDB)),
            )
            .count()
    };
    assert_eq!(marked(""), 3, "an automatic marker on each point");
    assert_eq!(marked(r#"<c:symbol val="square"/>"#), 3);
    assert_eq!(marked(r#"<c:symbol val="none"/>"#), 0);
}

/// `c:symbol val="auto"` asks for the series' automatic marker, as a series naming no symbol in
/// a chart that shows markers gets: the series with index 0 a diamond, index 2 a triangle.
#[test]
fn an_auto_marker_symbol_takes_the_series_automatic_marker() {
    let symbol = |idx: usize, symbol: &str| {
        let x = format!(
            r#"<c:lineChart {NS}><c:ser><c:idx val="{idx}"/><c:order val="{idx}"/><c:marker>{symbol}</c:marker><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:marker val="1"/></c:lineChart>"#
        );
        let ty = crate::xml::parse(x.as_bytes()).unwrap();
        read_series(&ty, false, &theme(), &mut { MAX_CELLS }).first().map(|s| (s.markers, s.symbol))
    };
    assert_eq!(symbol(0, r#"<c:symbol val="auto"/>"#), Some((Some(true), model::Symbol::Diamond)));
    assert_eq!(symbol(2, r#"<c:symbol val="auto"/>"#), Some((Some(true), model::Symbol::Triangle)));
    assert_eq!(symbol(2, ""), Some((Some(true), model::Symbol::Triangle)));
    assert_eq!(symbol(2, r#"<c:symbol val="square"/>"#), Some((Some(true), model::Symbol::Square)));
}

/// Display units (`c:dispUnits`): the value axis labels count thousands, and the unit's name
/// stands turned beside the axis, in its own weight.
#[test]
fn display_units_divide_the_axis_labels_and_name_the_unit() {
    let units = |label: &str| {
        let plot = format!(
            r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/><c:dispUnits><c:builtInUnit val="thousands"/>{label}</c:dispUnits></c:valAx></c:plotArea>"#,
            ser(0, "S", None, &["A", "B"], &["4000", "8000"]),
        );
        chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0)
    };
    let items = units("<c:dispUnitsLbl/>");
    let texts = chart_texts(&items);
    let ticks: Vec<&str> = texts.iter().map(|(t, _, _)| t.as_str()).collect();
    assert!(ticks.contains(&"8") && !ticks.contains(&"8000"), "labels in thousands: {ticks:?}");
    let (_, rect, angle) = texts.iter().find(|(t, _, _)| t == "Thousands").expect("the unit's name");
    assert!(*angle != 0.0, "the unit's name is turned");
    let label_left = texts.iter().filter(|(t, _, _)| t == "8").map(|(_, r, _)| r[0]).next().unwrap();
    assert!(rect[0] + rect[2] <= label_left + 0.01, "left of the value labels, apart from them: {rect:?} {label_left}");
    let bold = |items: &[GraphicItem]| {
        items.iter().find_map(|it| match it {
            GraphicItem::Text { text, bold, .. } if text == "Thousands" => Some(*bold),
            _ => None,
        })
    };
    assert_eq!(bold(&items), Some(false));
    let bold_label = units(r#"<c:dispUnitsLbl><c:txPr><a:bodyPr/><a:p><a:pPr><a:defRPr b="1"/></a:pPr></a:p></c:txPr></c:dispUnitsLbl>"#);
    assert_eq!(bold(&bold_label), Some(true), "a bold display-units label");
    // Without a label, the values are still counted in thousands.
    let quiet = chart_texts(&units(""));
    assert!(quiet.iter().any(|(t, _, _)| t == "8") && !quiet.iter().any(|(t, _, _)| t == "Thousands"), "{quiet:?}");
}

/// A custom display unit from a hostile file (tiny, huge) draws finite labels and never panics.
#[test]
fn hostile_display_units_never_panic() {
    for unit in ["1e-300", "1e300", "0", "-5", "NaN", "inf", "x"] {
        let plot = format!(
            r#"<c:plotArea><c:barChart><c:barDir val="col"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/><c:dispUnits><c:custUnit val="{unit}"/><c:dispUnitsLbl/></c:dispUnits></c:valAx></c:plotArea>"#,
            ser(0, "S", None, &["A", "B"], &["1e308", "-1e308"]),
        );
        let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
        assert!(!items.is_empty(), "{unit}");
    }
}

/// A data table (`c:dTable`) under the plot: the categories head its columns, and each series
/// has a row of its values in its number format. The categories aren't drawn twice.
#[test]
fn a_data_table_lists_each_series_values_under_the_plot() {
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx><c:dTable><c:showHorzBorder val="1"/><c:showVertBorder val="1"/><c:showOutline val="1"/><c:showKeys val="1"/></c:dTable></c:plotArea>"#,
        ser(0, "North", None, &["Q1", "Q2"], &["41", "-13"]),
        ser(1, "South", None, &["Q1", "Q2"], &["27", "33"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let texts = chart_texts(&items);
    let count = |s: &str| texts.iter().filter(|(t, _, _)| t == s).count();
    // Values that are no axis tick, so each is drawn once.
    for s in ["North", "South", "41", "-13", "27", "33"] {
        assert_eq!(count(s), 1, "{s} in the table: {texts:?}");
    }
    assert_eq!(count("Q1"), 1, "the categories head the table, once");
    let y = |s: &str| texts.iter().find(|(t, _, _)| t == s).map(|(_, r, _)| r[1]).unwrap();
    assert!(y("Q1") < y("41") && y("41") < y("27"), "header row, then a row per series");
}

/// A data table's name column stays inside the margin reserved for it: a series name wider
/// than that is cut short instead of running off the chart.
#[test]
fn a_long_data_table_name_stays_inside_the_chart() {
    let long = "A very long series name that is much wider than the chart allows for its table";
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx><c:dTable><c:showKeys val="1"/></c:dTable></c:plotArea>"#,
        ser(0, long, None, &["Q1", "Q2"], &["41", "27"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let (name, rect, _) = chart_texts(&items).into_iter().find(|(t, _, _)| t.starts_with("A very")).expect("the series name");
    assert!(rect[0] >= 0.0, "the name starts inside the chart: {rect:?}");
    assert!(name.ends_with('…'), "the name is cut short: {name}");
}

/// A legend placed by hand (`c:manualLayout`, edge mode) is drawn where the file puts it, over
/// the chart, rather than in its automatic place.
#[test]
fn a_legend_placed_by_hand_is_drawn_where_the_file_puts_it() {
    let legend = r#"<c:legend><c:legendPos val="b"/><c:layout><c:manualLayout><c:xMode val="edge"/><c:yMode val="edge"/><c:x val="0.1"/><c:y val="0.1"/><c:w val="0.4"/><c:h val="0.1"/></c:manualLayout></c:layout></c:legend>"#;
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>{legend}"#,
        ser(0, "Sales", None, &["A", "B"], &["1", "2"]),
    );
    let texts = chart_texts(&chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0));
    let (_, r, _) = texts.iter().find(|(t, _, _)| t == "Sales").expect("legend entry");
    assert!(r[1] >= 18.0 && r[1] + r[3] <= 42.0, "inside the box 20..40 pt down: {r:?}");
    assert!(r[0] >= 30.0 && r[0] + r[2] <= 150.0, "inside the box 30..150 pt across: {r:?}");
}

/// A column chart with negative values: the category axis crosses the value axis at zero
/// (`c:crosses` autoZero, the default), so its line is drawn there, in the colour and width
/// its `c:spPr` states; its labels sit next to it unless `c:tickLblPos` says `low`.
#[test]
fn the_category_axis_crosses_at_zero_in_its_own_line() {
    let chart = |tick: &str| {
        let plot = format!(
            r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/>{tick}<c:spPr><a:ln w="19050"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln></c:spPr></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#,
            ser(0, "S", Some("FF0000"), &["A", "B"], &["-2", "4"]),
        );
        chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0)
    };
    let items = chart("");
    let texts = chart_texts(&items);
    let mid = |s: &str| texts.iter().find(|(t, _, _)| t == s).map(|(_, r, _)| r[1] + r[3] / 2.0).unwrap();
    let zero = mid("0");
    let axis: Vec<(f32, f32)> = items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Path { segs, stroke: Some(c), stroke_width, .. } if *c == Rgb::BLACK => match segs.first() {
                Some(PathSeg::Move(_, y)) => Some((*y, *stroke_width)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(axis.len(), 1, "one black axis line: {axis:?}");
    assert!((axis[0].0 - zero).abs() < 0.5, "the axis at zero ({zero}), not the bottom: {axis:?}");
    assert!(axis[0].1 > 0.75, "its own, wider line: {axis:?}");
    let label_top = |items: &[GraphicItem]| chart_texts(items).iter().find(|(t, _, _)| t == "A").map(|(_, r, _)| r[1]).unwrap();
    assert!((label_top(&items) - zero).abs() < 10.0, "labels next to the axis");
    let low = chart(r#"<c:tickLblPos val="low"/>"#);
    assert!(label_top(&low) > zero + 40.0, "low labels under the plot");
}

/// The chart area's gradient (`a:gradFill`) and the plot area's own fill are painted, the
/// plot's over the chart's and inside it.
#[test]
fn chart_and_plot_area_fills_are_painted() {
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx><c:spPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></c:spPr></c:plotArea>"#,
        ser(0, "S", Some("FF0000"), &["A", "B"], &["1", "2"]),
    );
    let x = format!(
        r#"<c:chartSpace {NS}><c:chart>{plot}</c:chart><c:spPr><a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="DDDDDD"/></a:gs><a:gs pos="100000"><a:srgbClr val="999999"/></a:gs></a:gsLst><a:lin ang="5400000" scaled="1"/></a:gradFill></c:spPr></c:chartSpace>"#
    );
    let items = chart_items(&crate::xml::parse(x.as_bytes()).unwrap(), &theme(), 300.0, 200.0);
    let fills: Vec<(&[f32; 4], &Fill)> = items
        .iter()
        .filter_map(|it| match it {
            GraphicItem::Fill { rect, fill } => Some((rect, fill)),
            _ => None,
        })
        .collect();
    assert_eq!(fills.len(), 2, "the chart's and the plot's: {fills:?}");
    assert!(matches!(fills[0].1, Fill::Gradient(_)), "the chart area's gradient first");
    assert_eq!(fills[0].0, &[0.0, 0.0, 300.0, 200.0]);
    assert!(matches!(fills[1].1, Fill::Solid { color: Rgb::WHITE, .. }), "then the white plot");
    assert!(fills[1].0[2] < 300.0 && fills[1].0[3] < 200.0, "inside the chart");
}

/// A series filled with a gradient (Word's chart styles shade their columns) paints each
/// column with that gradient rather than the gradient's first colour.
#[test]
fn a_gradient_series_fills_its_columns_with_the_gradient() {
    let grad = r#"<c:spPr><a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="FFCC99"/></a:gs><a:gs pos="100000"><a:srgbClr val="CC6600"/></a:gs></a:gsLst><a:lin ang="5400000" scaled="0"/></a:gradFill></c:spPr>"#;
    let ser = ser(0, "S", None, &["A", "B"], &["1", "2"]).replacen("<c:cat>", &format!("{grad}<c:cat>"), 1);
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{ser}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let shaded = items.iter().filter(|it| matches!(it, GraphicItem::Fill { fill: Fill::Gradient(_), .. })).count();
    assert_eq!(shaded, 2, "both columns shaded");
    assert!(filled(&items).is_empty(), "no flat column under them");
}

/// A legend placed by hand is drawn over the plot, after its fill and series: drawn first, an
/// opaque plot area or a column hid it.
#[test]
fn a_hand_placed_legend_is_drawn_over_the_plot() {
    let legend = r#"<c:legend><c:legendPos val="b"/><c:layout><c:manualLayout><c:xMode val="edge"/><c:yMode val="edge"/><c:x val="0.3"/><c:y val="0.4"/><c:w val="0.4"/><c:h val="0.1"/></c:manualLayout></c:layout></c:legend>"#;
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx><c:spPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></c:spPr></c:plotArea>{legend}"#,
        ser(0, "Sales", Some("FF0000"), &["A", "B"], &["1", "2"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let at = |f: &dyn Fn(&GraphicItem) -> bool| items.iter().rposition(f);
    let entry = items.iter().position(|it| matches!(it, GraphicItem::Text { text, .. } if text == "Sales")).expect("the legend entry");
    let plot_fill = at(&|it| matches!(it, GraphicItem::Fill { .. })).expect("the plot's fill");
    let column = at(&|it| matches!(it, GraphicItem::Path { fill: Some(c), .. } if *c == Rgb(0xFF, 0, 0))).expect("a column");
    assert!(entry > plot_fill && entry > column, "the legend after the plot: {entry} vs {plot_fill}, {column}");
}

/// A combination whose first chart type is a line still has columns, and columns stand on zero:
/// the value axis keeps zero for a narrow range (1000..1050) instead of cutting the columns off.
/// A line chart alone keeps its own narrow scale.
#[test]
fn a_line_first_combination_with_columns_keeps_zero_on_its_axis() {
    let cats = ["A", "B"];
    let line = |s: &str| format!(r#"<c:lineChart><c:grouping val="standard"/>{s}<c:axId val="1"/><c:axId val="2"/></c:lineChart>"#);
    let bars = format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart>"#,
        ser(1, "Columns", Some("FF0000"), &cats, &["1000", "1050"])
    );
    let ticks = |types: &str| {
        let plot = format!(r#"<c:plotArea>{types}<c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#);
        chart_texts(&chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0))
            .into_iter()
            .filter_map(|(t, _, _)| t.parse::<f64>().ok())
            .collect::<Vec<_>>()
    };
    let lone = line(&ser(0, "Line", Some("0000FF"), &cats, &["1000", "1050"]));
    assert!(ticks(&lone).iter().all(|t| *t > 0.0), "a line alone keeps its narrow scale: {:?}", ticks(&lone));
    let both = ticks(&format!("{lone}{bars}"));
    assert!(both.contains(&0.0), "the columns' zero is on the axis: {both:?}");
}

/// A data table under a chart with many series keeps the plot: the table takes at most half the
/// chart's height, its header and the series rows that fit. It used to take all of it, and the
/// chart drew nothing.
#[test]
fn a_data_table_with_many_series_leaves_the_plot_its_room() {
    let cats = ["Q1", "Q2"];
    let sers: String = (0..15).map(|i| ser(i, &format!("S{i}"), None, &cats, &["1", "2"])).collect();
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{sers}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx><c:dTable><c:showKeys val="1"/></c:dTable></c:plotArea>"#
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    assert!(!filled(&items).is_empty(), "the columns are drawn");
    let texts = chart_texts(&items);
    assert!(texts.iter().any(|(t, _, _)| t == "Q1"), "the table's header is drawn: {texts:?}");
    let names = texts.iter().filter(|(t, _, _)| t.starts_with('S') && t[1..].parse::<u32>().is_ok()).count();
    assert!((1..15).contains(&names), "the rows that fit: {names}");
    assert!(texts.iter().all(|(_, r, _)| r[1] + r[3] <= 200.5), "inside the chart: {texts:?}");
}

/// A data table shows the data as it is: a value far smaller than an axis tick's rounding error
/// keeps its digits in its own format, and a small one in General is written in scientific
/// notation (as Excel's General shows it), not as 0.
#[test]
fn small_values_keep_their_digits_in_a_data_table() {
    let table = |fmt: &str, v: &str| {
        let s = ser(0, "S", None, &["Q1"], &[v]).replace("<c:formatCode>General</c:formatCode>", &format!("<c:formatCode>{fmt}</c:formatCode>"));
        let plot = format!(
            r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{s}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx><c:dTable/></c:plotArea>"#
        );
        chart_texts(&chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0)).into_iter().map(|(t, _, _)| t).collect::<Vec<_>>()
    };
    let fixed = table("0.0000000000", "0.0000000005");
    assert!(fixed.iter().any(|t| t == "0.0000000005"), "{fixed:?}");
    let general = table("General", "0.0000004");
    assert!(general.iter().any(|t| t == "4E-07"), "{general:?}");
    let negative = table("General", "-0.0000004");
    assert!(negative.iter().any(|t| t == "-4E-07"), "{negative:?}");
}

/// Each chart type of a combination keeps its own grouping: with a standard line first, the
/// stacked columns after it still stack (6 and 7 make one column reaching 13), and the value
/// axis takes in their sum.
#[test]
fn a_line_first_combination_keeps_its_columns_stacked() {
    let cats = ["A", "B"];
    let plot = format!(
        r#"<c:plotArea><c:lineChart><c:grouping val="standard"/>{}<c:axId val="1"/><c:axId val="2"/></c:lineChart><c:barChart><c:barDir val="col"/><c:grouping val="stacked"/><c:overlap val="100"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>"#,
        ser(0, "Line", Some("0000FF"), &cats, &["1", "2"]),
        ser(1, "Low", Some("FF0000"), &cats, &["6", "6"]),
        ser(2, "High", Some("00FF00"), &cats, &["7", "7"]),
    );
    let items = chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0);
    let ticks: Vec<f64> = chart_texts(&items).iter().filter_map(|(t, _, _)| t.parse().ok()).collect();
    assert!(ticks.iter().any(|t| *t >= 13.0), "the axis takes in the stack's 13: {ticks:?}");
    // (colour, top, bottom) of each column: the second series' columns stand on the first's.
    let columns = filled(&items);
    let span = |c: Rgb| columns.iter().find(|(k, _, _)| *k == c).map(|(_, top, bottom)| (*top, *bottom));
    let (Some((low_top, _)), Some((_, high_bottom))) = (span(Rgb(0xFF, 0, 0)), span(Rgb(0, 0xFF, 0))) else { panic!("{columns:?}") };
    assert!((low_top - high_bottom).abs() < 0.01, "one stack: {columns:?}");
}

/// A legend placed by hand is laid out as its position says: on the right, as a column, so a
/// tall box one entry wide lists every entry, not the three rows a row layout allows.
#[test]
fn a_hand_placed_side_legend_lists_its_entries_in_a_column() {
    let legend = r#"<c:legend><c:legendPos val="r"/><c:layout><c:manualLayout><c:xMode val="edge"/><c:yMode val="edge"/><c:x val="0.8"/><c:y val="0.1"/><c:w val="0.15"/><c:h val="0.8"/></c:manualLayout></c:layout></c:legend>"#;
    let names = ["N1", "N2", "N3", "N4", "N5", "N6"];
    let sers: String = names.iter().enumerate().map(|(i, n)| ser(i, n, None, &["A"], &["1"])).collect();
    let plot = format!(
        r#"<c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{sers}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea>{legend}"#
    );
    let texts = chart_texts(&chart_items(&whole_chart(&plot), &theme(), 300.0, 200.0));
    for n in names {
        let r = texts.iter().find(|(t, _, _)| t == n).map(|(_, r, _)| *r).unwrap_or_else(|| panic!("{n} missing: {texts:?}"));
        assert!(r[0] >= 239.0 && r[1] >= 19.0 && r[1] + r[3] <= 181.0, "{n} inside the box: {r:?}");
    }
}
