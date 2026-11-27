//! `TextField::color_ranges`: the ranges an app gives take theme inks that read on the field; colouring changes no
//! layout; typing in a coloured field edits it and colours the new text; a text past the limit, and a range past the
//! text, are not coloured.

use craft_theme::{Contrast, Mode, Rgba, contrast_ratio, text_contrast_min};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::primitives::{COLORED_TEXT_LIMIT, ColorRange, RangeInk};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Color32, Key, Modifiers, Shape};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Every ink, in the order [`INKED`] names its words.
const INKS: [RangeInk; 6] = [RangeInk::Muted, RangeInk::Accent, RangeInk::Good, RangeInk::Warn, RangeInk::Bad, RangeInk::Info];

/// One word per ink, then a plain one.
const INKED: &str = "muted accent good warn bad info plain";

/// The ranges of every `var` in `text`, in the accent ink.
fn vars(text: &str) -> Vec<ColorRange> {
    text.match_indices("var").map(|(at, word)| ColorRange::new(at..at + word.len(), RangeInk::Accent)).collect()
}

/// The text shapes among `shapes`, those inside a `Shape::Vec` included.
fn text_shapes<'a>(shapes: &'a [Shape], out: &mut Vec<&'a egui::epaint::TextShape>) {
    for shape in shapes {
        match shape {
            Shape::Vec(shapes) => text_shapes(shapes, out),
            Shape::Text(text) => out.push(text),
            _ => {}
        }
    }
}

/// The galley of the field's text, found by a piece of it.
fn galley_with(fixture: &mut Fixture, piece: &str) -> std::sync::Arc<egui::Galley> {
    let shapes = fixture.frame_shapes();
    let mut texts = Vec::new();
    text_shapes(&shapes, &mut texts);
    texts
        .into_iter()
        .find(|text| text.galley.job.text.contains(piece))
        .map(|text| text.galley.clone())
        .unwrap_or_else(|| panic!("the field paints its text holding {piece:?}"))
}

/// The text of each section of `galley`, with its colour.
fn sections(galley: &egui::Galley) -> Vec<(String, Color32)> {
    let text = &galley.job.text;
    galley
        .job
        .sections
        .iter()
        .map(|section| (text.get(section.byte_range.start.0..section.byte_range.end.0).unwrap_or_default().to_owned(), section.format.color))
        .collect()
}

/// The colour of each byte of the first `piece` in `galley`'s text, when they share one. Sections of one format merge,
/// so a range is found by its bytes, not as a section of its own.
fn color_of(galley: &egui::Galley, piece: &str) -> Option<Color32> {
    let start = galley.job.text.find(piece)?;
    let mut colors = galley.job.sections.iter().filter_map(|section| {
        let range = section.byte_range.start.0..section.byte_range.end.0;
        (range.start < start + piece.len() && start < range.end).then_some(section.format.color)
    });
    let first = colors.next()?;
    colors.all(|color| color == first).then_some(first)
}

/// A fixture showing a three-line monospace field over `text`, coloured by `ranges` when given; the field's fill is
/// written to `fill` each frame.
fn field(text: &str, ranges: Option<fn(&str) -> Vec<ColorRange>>, fill: Rc<Cell<Rgba>>) -> Fixture {
    let value = Rc::new(RefCell::new(text.to_owned()));
    Fixture::new(move |ui| {
        fill.set(Env::of(ui).tokens.tokens().color.surface.sunken);
        let mut text = value.borrow_mut();
        let mut field = TextField::new("Script", Bind::bound(&mut *text)).lines(3).mono();
        if let Some(ranges) = ranges {
            field = field.color_ranges(ranges);
        }
        field.show(ui);
    })
}

/// Each word in its own ink: the ink is a colour of its own, and keeps the text contrast minimum on the field's fill in
/// every built-in theme, both modes and both contrasts, the status inks included, which the theme holds to it only on panels.
#[trial]
fn ranges_take_inks_that_read_on_the_field_in_every_theme() {
    fn inked(text: &str) -> Vec<ColorRange> {
        let (mut at, mut out) = (0, Vec::new());
        for (word, ink) in text.split(' ').zip(INKS) {
            out.push(ColorRange::new(at..at + word.len(), ink));
            at += word.len() + 1;
        }
        out
    }
    let modes = [(Mode::Light, Contrast::Standard), (Mode::Dark, Contrast::Standard), (Mode::Light, Contrast::High), (Mode::Dark, Contrast::High)];
    for (theme, (mode, contrast)) in craft_theme::builtins().iter().flat_map(|theme| modes.map(|mode| (theme.id.clone(), mode))) {
        let fill = Rc::new(Cell::new(Rgba::BLACK));
        let mut fixture = field(INKED, Some(inked), fill.clone());
        fixture.theme_id(theme.clone()).theme(mode, contrast).settle();
        let galley = galley_with(&mut fixture, "plain");
        let plain = color_of(&galley, "plain");
        for (word, ink) in INKED.split(' ').zip(INKS) {
            let color =
                color_of(&galley, word).unwrap_or_else(|| panic!("{theme:?} {mode:?} {contrast:?}: {word} is one colour: {:?}", sections(&galley)));
            let ratio = contrast_ratio(Rgba::rgb(color.r(), color.g(), color.b()), fill.get());
            assert!(ratio >= text_contrast_min(contrast), "{theme:?} {mode:?} {contrast:?}: {ink:?} at {ratio:.2}:1 on the field");
        }
        // High contrast may take an ink as far as the plain text's colour; standard keeps the accent apart.
        if contrast == Contrast::Standard {
            assert_ne!(color_of(&galley, "accent"), plain, "{theme:?} {mode:?}: the accent ink is not the plain text's");
        }
    }
}

