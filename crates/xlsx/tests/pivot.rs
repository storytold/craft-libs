//! PivotTables: caches, records and tables read, written back, and made new.

use craft_xlsx::testing::zip;
use craft_xlsx::{
    CacheItem, CacheValue, DataField, Document, Error, Limits, Location, Package, PivotCacheDefinition, PivotCachePart, PivotField, PivotItem,
    PivotTableDefinition, PivotTablePart, Range, SharedItems, records_xml,
};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

const CACHE: &str = r#"<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId1" refreshOnLoad="1" refreshedBy="Me" createdVersion="6" recordCount="2" xr:uid="{C}" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision"><cacheSource type="worksheet"><worksheetSource ref="A1:B3" sheet="Data"/></cacheSource><cacheFields count="3"><cacheField name="Region" numFmtId="0"><sharedItems count="2"><s v="East"/><s v="West" u="1"/></sharedItems></cacheField><cacheField name="Day" numFmtId="14"><sharedItems containsSemiMixedTypes="0" containsNonDate="0" containsDate="1" containsString="0" minDate="2024-01-01T00:00:00" maxDate="2024-02-01T00:00:00"/><fieldGroup par="2" base="1"><rangePr groupBy="months" startDate="2024-01-01T00:00:00" endDate="2024-02-02T00:00:00"/><groupItems count="3"><s v="&lt;1/1/2024"/><s v="Jan"/><s v="&gt;2/2/2024"/></groupItems></fieldGroup></cacheField><cacheField name="Ratio" numFmtId="0" formula="Day/2" databaseField="0"/></cacheFields><extLst><ext uri="{725AE2AE-9491-48be-B2B4-4EB974FC3084}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:pivotCacheDefinition/></ext></extLst></pivotCacheDefinition>"#;
const RECORDS: &str = r#"<pivotCacheRecords xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2"><r><x v="0"/><d v="2024-01-01T00:00:00"/></r><r><x v="1"/><d v="2024-02-01T00:00:00"/></r></pivotCacheRecords>"#;
const TABLE: &str = r#"<pivotTableDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" name="PivotTable1" cacheId="5" dataCaption="Values" applyNumberFormats="0" outline="1" outlineData="1" rowGrandTotals="0" updatedVersion="6"><location ref="D1:E4" firstHeaderRow="1" firstDataRow="1" firstDataCol="1" rowPageCount="1" colPageCount="1"/><pivotFields count="3"><pivotField axis="axisRow" showAll="0" sortType="descending"><items count="3"><item x="1" h="1"/><item x="0" sd="0"/><item t="default"/></items></pivotField><pivotField axis="axisPage" showAll="0" multipleItemSelectionAllowed="1"><items count="1"><item x="0"/></items></pivotField><pivotField dataField="1" showAll="0"/></pivotFields><rowFields count="2"><field x="0"/><field x="-2"/></rowFields><rowItems count="1"><i><x/></i></rowItems><pageFields count="1"><pageField fld="1" item="0" hier="-1"/></pageFields><dataFields count="1"><dataField name="Rank of Day" fld="1" baseField="0" baseItem="0" numFmtId="3"><extLst><ext uri="{E15A36E0-9728-4e99-A89B-3F7291B0FE68}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:dataField pivotShowAs="rankDescending"/></ext></extLst></dataField></dataFields><formats count="1"><format dxfId="0"><pivotArea type="all"/></format></formats><pivotTableStyleInfo name="PivotStyleLight16" showRowHeaders="1" showColHeaders="1" showRowStripes="0" showColStripes="0" showLastColumn="1"/></pivotTableDefinition>"#;
/// An OLAP cache: its items carry member tuples, which this crate does not model.
const OLAP: &str = r#"<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cacheSource type="external" connectionId="1"/><cacheFields count="1"><cacheField name="[M]"><sharedItems><s v="a"><x v="0"/></s></sharedItems></cacheField></cacheFields></pivotCacheDefinition>"#;
const OLAP_TABLE: &str = r#"<pivotTableDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" name="Olap" cacheId="7"><location ref="H1:H2" firstHeaderRow="1" firstDataRow="1" firstDataCol="0"/></pivotTableDefinition>"#;

fn rels(items: &[(&str, &str, &str)]) -> String {
    let r: String = items.iter().map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{RT}/{ty}" Target="{t}"/>"#)).collect();
    format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{r}</Relationships>"#)
}

