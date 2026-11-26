//! A select's width: the field width by default, its widest entry when it is sized to its content.

use craft_ui::contract::{Bind, Entry, Props as _, Show as _, Sizing};
use craft_ui::pickers::{Select, SelectLook};
use craft_ui::testing::{Fixture, trial};
use std::cell::Cell;
use std::rc::Rc;

/// A select sized to its content is narrower than the field width a select takes by default, and still shows its
/// widest entry whole.
#[trial]
fn a_content_sized_select_fits_its_widest_entry() {
    let rects = Rc::new(Cell::new([egui::Rect::NOTHING; 2]));
    let seen = rects.clone();
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(0u8, "One"), Entry::new(1u8, "Twelve")];
        let plain = Select::new("Plain", Bind::given(1u8), &entries).show(ui).response.rect;
        let fitted = Select::new("Fitted", Bind::given(1u8), &entries).sizing(Sizing::Content).show(ui).response.rect;
        seen.set([plain, fitted]);
    });
    fixture.settle();
    let [plain, fitted] = rects.get();
    assert!(fitted.width() < plain.width(), "the content-sized select is narrower than the default one: {fitted:?} {plain:?}");
    let shown = fixture.texts().iter().find(|run| run.text == "Twelve" && fitted.contains_rect(run.rect)).map(|run| run.elided);
    assert_eq!(shown, Some(false), "the content-sized select shows its widest entry whole inside its box");
}

/// The width of a quiet select showing entry `held` of a short and a long entry.
fn quiet_width(held: u8) -> f32 {
    let width = Rc::new(Cell::new(0.0));
    let seen = width.clone();
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(0u8, "Essentials"), Entry::new(1u8, "Print production and prepress")];
        seen.set(Select::new("Workspace", Bind::given(held), &entries).look(SelectLook::Quiet).show(ui).response.rect.width());
    });
    fixture.settle();
    width.get()
}

/// A quiet select is as wide as the entry it shows, not its widest one: it widens with a longer pick.
#[trial]
fn a_quiet_select_fits_the_entry_it_shows() {
    let (short, long) = (quiet_width(0), quiet_width(1));
    assert!(short > 0.0 && short < long, "the quiet select is narrower showing the short entry: {short} against {long}");
}

/// A filled select (`SelectLook::Filled`, VectorCraft's status bar) paints its closed box in `surface.control`, which
/// a field does not, and keeps a field's width whichever entry it shows.
#[trial]
fn a_filled_select_lies_on_the_control_surface_at_a_fields_width() {
    let shown = |look: SelectLook, held: u8| {
        let rect = Rc::new(Cell::new(egui::Rect::NOTHING));
        let seen = rect.clone();
        let mut fixture = Fixture::new(move |ui| {
            let entries = [Entry::new(0u8, "Essentials"), Entry::new(1u8, "Print production and prepress")];
            seen.set(Select::new("Workspace", Bind::given(held), &entries).look(look).show(ui).response.rect);
        });
        fixture.settle();
        let control = craft_ui::style::StyleCx::with_root(fixture.ctx(), |cx| cx.tokens.color.surface.control).0;
        let control = egui::Color32::from_rgba_unmultiplied(control[0], control[1], control[2], control[3]);
        let box_rect = rect.get();
        let filled = fixture.frame_shapes().iter().any(|shape| {
            matches!(shape, egui::Shape::Rect(fill) if fill.fill == control && fill.rect.expand(0.5).contains_rect(box_rect) && box_rect.expand(0.5).contains_rect(fill.rect))
        });
        (box_rect.width(), filled)
    };
    let ((short, filled), (long, _)) = (shown(SelectLook::Filled, 0), shown(SelectLook::Filled, 1));
    assert!(filled, "the filled select's box is surface.control");
    assert!((short - long).abs() <= 0.5, "the filled select keeps its width: {short} against {long}");
    let (_, field) = shown(SelectLook::Field, 0);
    assert!(!field, "a field's box is not surface.control");
}
