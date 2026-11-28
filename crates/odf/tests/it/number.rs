use craft_odf::number::{DataStyles, DateOrder, Kind};

use crate::styles_list;

fn read(inner: &str) -> DataStyles {
    let mut d = DataStyles::default();
    d.add(&styles_list(inner));
    d
}

fn code(inner: &str) -> String {
    read(inner).format_code("N1").unwrap_or_default()
}

#[test]
fn each_data_style_element_reads_as_its_code() {
    for (xml, expected) in [
        (r##"<number:number-style style:name="N1"><number:number number:min-integer-digits="1"/></number:number-style>"##, "General"),
        (
            r##"<number:number-style style:name="N1"><number:number number:decimal-places="2" number:min-integer-digits="1" number:grouping="true"/></number:number-style>"##,
            "#,##0.00",
        ),
        (
            r##"<number:number-style style:name="N1"><number:number number:decimal-places="3" number:min-decimal-places="0" number:min-integer-digits="1"/></number:number-style>"##,
            "0.###",
        ),
        (
            r##"<number:number-style style:name="N1"><number:number number:min-integer-digits="1" number:display-factor="1000" number:grouping="true"/></number:number-style>"##,
            "#,##0,",
        ),
        (
            r##"<number:number-style style:name="N1"><number:scientific-number number:decimal-places="2" number:min-integer-digits="1" number:min-exponent-digits="2"/></number:number-style>"##,
            "0.00E+00",
        ),
        (
            r##"<number:number-style style:name="N1"><number:fraction number:min-integer-digits="0" number:min-numerator-digits="1" number:denominator-value="8"/></number:number-style>"##,
            "# ?/8",
        ),
        (
            r##"<number:number-style style:name="N1"><number:number number:min-integer-digits="9"><number:embedded-text number:position="4">-</number:embedded-text><number:embedded-text number:position="6">-</number:embedded-text></number:number></number:number-style>"##,
            "000-00-0000",
        ),
        (
            r##"<number:number-style style:name="N1"><number:text>Total: </number:text><number:number number:decimal-places="2" number:min-integer-digits="1"/><number:text> kg</number:text></number:number-style>"##,
            "\"Total: \"0.00\" kg\"",
        ),
        (
            r##"<number:percentage-style style:name="N1"><number:number number:decimal-places="2" number:min-integer-digits="1"/><number:text>%</number:text></number:percentage-style>"##,
            "0.00%",
        ),
        (
            r##"<number:currency-style style:name="N1"><number:number number:decimal-places="2" number:min-integer-digits="1" number:grouping="true"/><number:text> </number:text><number:currency-symbol number:language="de" number:country="DE">€</number:currency-symbol></number:currency-style>"##,
            "#,##0.00 [$€]",
        ),
        (r##"<number:text-style style:name="N1"><number:text-content/></number:text-style>"##, "@"),
        (
            r##"<number:date-style style:name="N1"><number:year number:style="long"/><number:text>-</number:text><number:month number:style="long"/><number:text>-</number:text><number:day number:style="long"/></number:date-style>"##,
            "yyyy-mm-dd",
        ),
        (
            r##"<number:date-style style:name="N1"><number:day-of-week number:style="long"/><number:text>, </number:text><number:month number:style="long" number:textual="true"/><number:text> </number:text><number:day/><number:text>, </number:text><number:year number:style="long"/></number:date-style>"##,
            "dddd, mmmm d, yyyy",
        ),
        (
            r##"<number:time-style style:name="N1" number:truncate-on-overflow="false"><number:hours/><number:text>:</number:text><number:minutes number:style="long"/><number:text>:</number:text><number:seconds number:style="long"/></number:time-style>"##,
            "[h]:mm:ss",
        ),
        (
            r##"<number:time-style style:name="N1"><number:minutes number:style="long"/><number:text>:</number:text><number:seconds number:style="long" number:decimal-places="1"/></number:time-style>"##,
            "mm:ss.0",
        ),
        (
            r##"<number:time-style style:name="N1"><number:hours/><number:text>:</number:text><number:minutes number:style="long"/><number:text> </number:text><number:am-pm/></number:time-style>"##,
            "h:mm AM/PM",
        ),
        (
            r##"<number:number-style style:name="N1"><style:text-properties fo:color="#ff0000"/><number:number number:decimal-places="2" number:min-integer-digits="1"/></number:number-style>"##,
            "[Red]0.00",
        ),
        (
            r##"<number:number-style style:name="N1"><number:number number:decimal-places="1" number:min-integer-digits="1"/><number:text loext:blank-width-char=")"> </number:text></number:number-style>"##,
            "0.0_)",
        ),
        (r##"<number:boolean-style style:name="N1"><number:boolean/></number:boolean-style>"##, "\"TRUE\";\"TRUE\";\"FALSE\""),
        (
            r##"<number:number-style style:name="N1"><number:text>a"b</number:text><number:number number:min-integer-digits="1" number:decimal-places="0"/></number:number-style>"##,
            "\"a\"\\\"\"b\"0",
        ),
    ] {
        assert_eq!(code(xml), expected, "{xml}");
    }
}

/// Two styles: `N1` maps `N1P0` (and `N1P1`, …) under conditions.
fn mapped(main: &str, conds: &[(&str, &str)]) -> String {
    let parts: String = conds
        .iter()
        .enumerate()
        .map(|(i, (_, body))| format!(r##"<number:number-style style:name="N1P{i}">{body}</number:number-style>"##))
        .collect();
    let maps: String =
        conds.iter().enumerate().map(|(i, (c, _))| format!(r##"<style:map style:condition="{c}" style:apply-style-name="N1P{i}"/>"##)).collect();
    format!("{parts}{}", main.replace("</MAPS>", &maps))
}

#[test]
fn conditions_become_sections_in_the_order_a_spreadsheet_tries_them() {
    let two = r##"<number:number number:decimal-places="2" number:min-integer-digits="1" number:grouping="true"/>"##;
    let red = format!(r##"<style:text-properties fo:color="#ff0000"/><number:text>-</number:text>{two}"##);
    // positive;negative: the main style is the negative section, the map the positive one.
    let xml = mapped(&format!(r##"<number:number-style style:name="N1">{red}</MAPS></number:number-style>"##), &[("value()&gt;=0", two)]);
    assert_eq!(code(&xml), "#,##0.00;[Red]-#,##0.00");
    // Explicit conditions are kept, and a last section takes the rest.
    let zero = r##"<number:number number:min-integer-digits="1"/>"##;
    let xml = mapped(
        r##"<number:number-style style:name="N1"><number:number number:decimal-places="1" number:min-integer-digits="1"/></MAPS></number:number-style>"##,
        &[("value()&gt;5", r##"<number:number number:decimal-places="0" number:min-integer-digits="1"/>"##), ("value()&lt;-5", zero)],
    );
    assert_eq!(code(&xml), "[>5]0;[<-5]General;0.0");
    // A text section after "any number", in either spelling of its bound.
    for bound in ["1.7976931348623157E+308", "1.79769313486232E+308"] {
        let xml = mapped(
            r##"<number:text-style style:name="N1"><number:text-content/></MAPS></number:text-style>"##,
            &[(&format!("value()&lt;={bound}"), two)],
        );
        assert_eq!(code(&xml), "#,##0.00;@", "{bound}");
    }
}

#[test]
fn a_short_date_in_the_locales_order_is_spelled_in_the_callers() {
    let xml = r##"<number:date-style style:name="N1" number:automatic-order="true"><number:day/><number:text>.</number:text><number:month number:style="long"/><number:text>.</number:text><number:year number:style="long"/></number:date-style>"##;
    let d = read(xml);
    assert_eq!(d.format_code("N1").as_deref(), Some("d.mm.yyyy"));
    assert_eq!(d.format_code_with("N1", DateOrder::MonthDayYear).as_deref(), Some("mm/d/yyyy"));
    assert_eq!(d.format_code_with("N1", DateOrder::YearMonthDay).as_deref(), Some("yyyy/mm/d"));
    assert_eq!(d.kind("N1"), Some(Kind::Date));
    // A date with more than day, month and year keeps its own order whatever the caller's.
    let long = r##"<number:date-style style:name="N1" number:automatic-order="true"><number:day-of-week/><number:text> </number:text><number:day/><number:text>.</number:text><number:month/><number:text>.</number:text><number:year/></number:date-style>"##;
    assert_eq!(read(long).format_code_with("N1", DateOrder::MonthDayYear).as_deref(), Some("ddd d.m.yy"));
}

#[test]
fn hostile_data_styles_stay_bounded() {
    // Attribute counts past what anyone writes are held, and a map to a missing style is General.
    let xml = r##"<number:number-style style:name="N1"><number:number number:decimal-places="99999999999999999999" number:min-integer-digits="4294967296"/><style:map style:condition="value()&gt;0" style:apply-style-name="missing"/></number:number-style>"##;
    let c = code(xml);
    assert!(c.starts_with("[>0]General;") && c.len() < 200, "{c}");
    let many: String = (0..10_000).map(|_| "<number:text>x</number:text>").collect();
    assert!(code(&format!(r##"<number:number-style style:name="N1">{many}</number:number-style>"##)).len() < 2_000);
    assert_eq!(read("").format_code("N1"), None);
}
