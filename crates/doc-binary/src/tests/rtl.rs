//! Synthetic regressions ported from WordCraft #5938.

use super::*;

/// Check the imported model, without depending on new decoder fields or test-only accessors.
fn table_props_from_sprms(prls: &[(u16, &[u8])]) -> craft_wordmodel::props::TableProps {
    let papx = [row_end_mark(false, true), grpprl(prls)].concat();
    let doc = read(&direction_split_doc("a\u{7}", vec![(0, 0, papx)])).expect("opens");
    doc.body.iter().find_map(|b| b.as_table()).expect("table").props.clone()
}

#[test]
fn right_to_left_paragraphs_runs_tables_and_sections_imported() {
    // An Arabic document from Word: sprmPFBiDi on its paragraphs, sprmCFBiDi on its runs,
    // sprmTFBiDi on its tables and sprmSFBiDi on its section. The table carries only the
    // physical sprmTJc90 = right, which in a right-to-left table is its start (logical left).
    use craft_wordmodel::props::Align;
    let text = "a\u{7}\u{7}text\r";
    let in_tbl = grpprl(&[(0x2416, &[0x01]), (0x6649, &[1, 0, 0, 0])]);
    let ttp = [
        grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01]), (0x6649, &[1, 0, 0, 0]), (0x560B, &1u16.to_le_bytes()), (0x5400, &2u16.to_le_bytes())]),
        t_def_edges(&[0, 1440]),
    ]
    .concat();
    // Word writes the physical sprmPJc80 and then the logical sprmPJc: physically right is the
    // start of a right-to-left paragraph.
    let rtl_para = grpprl(&[(0x2403, &[0x02]), (0x2461, &[0x00]), (0x2441, &[0x01])]);
    let spec = StructSpec {
        text,
        header_stories: vec![],
        sections: vec![(text.chars().count() as u32, grpprl(&[(0x3228, &[0x01])]))],
        chpx_runs: vec![(0, Vec::new()), (3, grpprl(&[(0x085A, &[0x01])]))],
        papx_entries: vec![(0, 0, in_tbl), (2, 0, ttp), (3, 0, rtl_para)],
        plf_lst: Vec::new(),
        plf_lfo: Vec::new(),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let table = doc.body.iter().find_map(|b| b.as_table()).expect("a table");
    assert!(table.props.bidi_visual, "sprmTFBiDi");
    assert_eq!(table.props.align, Some(Align::Left), "physically right in a right-to-left table is its start");
    let p = doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == "text").expect("the paragraph");
    assert_eq!(p.props.bidi, Some(true), "sprmPFBiDi");
    assert_eq!(p.props.align, Some(Align::Left), "the logical sprmPJc wins over the physical sprmPJc80");
    assert!(p.runs.iter().all(|r| r.props.rtl == Some(true)), "sprmCFBiDi: {:?}", p.runs);
    assert!(doc.last_section.rtl, "sprmSFBiDi");
}

#[test]
fn a_logical_table_justification_wins_over_the_physical_one() {
    // sprmTJc (logical) names the start or end whatever the direction; sprmTJc90 is physical.
    use craft_wordmodel::props::Align;
    let row = table_props_from_sprms;
    let (bidi, right, left) = (1u16.to_le_bytes(), 2u16.to_le_bytes(), 0u16.to_le_bytes());
    assert_eq!(row(&[(0x5400, &right), (0x548A, &left)]).align, Some(Align::Left));
    assert_eq!(row(&[(0x548A, &left), (0x5400, &right)]).align, Some(Align::Left), "logical wins even before physical");
    assert_eq!(row(&[(0x548A, &right), (0x560B, &bidi), (0x5400, &right)]).align, Some(Align::Right));
    assert_eq!(row(&[(0x5664, &bidi), (0x5400, &left)]).align, Some(Align::Right), "sprmTFBiDi90, physically left");
    assert_eq!(row(&[(0x5400, &right)]).align, Some(Align::Right), "left to right: physical is logical");
}

#[test]
fn either_table_direction_sprm_makes_the_table_right_to_left() {
    // [MS-DOC] §2.6.3: a table is right-to-left if either sprmTFBiDi or sprmTFBiDi90 is true.
    // Each sprm keeps its own last value, so a later false clears an earlier true of the same
    // sprm, but not of the other one.
    let row = |prls: &[(u16, &[u8])]| table_props_from_sprms(prls).bidi_visual;
    let (on, off) = (1u16.to_le_bytes(), 0u16.to_le_bytes());
    assert!(row(&[(0x5664, &on), (0x560B, &off)]), "sprmTFBiDi90 stays true");
    assert!(row(&[(0x560B, &on), (0x5664, &off)]), "sprmTFBiDi stays true");
    assert!(row(&[(0x560B, &off), (0x5664, &on)]));
    assert!(row(&[(0x5664, &on)]));
    assert!(!row(&[(0x560B, &on), (0x560B, &off)]), "a later false clears the same sprm");
    assert!(!row(&[(0x5664, &on), (0x5664, &off)]));
    assert!(!row(&[(0x560B, &off), (0x5664, &off)]));
}