/// A workbook with a pivot cache (with records) whose table relates to it, and an OLAP cache
/// whose table is found by its `cacheId` only.
fn with_pivots() -> Vec<u8> {
    let book = format!(
        r#"<workbook xmlns="{MAIN}" xmlns:r="{RT}"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets><pivotCaches><pivotCache cacheId="5" r:id="rId3"/><pivotCache cacheId="7" r:id="rId4"/></pivotCaches></workbook>"#
    );
    let sheet = format!(
        r#"<worksheet xmlns="{MAIN}"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Region</t></is></c></row></sheetData></worksheet>"#
    );
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/pivotCache/pivotCacheDefinition1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"/><Override PartName="/xl/pivotCache/pivotCacheDefinition2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"/><Override PartName="/xl/pivotCache/pivotCacheRecords1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml"/><Override PartName="/xl/pivotTables/pivotTable1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml"/><Override PartName="/xl/pivotTables/pivotTable2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml"/></Types>"#;
    zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", rels(&[("rId1", "officeDocument", "xl/workbook.xml")]).as_bytes()),
        ("xl/workbook.xml", book.as_bytes()),
        (
            "xl/_rels/workbook.xml.rels",
            rels(&[
                ("rId1", "worksheet", "worksheets/sheet1.xml"),
                ("rId3", "pivotCacheDefinition", "pivotCache/pivotCacheDefinition1.xml"),
                ("rId4", "pivotCacheDefinition", "pivotCache/pivotCacheDefinition2.xml"),
            ])
            .as_bytes(),
        ),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            rels(&[("rId2", "pivotTable", "../pivotTables/pivotTable1.xml"), ("rId3", "pivotTable", "../pivotTables/pivotTable2.xml")]).as_bytes(),
        ),
        ("xl/pivotTables/pivotTable1.xml", TABLE.as_bytes()),
        (
            "xl/pivotTables/_rels/pivotTable1.xml.rels",
            rels(&[("rId1", "pivotCacheDefinition", "../pivotCache/pivotCacheDefinition1.xml")]).as_bytes(),
        ),
        ("xl/pivotTables/pivotTable2.xml", OLAP_TABLE.as_bytes()),
        ("xl/pivotCache/pivotCacheDefinition1.xml", CACHE.as_bytes()),
        ("xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels", rels(&[("rId1", "pivotCacheRecords", "pivotCacheRecords1.xml")]).as_bytes()),
        ("xl/pivotCache/pivotCacheRecords1.xml", RECORDS.as_bytes()),
        ("xl/pivotCache/pivotCacheDefinition2.xml", OLAP.as_bytes()),
    ])
}

