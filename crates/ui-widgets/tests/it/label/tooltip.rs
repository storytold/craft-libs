//! The tooltip of a control that paints no label: its own label shows there, inside a field or not; a control named
//! only by its field label shows none, and the field's label stays where it was.

use super::kit::PlainStyle;
use craft_behavior::Sense;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Common, Component, Cx, GlyphLabel, LabelKind, MeasureCx, Props as _, Role, Show as _, ShowContainer as _, SideLabel};
use craft_ui::form::Field;
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, trial};
use craft_ui::tile_grid::{CaptionPlace, Tile};
use egui::{Pos2, Rect, Response, Ui, Vec2};
use std::cell::Cell;
use std::rc::Rc;

/// A mark that paints no text.
#[derive(craft_ui::contract::Component)]
struct Mark {
    #[common]
    common: Common,
}

impl Component for Mark {
    const ROLE: Role = Role::IMAGE;
    const SHOWS_LABEL: bool = false;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: MarkStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(24.0)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The texts the pointer's rest on what `show` shows (it returns its rect) adds after the tooltip delay.
fn texts_added_by_hover(show: impl Fn(&mut Ui) -> Rect + 'static) -> Vec<String> {
    let at = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&at);
    let mut fixture = Fixture::new(move |ui| seen.set(show(ui)));
    fixture.settle();
    // A cut run carries its full text too: runs are told apart by whether they are cut.
    let before: Vec<(String, bool)> = fixture.texts().iter().map(|run| (run.text.clone(), run.elided)).collect();
    let placed: Vec<(String, Rect)> = fixture.texts().iter().map(|run| (run.text.clone(), run.rect)).collect();
    fixture.frame(vec![moved(at.get().center())]);
    fixture.advance(DELAY);
    fixture.settle();
    // The tooltip shows on its own layer: every text shown before stays where it was, a field's label included.
    for shown in &placed {
        assert!(
            fixture.texts().iter().any(|run| run.text == shown.0 && run.rect == shown.1),
            "{:?} stays at {:?} under the tooltip",
            shown.0,
            shown.1
        );
    }
    let after = fixture.texts().iter().map(|run| (run.text.clone(), run.elided));
    after.filter(|run| !before.contains(run)).map(|(text, _)| text).collect()
}

/// A mark in the field "Layer", with `own` as its own label.
fn mark_in_field(own: Option<&'static str>) -> Vec<String> {
    texts_added_by_hover(move |ui| {
        let shown = Field::new("Layer").show(ui, |ui, _| {
            let mark = Mark { common: Common::new() };
            let mark = match own {
                Some(label) => mark.label(label),
                None => mark,
            };
            mark.show(ui).rect
        });
        shown.inner.unwrap_or(Rect::NOTHING)
    })
}

/// A mark in the field "Leading", whose label shows as a glyph at the row's start, with `hint` as its tooltip.
fn glyph_label(hint: Option<&'static str>) -> Vec<String> {
    texts_added_by_hover(move |ui| {
        let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text("A")));
        let side = match hint {
            Some(hint) => side.tooltip(hint),
            None => side,
        };
        Field::new("Leading").side(side).show(ui, |ui, _| Mark { common: Common::new() }.show(ui));
        // The glyph sits at the start of the row.
        Rect::from_min_size(Pos2::ZERO, Vec2::new(8.0, 24.0))
    })
}

#[trial]
fn a_control_that_paints_no_label_shows_its_own_label_as_tooltip() {
    assert_eq!(mark_in_field(Some("Lock")), ["Lock"], "a mark with its own label shows it inside a field");
    assert!(mark_in_field(None).is_empty(), "a mark the field names shows no tooltip");

    let tile = texts_added_by_hover(|ui| {
        let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::splat(96.0));
        Tile::new("Harbour at dusk").caption(CaptionPlace::None).show_at(ui, rect);
        rect
    });
    assert_eq!(tile, ["Harbour at dusk"], "a tile with no caption shows its name");

    let long = "Harbour at dusk, seen from the lighthouse on the northern breakwater";
    let cut = texts_added_by_hover(move |ui| {
        let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(96.0, 128.0));
        Tile::new(long).show_at(ui, rect);
        rect
    });
    assert_eq!(cut, [long], "a tile whose caption is cut shows its whole name");

    assert_eq!(glyph_label(None), ["Leading"], "a glyph label shows its name");
    assert_eq!(glyph_label(Some("Leading (1/1000 em)")), ["Leading (1/1000 em)"], "a glyph label's tooltip replaces its name");
}