#[test]
fn kashida_justification_is_justified() {
    // Arabic Word documents justify with kashida: sprmPJc 8 (low), 5 (medium), 7 (high), after
    // the physical sprmPJc80 = 3. They are justified, not start-aligned, as the DOCX reader
    // reads lowKashida/mediumKashida/highKashida; 9 (Thai distribution) is distributed.
    use craft_wordmodel::props::Align;
    let spec = RichSpec {
        text: "low\rmed\rhigh\rthai\r",
        styles: vec![(1, 0x0FFF, "Normal", grpprl(&[(0x2441, &[0x01])]), Vec::new())],
        fonts: vec!["Times New Roman"],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![
            (3, 0, grpprl(&[(0x2403, &[0x03]), (0x2461, &[0x08])])),
            (7, 0, grpprl(&[(0x2403, &[0x03]), (0x2461, &[0x05])])),
            (12, 0, grpprl(&[(0x2403, &[0x03]), (0x2461, &[0x07])])),
            (17, 0, grpprl(&[(0x2461, &[0x09])])),
        ],
    };
    let doc = read(&rich_doc(&spec)).expect("opens");
    let align = |t: &str| doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == t).expect("paragraph").props.align;
    assert_eq!(align("low"), Some(Align::Justify));
    assert_eq!(align("med"), Some(Align::Justify));
    assert_eq!(align("high"), Some(Align::Justify));
    assert_eq!(align("thai"), Some(Align::Distribute));
}

#[test]
fn a_kept_run_direction_is_the_styles() {
    // ToggleOperand 0x80 is the style's value: a PRM's sprmCFBiDi = 0x80 after the CHPX's
    // sprmCFBiDi = 0 makes a run of a right-to-left style right to left again.
    use craft_wordmodel::props::CharProps;
    for rtl in [Some(true), Some(false), None] {
        let base = CharProps { rtl, ..CharProps::default() };
        let mut p = base.clone();
        for (operand, expected) in [(0x00, Some(false)), (0x80, rtl), (0x01, Some(true)), (0x80, rtl), (0x81, Some(!rtl.unwrap_or(false)))] {
            for prl in crate::sprm::iter(&grpprl(&[(0x085A, &[operand])])) {
                crate::fmt::apply_char(&mut p, &prl, &base, &[]);
            }
            assert_eq!(p.rtl, expected, "style {rtl:?}, operand {operand:#04x}");
        }
    }
}

#[test]
fn physical_paragraph_indents_and_alignment_turn_with_the_direction() {
    // [MS-DOC] §2.6.2: sprmPDxaLeft80/Right80 and sprmPJc80 are physical, sprmPDxaLeft/Right
    // and sprmPJc logical. Normal reads right to left (sprmPFBiDi in the style), as in an Arabic
    // Word 2003 file (#6384): its centred author line stores one 3.5" indent on the page's right
    // both ways, Right80 = 5040 and Left = 5040. Read as two indents, the line had no width left.
    use craft_wordmodel::props::Align;
    let tw = |v: i16| v.to_le_bytes();
    let spec = RichSpec {
        text: "name\rphys\rltr\r",
        styles: vec![
            (1, 0x0FFF, "Normal", grpprl(&[(0x2441, &[0x01])]), Vec::new()),
            // Based on Normal, physical only: the page's left is its end.
            (1, 0, "Indented", grpprl(&[(0x840F, &tw(720))]), Vec::new()),
        ],
        fonts: vec!["Times New Roman"],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![
            (4, 0, grpprl(&[(0x2403, &[0x01]), (0x840E, &tw(5040)), (0x840F, &tw(0)), (0x845E, &tw(5040)), (0x2461, &[0x01])])),
            // Physical only, right to left from the style: right is the start, left the end.
            (9, 0, grpprl(&[(0x840E, &tw(720)), (0x840F, &tw(1440)), (0x2403, &[0x00])])),
            // The paragraph's own sprmPFBiDi = 0, after the physical sprms, makes them logical.
            (13, 0, grpprl(&[(0x840E, &tw(720)), (0x2403, &[0x02]), (0x2441, &[0x00])])),
        ],
    };
    let doc = read(&rich_doc(&spec)).expect("opens");
    let para = |t: &str| doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == t).expect("paragraph");
    let name = para("name");
    assert_eq!((name.props.indent_left, name.props.indent_right), (Some(252.0), Some(0.0)), "one start indent, as Word writes it twice");
    assert_eq!(name.props.align, Some(Align::Center));
    let phys = para("phys");
    assert_eq!((phys.props.indent_left, phys.props.indent_right), (Some(36.0), Some(72.0)), "physical right is the start");
    assert_eq!(phys.props.align, Some(Align::Right), "physically left is the end of a right-to-left paragraph");
    let ltr = para("ltr");
    assert_eq!((ltr.props.indent_left, ltr.props.indent_right), (None, Some(36.0)), "left to right: physical is logical");
    assert_eq!(ltr.props.align, Some(Align::Right));
    let indented = doc.styles.styles.iter().find(|s| s.name == "Indented").expect("style");
    assert_eq!((indented.para.indent_left, indented.para.indent_right), (None, Some(36.0)), "a style's direction comes from its base");
}

