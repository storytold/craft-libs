//! Writing: a workbook read, written and read again is the same workbook, and every part this
//! crate does not read comes back byte for byte.

use craft_xlsx::testing::zip;
use craft_xlsx::{
    Cell, CellRef, CellValue, DefinedName, Document, Error, Font, KeptPart, Limits, Package, Range, Row, SheetEntry, SheetKind, StringItem, Styles,
    Value, Visibility, Workbook, Worksheet, XlsxKind,
};
use craft_xlsx::{Marker, ObjectKind, Placement};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A workbook as Excel saves one: extensions, markup compatibility, a drawing with its own
/// relationships, a chart sheet, document properties and a macro project.
fn excel_like() -> Vec<u8> {
    let ns = format!(
        r#"xmlns="{MAIN}" xmlns:r="{RT}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision" mc:Ignorable="x14ac xr""#
    );
    let book = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook {ns}><fileVersion appName="xl" lastEdited="7"/><workbookPr defaultThemeVersion="166925"/><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="x15"><x15ac:absPath xmlns:x15ac="http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac" url="C:\x\"/></mc:Choice></mc:AlternateContent><bookViews><workbookView xWindow="0" activeTab="1"/></bookViews>
<sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Chart" sheetId="3" r:id="rId9"/><sheet name="Hidden &amp; more" sheetId="2" state="hidden" r:id="rId2"/></sheets>
<definedNames><definedName name="_xlnm.Print_Area" localSheetId="0">Data!$A$1:$C$3</definedName><definedName name="Macro" function="1" vbProcedure="1" hidden="1">Module1.Run</definedName></definedNames><calcPr calcId="191029"/>
<extLst><ext uri="{{140A7094-0E35-4892-8432-C4D2E57EDEB5}}" xmlns:x15="http://schemas.microsoft.com/office/spreadsheetml/2010/11/main"><x15:workbookPr chartTrackingRefBase="1"/></ext></extLst></workbook>"#
    );
    let book_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RT}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{RT}/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId9" Type="{RT}/chartsheet" Target="chartsheets/sheet1.xml"/><Relationship Id="rId3" Type="{RT}/theme" Target="theme/theme1.xml"/><Relationship Id="rId4" Type="{RT}/styles" Target="styles.xml"/><Relationship Id="rId5" Type="{RT}/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rId6" Type="{RT}/calcChain" Target="calcChain.xml"/><Relationship Id="rId7" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/></Relationships>"#
    );
    let sheet1 = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet {ns} xr:uid="{{00000000-0001-0000-0000-000000000000}}"><sheetPr><tabColor rgb="FFFF0000"/></sheetPr><dimension ref="A1:C3"/><sheetViews><sheetView tabSelected="1" workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews><sheetFormatPr defaultRowHeight="15" x14ac:dyDescent="0.25"/><cols><col min="1" max="2" width="12.5" customWidth="1"/><col min="3" max="3" width="9" hidden="1" style="1"/></cols>
