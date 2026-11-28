#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// The code came from an app whose lints allow this.
#![allow(clippy::type_complexity)]
//! A small "flow" model every text format maps to and from: paragraphs with a kind (heading,
//! quote, code…), list membership, alignment and formatted inline spans; tables of cells holding
//! blocks. [`to_doc`] and [`from_doc`] convert between it and the full [`Document`].

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

use std::sync::Arc;

use craft_wordmodel::numbering::{AbstractNum, Level, ListKind, Num, levels_for};
use craft_wordmodel::para::{InlineObject, OBJ, PAGE_BREAK, RubyAlign, Run, ruby_raise};
use craft_wordmodel::props::{
    Border, BorderStyle, Borders, CellProps, CharProps, NumRef, ParaProps, PropChange, RowProps, TextColor, Underline, VAlign, VMerge, VertAlign,
};
use craft_wordmodel::section::NumFormat;
use craft_wordmodel::styles::{Style, StyleKind};
use craft_wordmodel::{Align, Block, Blocks, Document, Paragraph, Revision, RevisionKind, Rgb, Table, para_block};

mod color;
mod from_doc;
mod media;
pub mod odf_object;
mod to_doc;
mod types;
pub use types::continues_list;

pub use color::parse_color;
pub use from_doc::{
    ListCounter, flow_paras, from_doc, from_doc_tracked, from_doc_with, is_custom_char_style, is_custom_style, is_mono, para_kind, part_flow,
    tabs_from_indent,
};
pub use media::{base64_decode, base64_encode, data_uri, image_px, make_img, mime_of, percent_decode, sized_img, sniff_image};
pub use to_doc::{char_props, part_blocks, to_doc, to_doc_in};
pub use types::{
    Affix, BoxLook, CODE_STYLE, Cell, CellPic, Chain, Chart, Conn, FBlock, FComment, FTable, Flow, FlowOpts, Fmt, FmtChange, Group, Img, Inline,
    Kind, Label, ListInfo, MAX_DEPTH, MAX_FONT_NAME, MAX_GROUP_DEPTH, MAX_LINK, MONO_FONT, Member, Meta, OdfObject, Para, RUBY_BASE_SIZE, RevMark,
    Spacing, TableExtra, clean_text, equations_as_text, flatten_tables, kept_font, kept_link, ranged_anchors, ruby_as_text, ruby_object,
    shows_nothing,
};
pub use types::{Dropped, MAX_TABLE_CELLS, begin_import, dropped};