#[test]
fn logical_paragraph_sides_win_in_either_order_with_a_derived_style() {
    use craft_wordmodel::props::Align;
    let physical = grpprl(&[(0x840F, &1440i16.to_le_bytes()), (0x840E, &2880i16.to_le_bytes()), (0x2403, &[0x02])]);
    let logical = grpprl(&[(0x845E, &720i16.to_le_bytes()), (0x845D, &360i16.to_le_bytes()), (0x2461, &[0x02])]);
    for (papx, has_logical) in [([physical.clone(), logical.clone()].concat(), true), ([logical, physical.clone()].concat(), true), (physical, false)]
    {
        let spec = RichSpec {
            text: "text\r",
            styles: vec![
                (1, 0x0FFF, "Normal", grpprl(&[(0x2441, &[0x01])]), Vec::new()),
                (1, 0x0FFF, "", Vec::new(), Vec::new()),
                (1, 0, "Derived", Vec::new(), Vec::new()),
            ],
            fonts: vec![],
            chpx_runs: vec![(0, Vec::new())],
            papx_entries: vec![(0, 2, papx)],
        };
        let doc = read(&rich_doc(&spec)).expect("opens");
        let p = para0(&doc);
        assert_eq!(p.props.style.as_deref(), Some("Derived"));
        let expected = if has_logical { (Some(36.0), Some(18.0), Some(Align::Right)) } else { (Some(144.0), Some(72.0), Some(Align::Left)) };
        assert_eq!((p.props.indent_left, p.props.indent_right, p.props.align), expected);
    }
}

#[test]
fn rows_of_different_directions_are_different_tables() {
    // [MS-DOC] §2.4.3: adjacent rows that differ in sprmTFBiDi belong to different tables, so a
    // right-to-left row doesn't turn the left-to-right table above it right to left.
    let text = "a\u{7}\u{7}b\u{7}\u{7}c\u{7}\u{7}tail\r";
    let in_tbl = grpprl(&[(0x2416, &[0x01]), (0x6649, &[1, 0, 0, 0])]);
    let ttp = |bidi: bool| {
        let mut prls = grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01]), (0x6649, &[1, 0, 0, 0])]);
        if bidi {
            prls.extend(grpprl(&[(0x560B, &1u16.to_le_bytes())]));
        }
        [prls, t_def_edges(&[0, 1440])].concat()
    };
    let spec = StructSpec {
        text,
        header_stories: vec![],
        sections: vec![(text.chars().count() as u32, Vec::new())],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries: vec![
            (0, 0, in_tbl.clone()),
            (2, 0, ttp(false)),
            (3, 0, in_tbl.clone()),
            (5, 0, ttp(true)),
            (6, 0, in_tbl),
            (8, 0, ttp(true)),
            (9, 0, Vec::new()),
        ],
        plf_lst: Vec::new(),
        plf_lfo: Vec::new(),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    };
    let doc = read(&struct_doc(&spec)).expect("opens");
    let tables: Vec<_> = doc.body.iter().filter_map(|b| b.as_table()).collect();
    let shape: Vec<_> = tables.iter().map(|t| (t.props.bidi_visual, t.rows.len())).collect();
    assert_eq!(shape, [(false, 1), (true, 2)], "one left-to-right row, then two right-to-left rows");
    assert_eq!(tables[1].grid, vec![72.0], "the second table has its own grid");
}