<sheetData><row r="1" spans="1:3" x14ac:dyDescent="0.25"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" s="1"><v>0.1</v></c></row><row r="2" ht="30" customHeight="1"><c r="A2"><f>SUM(C1:C3)</f><v>0.30000000000000004</v></c><c r="B2" t="b"><v>1</v></c><c r="C2" s="1"><f t="shared" ref="C2:C3" si="0">C1*2</f><v>0.2</v></c></row><row r="3"><c r="A3" t="e"><v>#N/A</v></c><c r="B3" t="inlineStr"><is><t xml:space="preserve"> inline </t></is></c><c r="C3" s="1"><f t="shared" si="0"/><v>0.4</v></c></row></sheetData>
<mergeCells count="1"><mergeCell ref="A5:B6"/></mergeCells><conditionalFormatting sqref="C1:C3"><cfRule type="cellIs" dxfId="0" priority="1" operator="greaterThan"><formula>0.25</formula></cfRule></conditionalFormatting><dataValidations count="1"><dataValidation type="list" allowBlank="1" showInputMessage="1" sqref="A1:A3 C:C" xr:uid="{{00000000-0002-0000-0000-000000000000}}"><formula1>"a,b"</formula1></dataValidation></dataValidations><hyperlinks><hyperlink ref="A1" r:id="rId2" tooltip="t"/><hyperlink ref="B1" location="Data!A1" display="here"/></hyperlinks><printOptions gridLines="1"/><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><pageSetup orientation="landscape" fitToWidth="1"/><headerFooter differentFirst="1"><oddHeader>&amp;C&amp;P</oddHeader><firstFooter xml:space="preserve"> x </firstFooter></headerFooter><rowBreaks count="1" manualBreakCount="1"><brk id="10" max="16383" man="1"/></rowBreaks><drawing r:id="rId1"/><legacyDrawing r:id="rId3"/><tableParts count="1"><tablePart r:id="rId5"/></tableParts>
<extLst><ext uri="{{05C60535-1F16-4fd2-B633-F4F36F0B64E0}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:sparklineGroups xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:sparklineGroup><x14:sparklines><x14:sparkline><xm:f>Data!C1:C3</xm:f><xm:sqref>D1</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext></extLst></worksheet>"#
    );
    let sheet1_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RT}/drawing" Target="../drawings/drawing1.xml"/><Relationship Id="rId2" Type="{RT}/hyperlink" Target="https://example.com/a?b&amp;c" TargetMode="External"/><Relationship Id="rId3" Type="{RT}/vmlDrawing" Target="../drawings/vmlDrawing1.vml"/><Relationship Id="rId4" Type="{RT}/comments" Target="../comments1.xml"/><Relationship Id="rId5" Type="{RT}/table" Target="../tables/table1.xml"/></Relationships>"#
    );
    let styles = format!(
        r#"<styleSheet {ns}><numFmts count="1"><numFmt numFmtId="164" formatCode="0.0%"/></numFmts><fonts count="2" x14ac:knownFonts="1"><font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font><font><b/><i val="0"/><u val="double"/><vertAlign val="superscript"/><sz val="9.5"/><color rgb="FF0000FF" tint="-0.25"/><name val="Arial"/></font></fonts>
<fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><gradientFill degree="90"><stop position="0"><color theme="0"/></stop><stop position="1"><color indexed="12"/></stop></gradientFill></fill></fills>
<borders count="2"><border><left/><right/><top/><bottom/><diagonal/></border><border diagonalUp="1" outline="0"><left style="thin"><color auto="1"/></left><bottom style="mediumDashDot"/></border></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="164" fontId="1" fillId="2" borderId="1" xfId="0" applyNumberFormat="1" quotePrefix="1"><alignment horizontal="centerContinuous" vertical="top" wrapText="1" textRotation="255"/><protection locked="0"/></xf></cellXfs>
<cellStyles count="2"><cellStyle name="Normal" xfId="0" builtinId="0"/><cellStyle name="RowLevel_1" xfId="0" builtinId="1" iLevel="0" hidden="1"/></cellStyles><dxfs count="1"><dxf><font><color rgb="FF9C0006"/></font><fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill></dxf></dxfs><tableStyles count="0" defaultTableStyle="TableStyleMedium2"/>
<colors><indexedColors><rgbColor rgb="FF000000"/><rgbColor rgb="FF123456"/></indexedColors><mruColors><color rgb="FF00FF00"/></mruColors></colors></styleSheet>"#
    );
    let strings = format!(
        r#"<sst {ns} count="2" uniqueCount="2"><si><t>Name</t></si><si><r><rPr><b/><sz val="11"/><rFont val="Calibri"/></rPr><t xml:space="preserve">東京 </t></r><r><t>x_x000D_</t></r><rPh sb="0" eb="2"><t>トウキョウ</t></rPh><phoneticPr fontId="1" type="Hiragana"/></si></sst>"#
    );
    let package_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RT}/officeDocument" Target="xl/workbook.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#
    );
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="bin" ContentType="application/vnd.ms-office.vbaProject"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.ms-excel.sheet.macroEnabled.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/chartsheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml"/><Override PartName="/xl/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/><Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/><Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/><Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/><Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>"#;
    zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", package_rels.as_bytes()),
        ("docProps/core.xml", b"<cp:coreProperties xmlns:cp=\"urn:core\"/>"),
        ("xl/workbook.xml", book.as_bytes()),
        ("xl/_rels/workbook.xml.rels", book_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet1.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet1_rels.as_bytes()),
        ("xl/worksheets/sheet2.xml", format!("<worksheet {ns}><sheetData/></worksheet>").as_bytes()),
        ("xl/chartsheets/sheet1.xml", b"<chartsheet/>"),
        ("xl/theme/theme1.xml", b"<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Office\"/>"),
        ("xl/drawings/drawing1.xml", DRAWING.as_bytes()),
        ("xl/drawings/_rels/drawing1.xml.rels", DRAWING_RELS.as_bytes()),
        ("xl/charts/chart1.xml", CHART.as_bytes()),
        ("xl/media/image1.png", b"\x89PNG\r\n\x1a\n"),
        ("xl/styles.xml", styles.as_bytes()),
        ("xl/sharedStrings.xml", strings.as_bytes()),
        ("xl/calcChain.xml", b"<calcChain/>"),
        ("xl/comments1.xml", COMMENTS.as_bytes()),
        ("xl/tables/table1.xml", TABLE.as_bytes()),
        ("xl/drawings/vmlDrawing1.vml", VML.as_bytes()),
        ("xl/vbaProject.bin", &[0xD0, 0xCF, 0x11, 0xE0, 1, 2, 3]),
    ])
}

