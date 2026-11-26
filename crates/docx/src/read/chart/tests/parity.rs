//! Charts drawn as Word draws them: text in its stated colour, weight and size. Synthetic chart
//! parts written here; ported from storytold/wordcraft#4507.

use super::*;

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