/// A body of `.doc` table paragraphs for the direction-split tests: `text` and the PAPX runs
/// (start CP, PAPX) that mark it up.
fn direction_split_doc(text: &'static str, papx_entries: Vec<(u32, u16, Vec<u8>)>) -> Vec<u8> {
    struct_doc(&StructSpec {
        text,
        header_stories: vec![],
        sections: vec![(text.chars().count() as u32, Vec::new())],
        chpx_runs: vec![(0, Vec::new())],
        papx_entries,
        plf_lst: Vec::new(),
        plf_lfo: Vec::new(),
        note_stories: vec![],
        endnote_stories: vec![],
        extra_pairs: Vec::new(),
        data_stream: Vec::new(),
    })
}

/// Each table's rows as cell texts.
fn table_cell_texts(doc: &craft_wordmodel::Document) -> Vec<Vec<Vec<String>>> {
    let cell_text = |c: &craft_wordmodel::Cell| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect::<String>();
    let row_texts = |r: &craft_wordmodel::Row| r.cells.iter().map(cell_text).collect::<Vec<_>>();
    doc.body.iter().filter_map(|b| b.as_table()).map(|t| t.rows.iter().map(row_texts).collect()).collect()
}

/// sprmPFInTable + sprmPItap at `depth`.
fn in_table_at(depth: u8) -> Vec<u8> {
    grpprl(&[(0x2416, &[0x01]), (0x6649, &[depth, 0, 0, 0])])
}

/// A row-end mark (sprmPFTtp), right to left when `bidi`, with a one-column `sprmTDefTable`
/// when `def_table`.
fn row_end_mark(bidi: bool, def_table: bool) -> Vec<u8> {
    let mut prls = grpprl(&[(0x2416, &[0x01]), (0x2417, &[0x01]), (0x6649, &[1, 0, 0, 0])]);
    if bidi {
        prls.extend(grpprl(&[(0x560B, &1u16.to_le_bytes())]));
    }
    if def_table {
        prls.extend(t_def_edges(&[0, 1440]));
    }
    prls
}

#[test]
fn direction_split_keeps_the_earlier_table_when_the_new_row_nests_a_table() {
    // The right-to-left row holds a nested (depth 2) paragraph before its row-end mark. The
    // split closes the left-to-right row above at its own depth, so it stays a table instead
    // of degrading to paragraphs because of the nesting in the row that follows it.
    let text = "a\u{7}\u{7}n\rb\u{7}\u{7}tail\r";
    let doc = read(&direction_split_doc(
        text,
        vec![
            (0, 0, in_table_at(1)),
            (2, 0, row_end_mark(false, true)),
            (3, 0, in_table_at(2)),
            (5, 0, in_table_at(1)),
            (7, 0, row_end_mark(true, true)),
            (8, 0, Vec::new()),
        ],
    ))
    .expect("opens");
    let first = doc.body.first().and_then(|b| b.as_table()).expect("the left-to-right row is still a table");
    assert!(!first.props.bidi_visual);
    assert_eq!(table_cell_texts(&doc).first(), Some(&vec![vec!["a".to_string()]]));
    // The nested row is deeper than the reader models: its text survives as paragraphs.
    let paras: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect();
    assert_eq!(paras, ["n", "b", "tail"]);
}

#[test]
fn nested_right_to_left_table_does_not_turn_its_outer_table() {
    // A left-to-right table whose second row nests a right-to-left table. The nested row's
    // end (a depth-2 paragraph mark with sprmPFInnerTtp) carries that table's sprmTFBiDi; the
    // outer table's direction comes from its own (depth-1) row-end marks only, so the outer
    // rows are never split apart by direction nor mirrored.
    let text = "a\u{7}\u{7}x\r\r\u{7}\u{7}tail\r";
    let inner_row_end = grpprl(&[(0x2416, &[0x01]), (0x6649, &[2, 0, 0, 0]), (0x244B, &[0x01]), (0x244C, &[0x01]), (0x560B, &1u16.to_le_bytes())]);
    let doc = read(&direction_split_doc(
        text,
        vec![
            (0, 0, in_table_at(1)),
            (2, 0, row_end_mark(false, true)),
            (3, 0, in_table_at(2)),
            (5, 0, inner_row_end),
            (6, 0, in_table_at(1)),
            (7, 0, row_end_mark(false, true)),
            (8, 0, Vec::new()),
        ],
    ))
    .expect("opens");
    let bidi: Vec<_> = doc.body.iter().filter_map(|b| b.as_table()).map(|t| t.props.bidi_visual).collect();
    assert!(bidi.iter().all(|b| !b), "no outer table reads right to left: {bidi:?}");
    // The nesting is deeper than the reader models: the whole outer table degrades to its
    // paragraphs, as it did before the direction split, and no text is lost (the nested
    // row's end mark is an empty paragraph of its own).
    let paras: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect();
    assert_eq!(paras, ["a", "x", "", "tail"]);
}

