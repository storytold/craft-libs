//! craft-hwpx integration tests: damaged packages, and the documents of a local sample folder.

use craft_testkit::hostile::{HammerConfig, hammer};

const HEAD: &str = r##"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hh:refList><hh:borderFills><hh:borderFill id="1"><hh:leftBorder type="SOLID" width="0.12 mm" color="#000000"/></hh:borderFill><hh:borderFill id="2"><hc:fillBrush><hc:winBrush faceColor="#FFEEDD"/></hc:fillBrush></hh:borderFill></hh:borderFills><hh:charProperties><hh:charPr id="0" height="1000"/></hh:charProperties><hh:paraProperties><hh:paraPr id="0"><hh:align horizontal="JUSTIFY"/><hh:lineSpacing type="PERCENT" value="160" unit="HWPUNIT"/></hh:paraPr><hh:paraPr id="1"><hh:lineSpacing type="BETWEEN_LINES" value="300" unit="HWPUNIT"/></hh:paraPr></hh:paraProperties><hh:styles><hh:style id="0" type="PARA" name="바탕글" paraPrIDRef="0" charPrIDRef="0"/></hh:styles></hh:refList></hh:head>"##;

/// A section touching most of what the reader reads: page setup with a master page and a
/// background, a header, text with a field and a note, a table with merged cells, a shape with
/// text, a group, a form object and an equation.
const SECTION: &str = r##"<hs:sec xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hp:p paraPrIDRef="0" styleIDRef="0"><hp:run charPrIDRef="0"><hp:secPr><hp:pagePr width="59528" height="84188"><hp:margin left="8504" right="8504" top="5668" bottom="4252" header="4252" footer="4252"/></hp:pagePr><hp:pageBorderFill type="BOTH" borderFillIDRef="2" textBorder="PAPER" fillArea="PAPER"/><hp:masterPage type="BOTH"><hp:subList><hp:p><hp:run><hp:rect textWrap="IN_FRONT_OF_TEXT"><hp:sz width="7200" height="7200"/><hp:pos treatAsChar="0" vertRelTo="PAGE" horzRelTo="PAGE"/></hp:rect></hp:run></hp:p></hp:subList></hp:masterPage></hp:secPr><hp:ctrl><hp:header applyPageType="BOTH"><hp:subList><hp:p><hp:run><hp:t>머리말</hp:t></hp:run></hp:p></hp:subList></hp:header></hp:ctrl><hp:t>본문 <hp:tab/>글</hp:t><hp:ctrl><hp:fieldBegin id="7" type="HYPERLINK"><hp:parameters><hp:stringParam name="Command">https://example.com;1;0;0;</hp:stringParam></hp:parameters></hp:fieldBegin></hp:ctrl><hp:t>링크</hp:t><hp:ctrl><hp:fieldEnd beginIDRef="7"/></hp:ctrl><hp:ctrl><hp:footNote><hp:subList><hp:p><hp:run><hp:t>각주</hp:t></hp:run></hp:p></hp:subList></hp:footNote></hp:ctrl></hp:run></hp:p><hp:p><hp:run><hp:tbl rowCnt="2" colCnt="2" borderFillIDRef="1"><hp:sz width="20000" height="4000"/><hp:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="COLUMN" horzAlign="CENTER"/><hp:inMargin left="141" right="141" top="141" bottom="141"/><hp:tr><hp:tc borderFillIDRef="1"><hp:subList><hp:p><hp:run><hp:t>가</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="2"/><hp:cellSz width="10000" height="4000"/></hp:tc><hp:tc><hp:subList><hp:p><hp:run><hp:t>나</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="1" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="10000" height="2000"/></hp:tc></hp:tr><hp:tr><hp:tc><hp:subList><hp:p><hp:run><hp:t>다</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="1" rowAddr="1"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="10000" height="2000"/></hp:tc></hp:tr></hp:tbl></hp:run></hp:p><hp:p paraPrIDRef="1"><hp:run><hp:ctrl><hp:colPr colCount="2" sameGap="1134"/></hp:ctrl><hp:t>둘째 단</hp:t></hp:run></hp:p><hp:p><hp:run><hp:ellipse textWrap="SQUARE"><hp:sz width="8000" height="6000"/><hp:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="COLUMN"/><hp:lineShape color="#FF0000" width="100" style="DASH" headStyle="ARROW"/><hc:fillBrush><hc:gradation type="LINEAR" angle="90"><hc:color value="#FFFFFF"/><hc:color value="#0000FF"/></hc:gradation></hc:fillBrush><hp:drawText><hp:textMargin left="283" right="283" top="283" bottom="283"/><hp:subList vertAlign="CENTER"><hp:p><hp:run><hp:t>도형</hp:t></hp:run></hp:p></hp:subList></hp:drawText></hp:ellipse><hp:container><hp:sz width="9000" height="9000"/><hp:pos treatAsChar="1"/><hp:rect><hp:offset x="0" y="0"/><hp:sz width="4000" height="4000"/></hp:rect><hp:polygon><hp:offset x="4000" y="4000"/><hp:sz width="5000" height="5000"/><hp:orgSz width="5000" height="5000"/><hc:pt x="0" y="0"/><hc:pt x="5000" y="0"/><hc:pt x="2500" y="5000"/><hc:pt x="0" y="0"/></hp:polygon></hp:container><hp:checkBtn value="1" caption="동의"><hp:sz width="5000" height="1200"/><hp:pos treatAsChar="1"/></hp:checkBtn><hp:equation><hp:script>x over y</hp:script></hp:equation></hp:run></hp:p></hs:sec>"##;

