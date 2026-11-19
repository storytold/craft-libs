//! A tile's badge bar never draws one part over another: at every width the stars, the flag and the colour dot it
//! shows are apart and inside the tile, and a part that does not fit is left out.

use craft_theme::Srgba;
use craft_ui::contract::Show as _;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, trial};
use craft_ui::tile_grid::{BadgeVisibility, Tile, TileFlag};
use egui::{Rect, Shape, pos2, vec2};

/// The tile's rect and the bounds of the glyphs (stars and flag) and colour dots it paints in a region `width` wide.
fn badges(width: f32) -> (Rect, Vec<Rect>, Vec<Rect>) {
    let mut frames = Frames::new();
    let (mut tile, mut shapes) = (Rect::NOTHING, Vec::new());
    for _ in 0..3 {
        shapes = frames.frame_shapes(|ui| {
            let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 400.0));
            tile = in_region(ui, region, |ui| {
                Tile::new("IMG_0002.jpg")
                    .rating(5)
                    .flag(TileFlag::Picked)
                    .label_color(Srgba::rgb(0.86, 0.31, 0.24))
                    .bar(BadgeVisibility::Always)
                    .show(ui)
                    .response
                    .rect
            });
        });
    }
    let glyphs = shapes.iter().filter(|shape| matches!(shape, Shape::Mesh(_))).map(Shape::visual_bounding_rect).collect();
    let mut dots: Vec<Rect> = Vec::new();
    for shape in &shapes {
        if let Shape::Circle(circle) = shape {
            let at = Rect::from_center_size(circle.center, vec2(2.0 * circle.radius, 2.0 * circle.radius));
            if !dots.iter().any(|dot| dot.center() == at.center()) {
                dots.push(at);
            }
        }
    }
    (tile, glyphs, dots)
}

fn overlap(a: &Rect, b: &Rect) -> bool {
    let both = a.intersect(*b);
    both.width() > 0.01 && both.height() > 0.01
}

#[trial]
fn the_badge_bar_leaves_out_what_does_not_fit_and_never_overlaps() {
    let (_, glyphs, dots) = badges(400.0);
    assert_eq!((glyphs.len(), dots.len()), (6, 1), "at the default card width the five stars, the flag and the dot show");
    for width in [40.0, 64.0, 80.0, 96.0, 128.0, 400.0] {
        let (tile, glyphs, dots) = badges(width);
        let parts: Vec<Rect> = glyphs.into_iter().chain(dots).collect();
        for (at, a) in parts.iter().enumerate() {
            assert!(tile.expand(0.5).contains_rect(*a), "at {width}: {a:?} is inside the tile {tile:?}");
            for b in &parts[at + 1..] {
                assert!(!overlap(a, b), "at {width}: {a:?} and {b:?} overlap");
            }
        }
    }
}

/// A named hover action carries its tile in its accessible name, so the shown actions of different tiles are told
/// apart; a selected tile shows its actions without the pointer, and the actions of a tile that does not show them
/// stay out of the tree.
#[trial]
fn a_named_action_is_exposed_by_its_own_name_while_its_tile_shows_it() {
    use craft_behavior::IconId;
    use craft_ui::contract::Props as _;
    use craft_ui::runtime::commands::CommandRef;
    use craft_ui::testing::Fixture;
    use craft_ui::testing::pointer::moved;
    const NAMES: [&str; 3] = ["Rotate page 1 clockwise", "Rotate page 2 clockwise", "Rotate page 3 clockwise"];
    const PAGES: [&str; 3] = ["Page 1", "Page 2", "Page 3"];
    let mut fixture = Fixture::new(|ui| {
        ui.horizontal(|ui| {
            for (at, (page, name)) in PAGES.iter().zip(NAMES).enumerate() {
                Tile::new(*page).action_named(IconId::new("rotate_cw"), CommandRef::new("page.rotate"), name).selected(at == 2).show(ui);
            }
        });
    });
    fixture.room(vec2(600.0, 300.0)).settle();
    assert!(NAMES[..2].iter().all(|name| fixture.by_label(name).is_none()), "no tile is hovered, so no action shows");
    assert!(fixture.by_label(NAMES[2]).is_some(), "the selected tile shows its action unhovered");
    let over = fixture.by_label(PAGES[1]).expect("the second tile").rect.center();
    fixture.frame(vec![moved(over)]).settle();
    assert!(fixture.by_label(NAMES[1]).is_some(), "the hovered tile's action is named for its tile");
    assert!(fixture.by_label(NAMES[0]).is_none(), "the other tile's action stays hidden");
}
