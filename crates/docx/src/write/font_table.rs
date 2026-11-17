//! A font table (`word/fontTable.xml`, ECMA-376 Part 1 §17.8) for a document that brings none of
//! its own: one built in code, or read from another format. It names each font the document
//! uses with its generic family and pitch, so a reader without the font draws a like one (a sans
//! for Arial, not its own serif default). A table read from a file is written back as it was
//! instead ([`super::embed::EmbedWriter::font_table`]).

use std::collections::BTreeSet;

use craft_wordmodel::font_kind::GenericFamily;
use craft_wordmodel::props::CharProps;
use craft_wordmodel::symbol_fonts::is_symbol_font;
use craft_wordmodel::walk::each_para;
use craft_wordmodel::{Document, Paragraph};

use crate::xml::W;

/// Most fonts the table names (hostile input: a font per run).
const MAX_FONTS: usize = 1000;
/// Longest font name it keeps, bytes (as the reader, `MAX_FONT_NAME`).
const MAX_NAME: usize = 256;

/// Add the fonts `c` names to `out`, up to [`MAX_FONTS`].
fn take<'a>(c: &'a CharProps, out: &mut BTreeSet<&'a str>) {
    for f in [&c.font, &c.font_ea, &c.font_cs, &c.sym_font].into_iter().flatten() {
        let f = f.trim();
        if out.len() < MAX_FONTS && !f.is_empty() && f.len() <= MAX_NAME {
            out.insert(f);
        }
    }
}

/// The fonts `doc` names, sorted: the document defaults', its styles' and list levels', and
/// every run's and paragraph mark's in every story.
fn used_fonts(doc: &Document) -> BTreeSet<&str> {
    let mut out = BTreeSet::new();
    take(&doc.styles.default_chr, &mut out);
    for s in &doc.styles.styles {
        take(&s.chr, &mut out);
    }
    for a in &doc.numbering.abstracts {
        for l in &a.levels {
            take(&l.chr, &mut out);
        }
    }
    for n in &doc.numbering.nums {
        for (_, l) in &n.level_overrides {
            take(&l.chr, &mut out);
        }
    }
    let stories = std::iter::once(&doc.body).chain(doc.parts.values().map(|p| &p.blocks));
    for b in stories.flat_map(|blocks| blocks.iter()) {
        if out.len() >= MAX_FONTS {
            break;
        }
        each_para(b, 0, &mut |p: &Paragraph| {
            take(&p.mark, &mut out);
            for r in &p.runs {
                take(&r.props, &mut out);
            }
        });
    }
    out
}

/// `word/fontTable.xml` for `doc`, or `None` when it names no font.
pub(super) fn font_table_xml(doc: &Document) -> Option<Vec<u8>> {
    let fonts = used_fonts(doc);
    if fonts.is_empty() {
        return None;
    }
    let mut w = W::new();
    w.open(
        "w:fonts",
        &[
            ("xmlns:w", "http://schemas.openxmlformats.org/wordprocessingml/2006/main"),
            ("xmlns:r", "http://schemas.openxmlformats.org/officeDocument/2006/relationships"),
        ],
    );
    for name in fonts {
        w.open("w:font", &[("w:name", name)]);
        if is_symbol_font(name) {
            // Symbol, Wingdings…: the symbol character set, so a reader maps their codes as the
            // font's own and doesn't substitute a text face.
            w.val("w:charset", "02");
            w.val("w:family", "auto");
            w.val("w:pitch", "variable");
        } else {
            // A family the file's own table named roman stays roman (Word draws it in a serif).
            let family =
                if doc.settings.roman_fonts.iter().any(|r| r.eq_ignore_ascii_case(name)) { GenericFamily::Roman } else { GenericFamily::of(name) };
            w.val("w:family", family.name());
            w.val("w:pitch", if family.fixed_pitch() { "fixed" } else { "variable" });
        }
        w.close("w:font");
    }
    w.close("w:fonts");
    Some(w.into_bytes())
}