/// The same text with and without ranges lays out alike, glyph for glyph: the ranges change colours and nothing else,
/// and without them the text is one colour.
#[trial]
fn ranges_change_only_the_colors_of_the_text() {
    let text = "var a = 1;\n\tvar b = 2; // a long line that wraps inside the field's width at least once or twice";
    let fill = Rc::new(Cell::new(Rgba::BLACK));
    let mut plain = field(text, None, fill.clone());
    let mut colored = field(text, Some(vars), fill);
    let (plain, colored) = (galley_with(plain.settle(), "long line"), galley_with(colored.settle(), "long line"));
    let glyphs = |galley: &egui::Galley| -> Vec<(egui::Rect, Vec<egui::Pos2>)> {
        galley.rows.iter().map(|row| (row.rect(), row.glyphs.iter().map(|glyph| glyph.pos).collect())).collect()
    };
    assert!(plain.rows.len() > 2, "the text wraps: {} rows", plain.rows.len());
    assert_eq!(glyphs(&plain), glyphs(&colored), "the coloured text is laid out as the plain one");
    let ink = color_of(&plain, text);
    assert!(ink.is_some(), "a field without ranges shows one colour: {:?}", sections(&plain));
    assert_ne!(color_of(&colored, "var"), ink, "the ranges are coloured: {:?}", sections(&colored));
    assert_eq!(color_of(&colored, " a = 1;"), ink, "the text between the ranges keeps the field's ink");
}

/// Typing into a coloured field edits its value, Tab inserts there, and the ranges follow the text as typed.
#[trial]
fn typing_in_a_colored_field_edits_it_and_colors_the_new_text() {
    let value = Rc::new(RefCell::new(String::new()));
    let shown = value.clone();
    let mut fixture = Fixture::new(move |ui| {
        TextField::new("Script", Bind::bound(&mut *shown.borrow_mut())).lines(3).mono().tab_inserts(true).color_ranges(vars).show(ui);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    fixture.type_text("x = var");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("y").settle();
    assert_eq!(*value.borrow(), "x = var\ty", "the typing reaches the value, with the tab");
    let galley = galley_with(&mut fixture, "x = ");
    assert_ne!(color_of(&galley, "var"), color_of(&galley, "x = "), "the typed range is coloured: {:?}", sections(&galley));
}

/// A text at the limit is coloured; one byte longer shows in one colour, and its ranges are never asked for.
#[trial]
fn a_text_past_the_limit_is_not_colored() {
    let longest = Rc::new(Cell::new(0));
    for (len, colored) in [(COLORED_TEXT_LIMIT, true), (COLORED_TEXT_LIMIT + 1, false)] {
        let text = format!("var {}", "x".repeat(len - 4));
        let value = Rc::new(RefCell::new(text));
        let (shown, asked) = (value.clone(), longest.clone());
        let mut fixture = Fixture::new(move |ui| {
            let asked = asked.clone();
            TextField::new("Script", Bind::bound(&mut *shown.borrow_mut()))
                .lines(3)
                .color_ranges(move |text| {
                    asked.set(asked.get().max(text.len()));
                    vars(text)
                })
                .show(ui);
        });
        let sections = sections(&galley_with(fixture.settle(), "var"));
        assert_eq!(sections.len() > 1, colored, "{len} bytes: coloured {colored}");
    }
    assert!(longest.get() <= COLORED_TEXT_LIMIT, "no text past the limit is handed to the ranges: {}", longest.get());
}

/// Ranges that are empty, overlap the one before, split a character or end past the text are skipped, and the valid
/// ones still colour.
#[trial]
fn ranges_that_do_not_fit_the_text_are_skipped() {
    fn hostile(text: &str) -> Vec<ColorRange> {
        let accent = RangeInk::Accent;
        // "é" is two bytes: 1..2 splits it. "var" is 3..6.
        vec![
            ColorRange::new(1..2, accent),
            ColorRange::new(3..6, accent),
            ColorRange::new(4..5, accent),
            ColorRange::new(7..7, accent),
            ColorRange::new(8..text.len() + 9, accent),
            ColorRange::new(usize::MAX - 1..usize::MAX, accent),
        ]
    }
    let fill = Rc::new(Cell::new(Rgba::BLACK));
    let mut fixture = field("é var b", Some(hostile), fill);
    let galley = galley_with(fixture.settle(), "var");
    let text: String = sections(&galley).into_iter().map(|(text, _)| text).collect();
    assert_eq!(text, "é var b", "the sections cover the text once, in order: {:?}", sections(&galley));
    let plain = color_of(&galley, "é ");
    assert!(plain.is_some(), "a range that splits a character is skipped: {:?}", sections(&galley));
    assert_ne!(color_of(&galley, "var"), plain, "the valid range is coloured: {:?}", sections(&galley));
    assert_eq!(color_of(&galley, " b"), plain, "the ranges past the text are skipped: {:?}", sections(&galley));
}