thread_local! {
    /// The kinds of [`Dropped`] already reported for the document being imported on this thread.
    /// Per thread because a document is parsed and built on one, and the parsers drop things
    /// from places that have no log of their own to hand.
    static REPORTED: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ASK and SET show nothing (layout shows no result for them): no text export writes the
    /// result they hold, from the document or from a flow with fields; ODT keeps it.
    #[test]
    fn ask_and_set_fields_export_no_text() {
        let mut doc = craft_wordmodel::Document::from_text("Before after");
        if let Ok(p) = doc.para_mut(craft_wordmodel::StoryRef::Body, &craft_wordmodel::Path::top(0)) {
            for instr in ["SET name other", "ASK other \"Prompt\""] {
                let field = craft_wordmodel::para::InlineObject::Field {
                    instr: instr.into(),
                    result: "SECRETVALUE".into(),
                    locked: false,
                    dirty: false,
                    code: false,
                    form: String::new(),
                };
                p.insert_object(7, field, &Default::default()).unwrap();
            }
        }
        let flow = from_doc_with(&doc, FlowOpts { fields: true, ..Default::default() });
        // The flow keeps the result for ODT.
        let kept = flow
            .blocks
            .iter()
            .any(|b| matches!(b, FBlock::Para(p) if p.inlines.iter().any(|i| matches!(i, Inline::Field { result, .. } if result == "SECRETVALUE"))));
        assert!(kept);
    }

    /// The default look's header row is an option of the table style: a table with no style (or a
    /// style the document doesn't have) has an ordinary first row, so RTF doesn't bold it or
    /// repeat it on each page, and HTML doesn't make it `th`. Repeat as header row still does.
    #[test]
    fn look_header_row_needs_a_table_style() {
        use craft_wordmodel::{Block, Document, table::Table};
        let first_row_header = |style: Option<&str>, repeat: bool| {
            let mut t = Table::new(2, 2, 200.0);
            t.props.style = style.map(str::to_string);
            assert!(t.props.look.header_row, "the default look has a header row");
            if let Some(r) = t.rows.first_mut() {
                r.props.header = repeat;
            }
            let mut doc = Document::new();
            doc.body = vec![std::sync::Arc::new(Block::Table(t))];
            let flow = from_doc(&doc);
            let Some(FBlock::Table(ft)) = flow.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table in {:?}", flow.blocks) };
            assert!(ft.rows.get(1).is_some_and(|r| r.iter().all(|c| !c.header)), "the second row is never a header");
            ft.rows.first().is_some_and(|r| r.iter().all(|c| c.header))
        };
        assert!(first_row_header(Some("TableGrid"), false), "a table style applies the look's header row");
        assert!(!first_row_header(None, false), "no style, no header row");
        assert!(!first_row_header(Some("NoSuchStyle"), false), "a missing style applies nothing");
        assert!(first_row_header(None, true), "Repeat as header row");
    }

    #[test]
    fn percent_decode_requires_hex_digits() {
        assert_eq!(percent_decode("%+A"), b"%+A".to_vec());
        assert_eq!(percent_decode("%-A"), b"%-A".to_vec());
        assert_eq!(percent_decode("%G1"), b"%G1".to_vec());
        assert_eq!(percent_decode("%41"), b"A".to_vec());
    }

    #[test]
    fn insert_covered_is_bounded_on_hostile_rowspan() {
        // Many rows each carrying the maximum rowspan would be O(rows × rowspan) covered cells;
        // the work cap must keep insertion bounded (and quick) instead of exhausting memory.
        let mut t = FTable {
            rows: (0..20_000).map(|_| vec![Cell { rowspan: 1000, ..Default::default() }]).collect(),
            widths: Vec::new(),
            borderless: false,
            extra: None,
            kept: None,
            wrap: None,
        };
        t.insert_covered();
        assert!(t.rows.iter().all(|r| r.len() <= 64), "a row grew past the column cap");
    }

    #[test]
    fn base64_round_trip() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7]] {
            let e = base64_encode(data);
            assert_eq!(base64_decode(&e).unwrap(), data, "{e}");
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert!(base64_decode("@@").is_none());
        assert_eq!(data_uri("data:text/plain,a%20b").unwrap(), b"a b");
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#f00"), Some(Rgb(255, 0, 0)));
        assert_eq!(parse_color("#f00f"), Some(Rgb(255, 0, 0)));
        assert_eq!(parse_color("#0080FF"), Some(Rgb(0, 128, 255)));
        assert_eq!(parse_color("#0080FFff"), Some(Rgb(0, 128, 255)));
        // A colour with alpha is composited over the white page it lands on (#0080ff at 50%).
        assert_eq!(parse_color("#0080ff80"), Some(Rgb(127, 191, 255)));
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some(Rgb(1, 2, 3)));
        assert_eq!(parse_color("rgb(1 2 3)"), Some(Rgb(1, 2, 3)));
        assert_eq!(parse_color("rgb(1 2 3 / 50%)"), Some(Rgb(128, 129, 129)));
        assert_eq!(parse_color("hsl(0, 100%, 50%)"), Some(Rgb(255, 0, 0)));
        assert_eq!(parse_color("hsl(120deg 100% 50%)"), Some(Rgb(0, 255, 0)));
        assert_eq!(parse_color("hsla(240, 100%, 50%, 0.8)"), Some(Rgb(51, 51, 255)));
        assert_eq!(parse_color("navy"), Some(Rgb(0, 0, 128)));
        assert_eq!(parse_color("#zz"), None);
        assert_eq!(parse_color("rgb(1)"), None);
    }

    #[test]
    fn list_labels() {
        let mut c = ListCounter::default();
        let o = |level| ListInfo { ordered: true, level, start: 1, ..Default::default() };
        assert_eq!(c.label(o(0)), "1.");
        assert_eq!(c.label(o(1)), "a.");
        assert_eq!(c.label(o(1)), "b.");
        assert_eq!(c.label(o(0)), "2.");
        assert_eq!(c.label(ListInfo { ordered: false, level: 0, start: 1, ..Default::default() }), "•");
    }

    #[test]
    fn covered_cells_inserted() {
        let mut t = FTable {
            rows: vec![vec![Cell { rowspan: 2, ..Default::default() }, Cell::default()], vec![Cell::default()]],
            widths: vec![],
            borderless: false,
            extra: None,
            kept: None,
            wrap: None,
        };
        t.insert_covered();
        assert_eq!(t.rows[1].len(), 2);
        assert!(t.rows[1][0].covered);
    }

    #[test]
    fn trim_drops_a_long_run_of_blank_spans_in_linear_time() {
        // Separately formatted line breaks don't merge; each used to be removed with
        // `Vec::remove(0)`, shifting all the rest (quadratic).
        let mut p = Para::default();
        let (b, i) = (Fmt { bold: true, ..Default::default() }, Fmt { italic: true, ..Default::default() });
        for _ in 0..200_000 {
            p.push_text("\n", &b);
            p.push_text(" ", &i);
        }
        p.push_text("  x ", &Fmt::default());
        let t = std::time::Instant::now();
        p.trim();
        // A fraction of a second; `Vec::remove(0)` per span ran past 6 minutes.
        assert!(t.elapsed() < std::time::Duration::from_secs(10), "{:?}", t.elapsed());
        assert_eq!(p.inlines, vec![Inline::Text("x".into(), Fmt::default())]);
        // The first span left keeps its formatting and loses only its leading blanks.
        let mut p = Para::default();
        p.push_text(" \t", &b);
        p.push_text(" a ", &i);
        p.push_text("b ", &b);
        p.trim();
        assert_eq!(p.inlines, vec![Inline::Text("a ".into(), i), Inline::Text("b".into(), b)]);
    }

    /// Paragraphs of `text` in order, each with its props set by `set`.
    fn doc_of(paras: &[(&str, &dyn Fn(&mut craft_wordmodel::ParaProps))]) -> craft_wordmodel::Document {
        let mut doc = craft_wordmodel::Document::new();
        doc.body = paras
            .iter()
            .map(|(t, set)| {
                let mut p = craft_wordmodel::Paragraph::with_text(t, Default::default());
                set(&mut p.props);
                std::sync::Arc::new(craft_wordmodel::Block::Para(p))
            })
            .collect();
        doc
    }

    /// The text and `item` of each flow paragraph, a table cell's included.
    fn items(blocks: &[FBlock]) -> Vec<(String, Option<u8>)> {
        let mut out = Vec::new();
        for b in blocks {
            match b {
                FBlock::Para(p) => out.push((p.text(), p.item)),
                FBlock::Table(t) => {
                    for c in t.rows.iter().flatten() {
                        out.extend(items(&c.blocks));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn character_indents_set_the_list_continuation_baseline() {
        let mut doc = craft_wordmodel::Document::new();
        let num = std::sync::Arc::make_mut(&mut doc.numbering).add_list(craft_wordmodel::ListKind::Numbered);
        let numbered = move |p: &mut craft_wordmodel::ParaProps| {
            p.numbering = Some(NumRef { num, level: 0 });
            p.indent_left = Some(0.0);
            p.indent_left_chars = Some(6.0);
        };
        let outdented = |p: &mut craft_wordmodel::ParaProps| {
            p.list_item = Some(0);
            p.indent_left = Some(0.0);
        };
        doc.body = doc_of(&[("One", &numbered), ("Out", &outdented)]).body;
        let got = items(&from_doc(&doc).blocks);
        assert_eq!(got.iter().find(|r| r.0 == "Out").map(|r| r.1), Some(None));
    }

    #[test]
    fn a_zero_indent_header_keeps_its_explicit_list_membership() {
        let header = |p: &mut craft_wordmodel::ParaProps| {
            p.list_item = Some(0);
            p.indent_left = Some(0.0);
        };
        let doc = doc_of(&[("Header", &header)]);
        assert_eq!(items(&from_doc(&doc).blocks), [("Header".into(), Some(0))]);
    }

    /// A list item's further paragraph cut by a page break is that paragraph on both pages: the
    /// text after the break stays in the item too.
    #[test]
    fn a_further_paragraph_stays_in_its_item_past_a_page_break() {
        let cont = |p: &mut craft_wordmodel::ParaProps| {
            p.list_item = Some(0);
            p.indent_left = Some(36.0);
        };
        let doc = doc_of(&[("Before\u{c}After", &cont)]);
        let got = items(&from_doc(&doc).blocks);
        assert_eq!(got, [("Before".to_string(), Some(0)), ("After".to_string(), Some(0))], "{got:?}");
    }

    /// Body text ends the lists before it: a list starting with a header (no number) at its text
    /// is a list's paragraph however far an earlier list's text was indented.
    #[test]
    fn a_list_header_after_body_text_is_in_its_list() {
        let mut doc = craft_wordmodel::Document::new();
        let num = std::sync::Arc::make_mut(&mut doc.numbering).add_list(craft_wordmodel::ListKind::Numbered);
        let numbered = move |p: &mut craft_wordmodel::ParaProps| {
            p.numbering = Some(NumRef { num, level: 0 });
            p.indent_left = Some(72.0);
        };
        let header = |p: &mut craft_wordmodel::ParaProps| {
            p.list_item = Some(0);
            p.indent_left = Some(36.0);
        };
        let numbered_36 = move |p: &mut craft_wordmodel::ParaProps| {
            p.numbering = Some(NumRef { num, level: 0 });
            p.indent_left = Some(36.0);
        };
        let body = doc_of(&[("One", &numbered), ("Body", &|_| {}), ("Header", &header), ("Two", &numbered_36)]).body;
        doc.body = body;
        let got = items(&from_doc(&doc).blocks);
        assert_eq!(got.iter().find(|r| r.0 == "Header").map(|r| r.1), Some(Some(0)), "{got:?}");
    }

    /// A further paragraph moved out of its item (to the margin) is body text: a list starting with
    /// a header after it, less indented than the list before, keeps the header in its list.
    #[test]
    fn a_list_header_after_an_outdented_further_paragraph_is_in_its_list() {
        let mut doc = craft_wordmodel::Document::new();
        let num = std::sync::Arc::make_mut(&mut doc.numbering).add_list(craft_wordmodel::ListKind::Numbered);
        let numbered = move |p: &mut craft_wordmodel::ParaProps| {
            p.numbering = Some(NumRef { num, level: 0 });
            p.indent_left = Some(72.0);
        };
        let outdented = |p: &mut craft_wordmodel::ParaProps| {
            p.list_item = Some(0);
            p.indent_left = Some(0.0);
        };
        let header = |p: &mut craft_wordmodel::ParaProps| {
            p.list_item = Some(0);
            p.indent_left = Some(36.0);
        };
        let numbered_36 = move |p: &mut craft_wordmodel::ParaProps| {
            p.numbering = Some(NumRef { num, level: 0 });
            p.indent_left = Some(36.0);
        };
        let body = doc_of(&[("One", &numbered), ("Out", &outdented), ("Header", &header), ("Two", &numbered_36)]).body;
        doc.body = body;
        let got = items(&from_doc(&doc).blocks);
        assert_eq!(got.iter().find(|r| r.0 == "Out").map(|r| r.1), Some(None), "{got:?}");
        assert_eq!(got.iter().find(|r| r.0 == "Header").map(|r| r.1), Some(Some(0)), "{got:?}");
    }

    /// A list item's further paragraph in a table cell a table style formats stays one.
    #[test]
    fn a_further_paragraph_in_a_styled_cell_stays_in_its_item() {
        let cont = |p: &mut craft_wordmodel::ParaProps| {
            p.list_item = Some(0);
            p.indent_left = Some(36.0);
        };
        let mut doc = craft_wordmodel::Document::new();
        // Table Grid: paragraph formatting of its own (no space after, single spacing).
        let mut t = craft_wordmodel::table::Table::new(1, 1, 200.0);
        if let Some(c) = t.rows.first_mut().and_then(|r| r.cells.first_mut()) {
            c.blocks = doc_of(&[("Cont", &cont)]).body;
        }
        doc.body = vec![
            std::sync::Arc::new(craft_wordmodel::Block::Table(t)),
            std::sync::Arc::new(craft_wordmodel::Block::Para(craft_wordmodel::Paragraph::new())),
        ];
        let got = items(&from_doc_with(&doc, FlowOpts { paras: true, ..Default::default() }).blocks);
        assert_eq!(got.iter().find(|r| r.0 == "Cont").map(|r| r.1), Some(Some(0)), "{got:?}");
    }
}