#[test]
fn the_package_reads_caches_records_and_tables() {
    let bytes = with_pivots();
    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    let wb = p.workbook().unwrap();
    assert_eq!(wb.pivot_caches.iter().map(|c| (c.cache_id, c.rel_id.as_str())).collect::<Vec<_>>(), [(5, "rId3"), (7, "rId4")]);
    let def = p.pivot_cache("xl/pivotCache/pivotCacheDefinition1.xml").unwrap().unwrap();
    let ws = def.source.worksheet.as_ref().unwrap();
    assert_eq!((ws.reference.as_deref(), ws.sheet.as_deref()), (Some("A1:B3"), Some("Data")));
    assert_eq!((def.refresh_on_load, def.record_count, def.attrs.clone()), (Some(true), Some(2), vec![("xr:uid".to_string(), "{C}".to_string())]));
    let names: Vec<_> = def.fields.iter().map(|f| f.name.as_deref().unwrap_or("")).collect();
    assert_eq!(names, ["Region", "Day", "Ratio"]);
    let region = def.fields[0].shared_items.as_ref().unwrap();
    assert_eq!(region.items[1], CacheItem { value: CacheValue::Text("West".into()), attrs: vec![("u".into(), "1".into())] });
    let group = def.fields[1].group.as_ref().unwrap();
    assert_eq!((group.par, group.base, group.range.as_ref().and_then(|r| r.group_by.as_deref())), (Some(2), Some(1), Some("months")));
    assert_eq!(group.items.as_ref().map(|i| i.len()), Some(3));
    assert_eq!((def.fields[2].formula.as_deref(), def.fields[2].database_field), (Some("Day/2"), Some(false)));
    assert_eq!(def.other.len(), 1);

    let mut records = Vec::new();
    p.pivot_records("xl/pivotCache/pivotCacheDefinition1.xml", |r| {
        records.push(r);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        records,
        [
            vec![CacheValue::Index(0), CacheValue::Date("2024-01-01T00:00:00".into())],
            vec![CacheValue::Index(1), CacheValue::Date("2024-02-01T00:00:00".into())]
        ]
    );
    // The caller can stop the records.
    let stopped = p.pivot_records("xl/pivotCache/pivotCacheDefinition1.xml", |_| Err(Error::TooLarge("enough".into())));
    assert_eq!(stopped, Err(Error::TooLarge("enough".into())));

    assert!(p.pivot_cache("xl/pivotCache/pivotCacheDefinition2.xml").unwrap().is_none());
    assert!(p.warnings().iter().any(|w| w.contains("pivotCacheDefinition2")));

    let sheet = p.sheet(&wb.sheets[0], |_| Ok(())).unwrap();
    let tables = p.pivot_tables(&wb, &sheet).unwrap();
    assert_eq!(tables.len(), 2);
    let (part, t, cache) = &tables[0];
    assert_eq!((part.as_str(), cache.as_deref()), ("xl/pivotTables/pivotTable1.xml", Some("xl/pivotCache/pivotCacheDefinition1.xml")));
    assert_eq!(
        (t.name.as_deref(), t.cache_id, t.row_grand_totals, t.location.range.a1()),
        (Some("PivotTable1"), Some(5), Some(false), "D1:E4".into())
    );
    assert_eq!(t.row_fields, [0, -2]);
    let items = t.fields[0].items.as_ref().unwrap();
    assert_eq!((items[0].h, items[1].sd, items[2].item_type.as_deref()), (Some(true), Some(false), Some("default")));
    assert_eq!((t.fields[0].sort_type.as_deref(), t.fields[0].axis.as_deref()), (Some("descending"), Some("axisRow")));
    assert_eq!((t.page_fields[0].fld, t.page_fields[0].item), (Some(1), Some(0)));
    assert!(t.data_fields[0].ext.as_ref().is_some_and(|e| e.find("x14:dataField").is_some()));
    assert_eq!(t.style.as_ref().and_then(|s| s.name.as_deref()), Some("PivotStyleLight16"));
    assert_eq!(t.other.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["rowItems", "formats"]);
    // The second table names its cache by id only.
    assert_eq!(tables[1].2.as_deref(), Some("xl/pivotCache/pivotCacheDefinition2.xml"));
}

fn read(bytes: &[u8]) -> Document {
    Document::read(bytes, &Limits::default()).unwrap()
}

#[test]
fn pivots_round_trip() {
    let doc = read(&with_pivots());
    // The OLAP cache and its table are kept parts; the others are typed.
    assert_eq!(doc.pivot_caches.len(), 1);
    let ws = doc.worksheets[0].as_ref().unwrap();
    assert_eq!(ws.pivot_tables.len(), 1);
    let mut kept: Vec<&str> = doc.parts.iter().map(|p| p.name.as_str()).filter(|n| n.contains("pivot")).collect();
    kept.sort();
    assert_eq!(kept, ["xl/pivotCache/pivotCacheDefinition2.xml", "xl/pivotTables/pivotTable2.xml"]);
    let c = &doc.pivot_caches[0];
    assert_eq!((c.cache_id, c.records_part.as_str(), c.records.as_deref()), (5, "xl/pivotCache/pivotCacheRecords1.xml", Some(RECORDS.as_bytes())));

    let bytes = doc.write().unwrap();
    let again = read(&bytes);
    assert_eq!(again.pivot_caches, doc.pivot_caches);
    assert_eq!(again.worksheets[0].as_ref().unwrap().pivot_tables, ws.pivot_tables);
    assert_eq!(again.workbook.pivot_caches, doc.workbook.pivot_caches);
    assert_eq!(again.parts.iter().filter(|p| p.name.contains("pivot")).count(), 2);
    // The kept table still finds the kept cache by its id.
    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    let wb = p.workbook().unwrap();
    let sheet = p.sheet(&wb.sheets[0], |_| Ok(())).unwrap();
    let tables = p.pivot_tables(&wb, &sheet).unwrap();
    assert_eq!(
        tables.iter().map(|t| t.2.clone().unwrap_or_default()).collect::<Vec<_>>(),
        ["xl/pivotCache/pivotCacheDefinition1.xml", "xl/pivotCache/pivotCacheDefinition2.xml"]
    );
    assert_eq!(read(&again.write().unwrap()).pivot_caches, doc.pivot_caches);
}

#[test]
fn a_new_pivot_table() {
    let mut doc = read(&craft_xlsx::testing::workbook(&[("Data", r#"<row r="1"><c r="A1"><v>1</v></c></row>"#), ("Report", "")], &[]));
    let values = [CacheValue::Number(1.0), CacheValue::Number(2.0)];
    let definition = PivotCacheDefinition {
        refresh_on_load: Some(true),
        source: craft_xlsx::CacheSource {
            source_type: Some("worksheet".into()),
            worksheet: Some(craft_xlsx::WorksheetSource { reference: Some("A1:A3".into()), sheet: Some("Data".into()), ..Default::default() }),
            ..Default::default()
        },
        fields: vec![craft_xlsx::CacheField {
            name: Some("N".into()),
            num_fmt_id: Some(0),
            shared_items: Some(SharedItems { items: values.iter().cloned().map(CacheItem::new).collect(), ..SharedItems::describe(&values) }),
            ..Default::default()
        }],
        ..Default::default()
    };
    let records = records_xml(&[vec![CacheValue::Index(0)], vec![CacheValue::Index(1)]]);
    doc.pivot_caches.push(PivotCachePart { cache_id: 1, definition, records: Some(records), ..Default::default() });
    let table = PivotTableDefinition {
        name: Some("P".into()),
        cache_id: Some(1),
        location: Location { range: Range::parse("A3:B6").unwrap(), first_header_row: Some(1), ..Default::default() },
        fields: vec![PivotField {
            axis: Some("axisRow".into()),
            data_field: Some(true),
            items: Some(vec![PivotItem { x: Some(0), ..Default::default() }, PivotItem { x: Some(1), ..Default::default() }]),
            ..Default::default()
        }],
        row_fields: vec![0],
        data_fields: vec![DataField { name: Some("Sum of N".into()), fld: Some(0), ..Default::default() }],
        ..Default::default()
    };
    let ws = doc.worksheets[1].as_mut().unwrap();
    ws.pivot_tables.push(PivotTablePart { table: table.clone(), ..Default::default() });
    let back = read(&doc.write().unwrap());
    let c = &back.pivot_caches[0];
    assert_eq!((c.part.as_str(), c.records_part.as_str()), ("xl/pivotCache/pivotCacheDefinition1.xml", "xl/pivotCache/pivotCacheRecords1.xml"));
    assert_eq!(c.definition, doc.pivot_caches[0].definition);
    assert_eq!(c.definition.fields[0].shared_items.as_ref().map(|s| (s.contains_integer, s.min_value)), Some((Some(true), Some(1.0))));
    assert_eq!(back.workbook.pivot_caches.len(), 1);
    let t = &back.worksheets[1].as_ref().unwrap().pivot_tables[0];
    assert_eq!((t.part.as_str(), &t.table), ("xl/pivotTables/pivotTable1.xml", &table));

    // Removing the cache removes its relationship and entry; a table left naming it is refused.
    let mut gone = back.clone();
    gone.pivot_caches.clear();
    assert!(matches!(gone.write(), Err(Error::Invalid(_))));
    gone.worksheets[1].as_mut().unwrap().pivot_tables.clear();
    let after = read(&gone.write().unwrap());
    assert!(after.workbook.pivot_caches.is_empty() && after.pivot_caches.is_empty());
    assert!(!after.workbook_rels.list.iter().any(|r| r.kind() == "pivotCacheDefinition"));
    assert!(!after.worksheets[1].as_ref().unwrap().part.rels.list.iter().any(|r| r.kind() == "pivotTable"));

    // Two caches may not share an id.
    let mut twice = back.clone();
    twice.pivot_caches.push(PivotCachePart { part: String::new(), ..back.pivot_caches[0].clone() });
    assert!(matches!(twice.write(), Err(Error::Invalid(_))));
}