fn package(head: &str, section: &str) -> Vec<u8> {
    let mut z = craft_zip::Writer::new();
    let _ = z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored);
    let _ = z.add("Contents/header.xml", head.as_bytes(), craft_zip::Compression::Deflated);
    let _ = z.add("Contents/section0.xml", section.as_bytes(), craft_zip::Compression::Deflated);
    z.finish().unwrap_or_default()
}

/// The sample package reads, with something of each kind in it, so the sweep below damages a
/// file that exercises the reader rather than one it rejects at once.
#[test]
fn the_sweep_seed_reads_whole() {
    let doc = craft_hwpx::read(&package(HEAD, SECTION)).unwrap();
    assert!(doc.body.iter().any(|b| b.as_table().is_some()), "no table");
    let objects: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).collect();
    assert!(objects.iter().any(|o| matches!(o, craft_wordmodel::InlineObject::Group { .. })), "no group");
    assert!(doc.parts.values().any(|p| p.kind == craft_wordmodel::PartKind::Header), "no header");
    assert!(doc.parts.values().any(|p| p.kind == craft_wordmodel::PartKind::TextBox), "no text box");
}

/// Damaged packages (every truncation and many mutations, of the package and of its section
/// XML) give a document or an error, never a panic or a hang. The rounds go through the shared
/// budget (`CRAFT_HOSTILE_DIVISOR`).
#[test]
fn damaged_packages_never_panic() {
    let seed = package(HEAD, SECTION);
    hammer(&HammerConfig::new("hwpx package", 400), &seed, |b| {
        let _ = craft_hwpx::read(b);
    });
    hammer(&HammerConfig::new("hwpx section xml", 400), SECTION.as_bytes(), |b| {
        let _ = craft_hwpx::read(&package(HEAD, &String::from_utf8_lossy(b)));
    });
}

/// Deep nesting (tables in cells, shapes in shapes) stops at the reader's limits instead of
/// exhausting the stack.
#[test]
fn deep_nesting_is_bounded() {
    let mut section = String::from("<hp:p><hp:run><hp:t>끝</hp:t></hp:run></hp:p>");
    for _ in 0..2000 {
        section = format!(
            r#"<hp:p><hp:run><hp:tbl rowCnt="1" colCnt="1"><hp:tr><hp:tc><hp:subList>{section}</hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/></hp:tc></hp:tr></hp:tbl></hp:run></hp:p>"#
        );
    }
    let section = format!(
        r#"<hs:sec xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section">{section}</hs:sec>"#
    );
    let _ = craft_hwpx::read(&package(HEAD, &section));
}

/// Every `.hwpx` in `CRAFT_HWP_SAMPLES` reads, and its body has content. Without the variable
/// the test says so and passes, unless `CRAFT_HWP_SAMPLES_REQUIRED=1`. Failures name a sample by
/// its place in the sorted folder, not its name (samples may be private).
#[test]
fn local_samples_read() {
    let required = match std::env::var("CRAFT_HWP_SAMPLES_REQUIRED").unwrap_or_default().as_str() {
        "" => false,
        "1" => true,
        other => panic!("CRAFT_HWP_SAMPLES_REQUIRED is {other:?}: leave it unset or empty, or set it to 1"),
    };
    let Some(dir) = std::env::var_os("CRAFT_HWP_SAMPLES").filter(|d| !d.is_empty()) else {
        assert!(!required, "CRAFT_HWP_SAMPLES_REQUIRED=1 but CRAFT_HWP_SAMPLES is not set");
        println!("SKIPPED: CRAFT_HWP_SAMPLES is not set (a folder of .hwpx files to read)");
        return;
    };
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("hwpx"))).collect())
        .unwrap_or_default();
    files.sort();
    if required {
        assert!(!files.is_empty(), "no .hwpx file in CRAFT_HWP_SAMPLES");
    }
    for (k, f) in files.iter().enumerate() {
        let bytes = std::fs::read(f).unwrap_or_default();
        let doc = craft_hwpx::read(&bytes).unwrap_or_else(|e| panic!("sample #{} did not read: {e}", k + 1));
        assert!(!doc.body.is_empty(), "sample #{} has an empty body", k + 1);
    }
    println!("{} samples read", files.len());
}

/// Every number attribute of the sweep's package set to the extremes a file can write, one at a
/// time: the document reads or gives an error, never a panic. Numbers come in as wide as `i64`.
#[test]
fn extreme_numbers_never_panic() {
    let extremes = ["9223372036854775807", "-9223372036854775808", "4294967295", "-1"];
    let number = |v: &str| !v.is_empty() && v.trim().parse::<f64>().is_ok();
    let sweep = |xml: &str, read: &dyn Fn(&str)| {
        let mut at = 0;
        let mut failed = Vec::new();
        while let Some(q) = xml[at..].find("=\"").map(|i| at + i + 2) {
            let Some(end) = xml[q..].find('"').map(|i| q + i) else { break };
            if number(&xml[q..end]) {
                for e in extremes {
                    let changed = format!("{}{e}{}", &xml[..q], &xml[end..]);
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(&changed))).is_err() {
                        let name_start = xml[..q - 2].rfind(' ').map_or(0, |i| i + 1);
                        failed.push(format!("{}={e}", &xml[name_start..q - 2]));
                    }
                }
            }
            at = end + 1;
        }
        failed
    };
    let mut failed = sweep(SECTION, &|s| {
        let _ = craft_hwpx::read(&package(HEAD, s));
    });
    failed.extend(sweep(HEAD, &|h| {
        let _ = craft_hwpx::read(&package(h, SECTION));
    }));
    assert!(failed.is_empty(), "panicked: {failed:?}");
}