#[test]
fn direction_split_with_content_on_the_row_end_mark() {
    // The right-to-left row's row-end mark carries text of its own: that text is a cell of the
    // new table, and the left-to-right table above keeps only its own cell.
    let text = "a\u{7}\u{7}b\u{7}c\u{7}tail\r";
    let doc = read(&direction_split_doc(
        text,
        vec![(0, 0, in_table_at(1)), (2, 0, row_end_mark(false, true)), (3, 0, in_table_at(1)), (5, 0, row_end_mark(true, true)), (7, 0, Vec::new())],
    ))
    .expect("opens");
    let bidi: Vec<_> = doc.body.iter().filter_map(|b| b.as_table()).map(|t| t.props.bidi_visual).collect();
    assert_eq!(bidi, [false, true]);
    assert_eq!(table_cell_texts(&doc), vec![vec![vec!["a".to_string()]], vec![vec!["b".to_string(), "c".to_string()]]]);
}

#[test]
fn direction_split_on_a_row_without_a_def_table() {
    // The right-to-left row has no sprmTDefTable: the split still gives it its own table (with
    // no grid inherited from the table above), and each cell stays with its row.
    let text = "a\u{7}\u{7}b\u{7}\u{7}tail\r";
    let doc = read(&direction_split_doc(
        text,
        vec![
            (0, 0, in_table_at(1)),
            (2, 0, row_end_mark(false, true)),
            (3, 0, in_table_at(1)),
            (5, 0, row_end_mark(true, false)),
            (6, 0, Vec::new()),
        ],
    ))
    .expect("opens");
    let tables: Vec<_> = doc.body.iter().filter_map(|b| b.as_table()).collect();
    let shape: Vec<_> = tables.iter().map(|t| (t.props.bidi_visual, t.grid.clone())).collect();
    assert_eq!(shape, [(false, vec![72.0]), (true, vec![])]);
    // Without a sprmTDefTable the row-end mark always ends a cell (the column count is unknown).
    assert_eq!(table_cell_texts(&doc), vec![vec![vec!["a".to_string()]], vec![vec!["b".to_string(), String::new()]]]);
}

#[test]
fn a_run_toggle_inverts_its_paragraph_styles_value() {
    // [MS-DOC] §2.6.1: a ToggleOperand of 0x81 is the opposite of the style's value. The text
    // runs of a paragraph were formatted before its style was known (the style comes from the
    // PAPX at the paragraph mark), so 0x81 inverted the default instead: a run inverting the
    // direction of a right-to-left style came out right to left, and a run inverting a bold
    // style came out bold. The runs keep only their own formatting; the style stays the style.
    let style_chpx = grpprl(&[(0x085A, &[0x01]), (0x0835, &[0x01])]);
    let spec = RichSpec {
        text: "ab\rcd\r",
        styles: vec![(1, 0x0FFF, "Normal", Vec::new(), Vec::new()), (1, 0, "Arabic Bold", grpprl(&[(0x2441, &[0x01])]), style_chpx)],
        fonts: vec!["Times New Roman"],
        chpx_runs: vec![(0, grpprl(&[(0x085A, &[0x81]), (0x0835, &[0x81])])), (1, Vec::new()), (3, grpprl(&[(0x085A, &[0x81])]))],
        papx_entries: vec![(2, 1, Vec::new()), (5, 0, Vec::new())],
    };
    let doc = read(&rich_doc(&spec)).expect("opens");
    let paras: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).collect();
    let [styled, normal] = &paras[..] else { panic!("{paras:?}") };
    assert_eq!(styled.props.style.as_deref(), Some("Arabic Bold"));
    let first = &styled.runs[0].props;
    assert_eq!((first.rtl, first.bold), (Some(false), Some(false)), "0x81 inverts the style's right to left and bold");
    let second = &styled.runs[1].props;
    assert_eq!((second.rtl, second.bold, second.font.as_deref()), (None, None, None), "a run without sprms keeps only its own formatting");
    // Under Normal (left to right), 0x81 still makes the run right to left.
    assert_eq!(normal.runs[0].props.rtl, Some(true));
}