const COMMENTS: &str = "<comments xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><authors><author>Ann</author></authors><commentList><comment ref=\"B2\" authorId=\"0\" shapeId=\"0\"><text><r><rPr><b/></rPr><t>Ann:</t></r><r><t xml:space=\"preserve\">\nnote</t></r></text></comment><comment ref=\"bad\"/></commentList></comments>";
const VML: &str = "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"1\"/></o:shapelayout><v:shape id=\"_x0000_s1025\" style=\"position:absolute;width:120pt;height:60pt\"><x:ClientData ObjectType=\"Note\"><x:Anchor>2, 15, 0, 2, 4, 15, 4, 16</x:Anchor><x:Row>1</x:Row><x:Column>1</x:Column><x:Visible/></x:ClientData></v:shape></xml>";

const DRAWING: &str = r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><xdr:twoCellAnchor><xdr:from><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>9</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>15</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"><a:extLst><a:ext uri="{FF2B5EF4-FFF2-40B4-BE49-F238E27FC236}"><a16:creationId xmlns:a16="http://schemas.microsoft.com/office/drawing/2014/main" id="{A}"/></a:ext></a:extLst></xdr:cNvPr><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor><xdr:oneCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>20</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="952500" cy="476250"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="3" name="Picture 2" descr="logo"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/></xdr:oneCellAnchor></xdr:wsDr>"#;
const DRAWING_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/></Relationships>"#;
const CHART: &str = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>S1</c:v></c:tx><c:cat><c:strRef><c:f>Data!$A$1:$A$2</c:f><c:strCache><c:ptCount val="2"/><c:pt idx="0"><c:v>a</c:v></c:pt><c:pt idx="1"><c:v>b</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Data!$C$1:$C$2</c:f><c:numCache><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;

const TABLE: &str = r#"<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision" id="1" xr:uid="{T}" name="Prices" displayName="Prices" ref="A1:C4" totalsRowCount="1"><autoFilter ref="A1:C3"/><tableColumns count="3"><tableColumn id="1" name="Name" totalsRowLabel="Total"/><tableColumn id="2" name="Flag"/><tableColumn id="3" name="Value" totalsRowFunction="sum" dataDxfId="0"><calculatedColumnFormula>Prices[[#This Row],[Flag]]*2</calculatedColumnFormula></tableColumn></tableColumns><tableStyleInfo name="TableStyleMedium2" showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/></table>"#;

fn read(bytes: &[u8]) -> Document {
    Document::read(bytes, &Limits::default()).unwrap()
}

/// Everything that should survive a round trip: the fields, and what each regenerated part is
/// written as (a kept tree reads back without what writing drops, such as a choice that needs a
/// namespace this crate does not know).
fn assert_same(a: &Document, b: &Document) {
    let written = |d: &Document| {
        let mut root = d.workbook.to_el();
        craft_xlsx::ns::declare(&mut root);
        root
    };
    assert_eq!(a.kind, b.kind);
    assert_eq!(
        (&a.workbook.sheets, &a.workbook.defined_names, a.workbook.date_1904, a.workbook.active_tab),
        (&b.workbook.sheets, &b.workbook.defined_names, b.workbook.date_1904, b.workbook.active_tab)
    );
    assert_eq!(written(a), written(b));
    assert_eq!(a.shared_strings, b.shared_strings);
    if let (Some(x), Some(y)) = (&a.styles, &b.styles) {
        assert_eq!(x.num_fmts, y.num_fmts);
        assert_eq!(x.fonts, y.fonts);
        assert_eq!(x.fills, y.fills);
        assert_eq!(x.borders, y.borders);
        assert_eq!(x.cell_style_xfs.iter().map(|x| x.to_el()).collect::<Vec<_>>(), y.cell_style_xfs.iter().map(|x| x.to_el()).collect::<Vec<_>>());
        assert_eq!(x.cell_xfs.iter().map(|x| x.to_el()).collect::<Vec<_>>(), y.cell_xfs.iter().map(|x| x.to_el()).collect::<Vec<_>>());
        assert_eq!(x.cell_styles, y.cell_styles);
        assert_eq!(x.dxfs, y.dxfs);
        assert_eq!(x.indexed_colors, y.indexed_colors);
        assert_eq!(String::from_utf8(x.to_xml()), String::from_utf8(y.to_xml()));
    } else {
        assert!(a.styles.is_none() && b.styles.is_none());
    }
    assert_eq!(a.worksheets.len(), b.worksheets.len());
    for (x, y) in a.worksheets.iter().zip(&b.worksheets) {
        let (Some(x), Some(y)) = (x, y) else {
            assert!(x.is_none() && y.is_none());
            continue;
        };
        assert_eq!(x.rows, y.rows);
        assert_eq!((&x.part.part, &x.part.columns, &x.part.merges), (&y.part.part, &y.part.columns, &y.part.merges));
        assert_eq!(String::from_utf8(x.part.to_xml(&x.rows)), String::from_utf8(y.part.to_xml(&y.rows)));
        assert_eq!(x.part.rels.list, y.part.rels.list);
        assert_eq!(x.part.features, y.part.features);
        assert_eq!(x.notes, y.notes);
        assert_eq!(x.drawing, y.drawing);
        assert_eq!(x.tables, y.tables);
        assert_eq!(
            x.comments.as_ref().map(|c| (&c.authors, &c.comments, c.to_xml())),
            y.comments.as_ref().map(|c| (&c.authors, &c.comments, c.to_xml()))
        );
    }
    let mut pa = a.parts.clone();
    let mut pb = b.parts.clone();
    pa.sort_by(|x, y| x.name.cmp(&y.name));
    pb.sort_by(|x, y| x.name.cmp(&y.name));
    assert_eq!(pa, pb);
}

#[test]
fn a_workbook_round_trips() {
    let doc = read(&excel_like());
    assert_eq!(doc.kind, XlsxKind::MacroWorkbook);
    assert_eq!(doc.worksheets.iter().map(Option::is_some).collect::<Vec<_>>(), [true, false, true]);
    let kept: Vec<&str> = doc.parts.iter().map(|p| p.name.as_str()).collect();
    for name in ["_rels/.rels", "docProps/core.xml", "xl/theme/theme1.xml", "xl/charts/chart1.xml", "xl/chartsheets/sheet1.xml", "xl/vbaProject.bin"]
    {
        assert!(kept.contains(&name), "{name} in {kept:?}");
    }
    assert!(
        !kept.iter().any(|n| n.contains("calcChain") || n.contains("styles") || n.contains("sharedStrings") || n.contains("worksheets")),
        "{kept:?}"
    );

    let bytes = doc.write().unwrap();
    let again = read(&bytes);
    assert_same(&doc, &again);
    // Written again, the bytes are the same.
    assert_eq!(again.write().unwrap(), bytes);

    // What the reader sees, through the part layer.
    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    assert!(p.opc().read("xl/calcChain.xml").unwrap().is_none());
    let wb = p.workbook().unwrap();
    assert_eq!(wb.active_tab, Some(1));
    assert_eq!(wb.defined_names[1].attrs, [("function".to_string(), "1".to_string()), ("vbProcedure".to_string(), "1".to_string())]);
    assert!(wb.root.child("mc:AlternateContent").is_none(), "a choice requiring an unlisted namespace is dropped");
    assert!(wb.root.find("x15:workbookPr").is_some());
    let strings = p.shared_strings().unwrap();
    let mut values = vec![];
    p.sheet(&wb.sheets[0], |row| {
        values.extend(row.cells.into_iter().map(|c| (c.at.a1(), c.value.resolve(&strings, false))));
        Ok(())
    })
    .unwrap();
    assert_eq!(values[1], ("B1".to_string(), Value::text("東京 x\r")));
    assert_eq!(values[7], ("B3".to_string(), Value::text(" inline ")));
    let book = String::from_utf8(p.opc().read("xl/workbook.xml").unwrap().unwrap()).unwrap();
    assert!(book.contains(r#"<sheet name="Hidden &amp; more" sheetId="2" state="hidden" r:id="rId2"/>"#), "{book}");
    let sheet = String::from_utf8(p.opc().read("xl/worksheets/sheet1.xml").unwrap().unwrap()).unwrap();
    assert!(sheet.contains(r#"mc:Ignorable="x14ac xr""#) && sheet.contains(r#"<row r="1" x14ac:dyDescent="0.25">"#), "{sheet}");
    let types = String::from_utf8(p.opc().read("[Content_Types].xml").unwrap().unwrap()).unwrap();
    assert!(types.contains("macroEnabled.main+xml") && types.contains(r#"Extension="bin""#) && !types.contains("calcChain"), "{types}");
}

#[test]
fn features_are_read() {
    let doc = read(&excel_like());
    let ws = doc.worksheets[0].as_ref().unwrap();
    let f = &ws.part.features;
    assert_eq!(f.views[0].pane.as_ref().and_then(|p| p.state.as_deref()), Some("frozen"));
    assert_eq!(f.views[0].tab_selected, Some(true));
    let dv = &f.validations.as_ref().unwrap().items[0];
    assert_eq!((dv.kind.as_deref(), dv.formula1.as_deref(), dv.sqref.len()), (Some("list"), Some("\"a,b\""), 2));
    assert_eq!(dv.attrs, [("xr:uid".to_string(), "{00000000-0002-0000-0000-000000000000}".to_string())]);
    assert_eq!(craft_xlsx::range_ref(&dv.sqref[1]), "C:C");
    assert_eq!(f.conditional_formats[0].rules[0].formulas, ["0.25"]);
    assert_eq!(f.hyperlinks[0].url.as_deref(), Some("https://example.com/a?b&c"));
    assert_eq!((f.hyperlinks[1].location.as_deref(), f.hyperlinks[1].url.as_deref()), (Some("Data!A1"), None));
    assert_eq!(f.print.options.as_ref().and_then(|o| o.grid_lines), Some(true));
    assert_eq!(f.print.setup.as_ref().and_then(|s| s.orientation.as_deref()), Some("landscape"));
    let hf = f.print.header_footer.as_ref().unwrap();
    assert_eq!((hf.odd_header.as_deref(), hf.first_footer.as_deref(), hf.different_first), (Some("&C&P"), Some(" x "), Some(true)));
    assert_eq!((f.print.row_breaks[0].id, f.print.row_breaks[0].man), (Some(10), Some(true)));
    assert_eq!(f.legacy_drawing.as_deref(), Some("rId3"));
    let comments = ws.comments.as_ref().unwrap();
    assert_eq!(comments.comments.len(), 1);
    assert_eq!((comments.author(&comments.comments[0]), &*comments.comments[0].text.text), (Some("Ann"), "Ann:\nnote"));
    assert_eq!(
        ws.notes,
        [craft_xlsx::NoteShape { at: CellRef::new(1, 1), visible: true, anchor: Some([2, 15, 0, 2, 4, 15, 4, 16]), size_pt: Some((120.0, 60.0)) }]
    );
    assert!(doc.parts.iter().any(|p| p.name == "xl/drawings/vmlDrawing1.vml") && !doc.parts.iter().any(|p| p.name == "xl/comments1.xml"));
}

#[test]
fn edits_are_written() {
    let mut doc = read(&excel_like());
    let ws = doc.worksheets[0].as_mut().unwrap();
    ws.rows.push(Row { index: 9, cells: vec![Cell { at: CellRef::new(9, 3), value: CellValue::Shared(2), ..Cell::default() }], ..Row::default() });
    ws.part.merges.push(Range::parse("E1:F1").unwrap());
    doc.shared_strings.push(StringItem::plain("new"));
    doc.workbook.sheets[2].visibility = Visibility::Visible;
    doc.workbook.date_1904 = true;
    doc.styles.as_mut().unwrap().fonts[0].italic = Some(true);
    let again = read(&doc.write().unwrap());
    assert_same(&doc, &again);
    let ws = again.worksheets[0].as_ref().unwrap();
    assert_eq!(ws.part.root.child("dimension").and_then(|d| d.attr("ref")), Some("A1:D10"));
    assert_eq!(ws.part.root.child("mergeCells").and_then(|m| m.attr("count")), Some("2"));

    // A sheet removed takes its relationship with it.
    doc.workbook.sheets.remove(2);
    doc.worksheets.remove(2);
    let bytes = doc.write().unwrap();
    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    let rels = p.workbook_rels().unwrap();
    assert!(!rels.list.iter().any(|r| r.target.contains("sheet2")));
    assert!(p.opc().read("xl/worksheets/sheet2.xml").unwrap().is_none());
    assert_eq!(rels.all("worksheet").count(), 1);
}

#[test]
fn features_are_edited() {
    let mut doc = read(&excel_like());
    let ws = doc.worksheets[0].as_mut().unwrap();
    let f = &mut ws.part.features;
    f.hyperlinks[0].url = Some("https://example.org/".into());
    f.conditional_formats.push(craft_xlsx::ConditionalFormatting {
        sqref: vec![Range::parse("A1:A9").unwrap()],
        rules: vec![craft_xlsx::CfRule { kind: Some("expression".into()), priority: Some(2), formulas: vec!["A1>1".into()], ..Default::default() }],
        ..Default::default()
    });
    f.print.setup = None;
    // A comment on a sheet without a VML drawing gets one, in a shape block no other drawing uses.
    let empty = doc.worksheets[2].as_mut().unwrap();
    empty.comments = Some(craft_xlsx::Comments {
        authors: vec!["Bo".into()],
        comments: vec![craft_xlsx::Comment { at: CellRef::new(0, 0), author_id: Some(0), text: StringItem::plain("hi"), ..Default::default() }],
        ..Default::default()
    });
    let bytes = doc.write().unwrap();
    let again = read(&bytes);
    assert_same(&doc_after(&doc), &again);
    let f = &again.worksheets[0].as_ref().unwrap().part.features;
    assert_eq!(f.hyperlinks[0].url.as_deref(), Some("https://example.org/"));
    assert_eq!(f.conditional_formats.len(), 2);
    assert!(f.print.setup.is_none());
    let sheet2 = again.worksheets[2].as_ref().unwrap();
    assert_eq!(sheet2.comments.as_ref().map(|c| c.comments[0].text.text.to_string()), Some("hi".into()));
    assert!(sheet2.part.features.legacy_drawing.is_some());
    assert_eq!(sheet2.notes.iter().map(|n| (n.at, n.visible)).collect::<Vec<_>>(), [(CellRef::new(0, 0), false)]);
    let vml = again.parts.iter().find(|p| p.name == "xl/drawings/vmlDrawing2.vml").unwrap();
    assert!(String::from_utf8_lossy(&vml.bytes).contains(r#"data="2""#));
    let sheet =
        String::from_utf8(Package::open(&bytes, &Limits::default()).unwrap().opc().read("xl/worksheets/sheet1.xml").unwrap().unwrap()).unwrap();
    let order = [
        "<conditionalFormatting",
        "<conditionalFormatting sqref=\"A1:A9\"",
        "<dataValidations",
        "<hyperlinks",
        "<printOptions",
        "<pageMargins",
        "<headerFooter",
    ];
    let at: Vec<usize> = order.iter().map(|o| sheet.find(o).unwrap()).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{sheet}");
    // Comments removed take their part and relationship with them.
    let mut doc = again;
    doc.worksheets[0].as_mut().unwrap().comments = None;
    let bytes = doc.write().unwrap();
    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    assert!(p.opc().read("xl/comments1.xml").unwrap().is_none());
    let wb = p.workbook().unwrap();
    let part = p.sheet(&wb.sheets[0], |_| Ok(())).unwrap();
    assert!(part.rels.first("comments").is_none());
}

/// The document as written: a link to a new URL has a relationship of its own, a new comments
/// part a VML drawing.
fn doc_after(doc: &Document) -> Document {
    read(&doc.write().unwrap())
}

#[test]
fn charts_and_pictures() {
    let bytes = excel_like();
    let mut doc = read(&bytes);
    let d = doc.worksheets[0].as_ref().unwrap().drawing.clone().unwrap();
    assert_eq!(d.part, "xl/drawings/drawing1.xml");
    let anchors = d.drawing.anchors();
    assert_eq!(anchors.iter().map(|a| a.kind()).collect::<Vec<_>>(), [ObjectKind::Chart, ObjectKind::Picture]);
    assert_eq!((anchors[1].picture_rel(), anchors[1].name()), (Some("rId2"), Some("Picture 2")));
    let chart_part = d.rels.iter().find(|r| Some(r.id.as_str()) == anchors[0].chart_rel()).map(|r| r.target.clone()).unwrap();
    assert!(!doc.parts.iter().any(|p| p.name.starts_with("xl/drawings/drawing1")), "the drawing is not a kept part");

    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    let chart = p.chart(&chart_part).unwrap().unwrap();
    assert_eq!(
        (chart.series.len(), chart.series[0].values.clone(), chart.categories.clone()),
        (1, vec![Some(1.0), Some(2.0)], vec!["a".to_string(), "b".to_string()])
    );
    let theme = p.theme().unwrap().unwrap();
    assert_eq!(theme.name, "Office");

    // A chart and a picture on the sheet without a drawing.
    let at = Placement::OneCell { from: Marker { col: 1, row: 1, ..Marker::default() }, cx: 3_000_000, cy: 2_000_000 };
    let mut fresh = chart.clone();
    fresh.raw = None;
    doc.add_chart(2, at.clone(), &fresh, "Chart 9").unwrap();
    let waterfall = craft_xlsx::craft_drawingml::chart::Chart { kind: craft_xlsx::craft_drawingml::chart::ChartType::Waterfall, ..fresh.clone() };
    doc.add_chart(2, at.clone(), &waterfall, "Waterfall").unwrap();
    doc.add_picture(2, at, b"GIF89a".to_vec(), "gif", "small").unwrap();
    assert!(
        doc.add_picture(1, Placement::Absolute { x: 0, y: 0, cx: 1, cy: 1 }, vec![], "png", "").is_err(),
        "a chart sheet has no drawing of its own here"
    );
    let out = doc.write().unwrap();
    let again = read(&out);
    assert_same(&doc_after(&doc), &again);
    let d2 = again.worksheets[2].as_ref().unwrap().drawing.clone().unwrap();
    assert_eq!(d2.part, "xl/drawings/drawing2.xml");
    let kinds: Vec<_> = d2.drawing.anchors().iter().map(|a| (a.kind(), a.props().and_then(|p| p.attr_u32("id")))).collect();
    assert_eq!(kinds, [(ObjectKind::Chart, Some(2)), (ObjectKind::Picture, Some(4))]);
    let alternate: Vec<_> = d2.drawing.items.iter().filter_map(|i| if let craft_xlsx::Item::Other(e) = i { Some(e) } else { None }).collect();
    let ex = craft_xlsx::Drawing::alternate_anchors(alternate[0]);
    assert_eq!((ex[0].kind(), ex[0].name()), (ObjectKind::Chart, Some("Waterfall")));
    let targets: Vec<&str> = d2.rels.iter().map(|r| r.target.as_str()).collect();
    assert_eq!(targets, ["xl/charts/chart2.xml", "xl/charts/chartEx1.xml", "xl/media/image1.gif"]);
    let mut p = Package::open(&out, &Limits::default()).unwrap();
    assert_eq!(p.chart("xl/charts/chart2.xml").unwrap().map(|c| c.series.len()), Some(1));
    assert_eq!(p.chart("xl/charts/chartEx1.xml").unwrap().map(|c| c.kind), Some(craft_xlsx::craft_drawingml::chart::ChartType::Waterfall));
    let drawing = String::from_utf8(p.opc().read("xl/drawings/drawing2.xml").unwrap().unwrap()).unwrap();
    assert!(drawing.contains(r#"Requires="cx1""#) && drawing.contains("xmlns:cx1=") && drawing.contains("<cx:chart r:id="), "{drawing}");
    let types = String::from_utf8(p.opc().read("[Content_Types].xml").unwrap().unwrap()).unwrap();
    assert!(types.contains(r#"Extension="gif" ContentType="image/gif""#) && types.contains("/xl/drawings/drawing2.xml"), "{types}");
    let wb = p.workbook().unwrap();
    let sheet = p.sheet(&wb.sheets[2], |_| Ok(())).unwrap();
    assert!(sheet.features.drawing.as_deref().is_some_and(|id| sheet.rels.by_id(id).is_some_and(|r| r.target == "xl/drawings/drawing2.xml")));

    // A drawing removed takes its relationship with it; its charts stay as kept parts.
    let mut doc = again;
    doc.worksheets[2].as_mut().unwrap().drawing = None;
    let out = doc.write().unwrap();
    let mut p = Package::open(&out, &Limits::default()).unwrap();
    let wb = p.workbook().unwrap();
    let sheet = p.sheet(&wb.sheets[2], |_| Ok(())).unwrap();
    assert!(sheet.rels.first("drawing").is_none() && sheet.features.drawing.is_none());
}

#[test]
fn tables() {
    let mut doc = read(&excel_like());
    let ws = doc.worksheets[0].clone().unwrap();
    assert_eq!(ws.part.features.table_parts, ["rId5"]);
    let t = &ws.tables[0];
    assert_eq!(
        (t.part.as_str(), t.table.name.as_deref(), t.table.range.a1(), t.table.totals_row_count),
        ("xl/tables/table1.xml", Some("Prices"), "A1:C4".to_string(), Some(1))
    );
    assert_eq!(t.table.attrs, [("xr:uid".to_string(), "{T}".to_string())]);
    let value = &t.table.columns[2];
    assert_eq!(
        (value.totals_row_function.as_deref(), value.data_dxf_id, value.calculated_formula.clone()),
        (Some("sum"), Some(0), Some(("Prices[[#This Row],[Flag]]*2".to_string(), false)))
    );
    assert_eq!(t.table.style.as_ref().and_then(|s| s.show_row_stripes), Some(true));
    assert!(t.table.auto_filter.is_some());
    assert!(!doc.parts.iter().any(|p| p.name.contains("tables/")));

    // A new table on the third sheet takes a name no other table has.
    let mut new = t.table.clone();
    new.id = Some(2);
    new.name = Some("Other".into());
    new.display_name = Some("Other".into());
    doc.worksheets[2].as_mut().unwrap().tables.push(craft_xlsx::TablePart { part: String::new(), table: new });
    let again = read(&doc.write().unwrap());
    assert_same(&doc_after(&doc), &again);
    let t2 = again.worksheets[2].clone().unwrap().tables.remove(0);
    assert_eq!((t2.part.as_str(), t2.table.name.as_deref()), ("xl/tables/table2.xml", Some("Other")));
    assert_eq!(again.worksheets[0].as_ref().unwrap().tables[0].table, t.table);

    // A table removed takes its relationship and its tableParts entry with it.
    let mut doc = again;
    doc.worksheets[0].as_mut().unwrap().tables.clear();
    let bytes = doc.write().unwrap();
    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    let wb = p.workbook().unwrap();
    let sheet = p.sheet(&wb.sheets[0], |_| Ok(())).unwrap();
    assert!(sheet.rels.first("table").is_none() && sheet.features.table_parts.is_empty() && sheet.root.child("tableParts").is_none());
    assert!(p.opc().read("xl/tables/table1.xml").unwrap().is_none());

    // A new part on an earlier sheet never takes the name a later sheet's part keeps.
    let mut doc = read(&excel_like());
    let kept = doc.worksheets[0].as_mut().unwrap().tables.remove(0);
    doc.worksheets[2].as_mut().unwrap().tables.push(kept);
    doc.worksheets[0].as_mut().unwrap().tables.push(craft_xlsx::TablePart { part: String::new(), table: t2.table.clone() });
    let again = read(&doc.write().unwrap());
    let names = |i: usize| again.worksheets[i].as_ref().unwrap().tables.iter().map(|t| t.part.clone()).collect::<Vec<_>>();
    assert_eq!((names(0), names(2)), (vec!["xl/tables/table2.xml".to_string()], vec!["xl/tables/table1.xml".to_string()]));
}

#[test]
fn a_new_workbook() {
    let cell = |at: &str, value| Cell { at: CellRef::parse(at).unwrap(), value, ..Cell::default() };
    let doc = Document {
        workbook: Workbook {
            sheets: vec![SheetEntry {
                name: "Sheet1".into(),
                sheet_id: 1,
                visibility: Visibility::Visible,
                rel_id: String::new(),
                kind: SheetKind::Worksheet,
                part: None,
            }],
            defined_names: vec![DefinedName { name: "Two".into(), formula: "Sheet1!$B$1".into(), ..DefinedName::default() }],
            ..Workbook::default()
        },
        shared_strings: vec![StringItem::plain("hello")],
        styles: Some(Styles { fonts: vec![Font { size: Some(11.0), name: Some("Calibri".into()), ..Font::default() }], ..Styles::default() }),
        worksheets: vec![Some(Worksheet {
            rows: vec![Row { index: 0, cells: vec![cell("A1", CellValue::Shared(0)), cell("B1", CellValue::Number(2.0))], ..Row::default() }],
            ..Worksheet::default()
        })],
        ..Document::default()
    };
    let bytes = doc.write().unwrap();
    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    assert_eq!(p.workbook_part(), "xl/workbook.xml");
    let wb = p.workbook().unwrap();
    assert_eq!((wb.sheets[0].part.as_deref(), wb.sheets[0].rel_id.as_str()), (Some("xl/worksheets/sheet1.xml"), "rId1"));
    assert_eq!(wb.defined_names[0].formula, "Sheet1!$B$1");
    let strings = p.shared_strings().unwrap();
    let mut values = vec![];
    p.sheet(&wb.sheets[0], |row| {
        values.extend(row.cells.into_iter().map(|c| c.value.resolve(&strings, false)));
        Ok(())
    })
    .unwrap();
    assert_eq!(values, [Value::text("hello"), Value::Number(2.0)]);
    assert_eq!(p.styles().unwrap().unwrap().fonts.len(), 1);
    assert_eq!(read(&bytes).parts.iter().map(|p: &KeptPart| p.name.as_str()).collect::<Vec<_>>(), ["_rels/.rels"]);
}

#[test]
fn what_cannot_be_written_is_refused() {
    let mut doc = read(&excel_like());
    doc.worksheets.pop();
    assert!(matches!(doc.write(), Err(Error::Invalid(_))));
    let mut doc = read(&excel_like());
    doc.workbook.sheets[1].rel_id = "nowhere".into();
    assert!(matches!(doc.write(), Err(Error::Invalid(_))));
}
