//! A tile's badge bar never draws one part over another: at every width the stars, the flag and the colour dot it
//! shows are apart and inside the tile, and a part that does not fit is left out.

use craft_theme::Srgba;
use craft_ui::contract::{Props as _, Show as _};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, trial};
use craft_ui::tile_grid::{BadgeVisibility, CardText, Tile, TileFlag};
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
fn the_badge_bar_leaves_out_what_does_not_fit_and_never_overlap() {
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

/// A card of a preset centres its name; a card of a file sets it at the reading start.
#[trial]
fn card_text_places_the_name_centred_for_presets_and_at_the_start_for_files() {
    let name_in = |text: CardText| {
        let mut frames = Frames::new();
        let mut tile = Rect::NOTHING;
        for _ in 0..3 {
            frames.frame(Vec::new(), |ui| {
                let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 300.0));
                tile = in_region(ui, region, |ui| Tile::new("Poster").card().card_text(text).detail("Just now").show(ui).response.rect);
            });
        }
        let name = frames.texts().iter().find(|line| line.text == "Poster").expect("the name is painted").rect;
        (tile, name)
    };
    let (tile, name) = name_in(CardText::Preset);
    assert!((name.center().x - tile.center().x).abs() <= 1.0, "a preset's name is centred: {name:?} in {tile:?}");
    let (tile, name) = name_in(CardText::File);
    assert!(name.left() - tile.left() < tile.width() / 4.0, "a file's name starts at the tile's start edge: {name:?} in {tile:?}");
}

/// The rects a tile of 200 wide paints, and its sunken plate rect when it paints one; `selected` and `hovered` ring it.
fn painted(plateless: bool, selected: bool, hovered: bool) -> (Vec<Rect>, Option<Rect>) {
    use craft_ui::runtime::env::Env;
    use egui::Color32;
    let mut frames = Frames::new();
    let (mut sunken, mut shapes) = (Color32::TRANSPARENT, Vec::new());
    let mut body = |ui: &mut egui::Ui| {
        let [r, g, b, a] = Env::of(ui).tokens.tokens().color.surface.sunken.0;
        sunken = Color32::from_rgba_unmultiplied(r, g, b, a);
        let region = Rect::from_min_size(pos2(20.0, 20.0), vec2(160.0, 200.0));
        in_region(ui, region, |ui| {
            let tile = Tile::new("Poster").selected(selected);
            let _ = (if plateless { tile.plateless() } else { tile }).show(ui);
        });
    };
    for _ in 0..3 {
        if hovered {
            frames.frame(vec![egui::Event::PointerMoved(pos2(100.0, 60.0))], &mut body);
        }
        shapes = frames.frame_shapes(&mut body);
    }
    let rects: Vec<(Rect, Color32)> = shapes
        .iter()
        .filter_map(|shape| if let Shape::Rect(r) = shape { Some((r.rect, r.fill)).filter(|(_, fill)| fill.a() > 0) } else { None })
        .collect();
    let plate = rects.iter().find(|(rect, fill)| *fill == sunken && rect.area() > 100.0).map(|(rect, _)| *rect);
    (rects.into_iter().map(|(rect, _)| rect).collect(), plate)
}

/// A plateless tile paints no sunken plate (a default one keeps it), and its hover and selection rings go around the
/// picture without filling it, so a transparent painter is not flooded by the accent or the muted ink.
#[trial]
fn a_plateless_tile_paints_no_plate_and_its_rings_leave_the_picture_clear() {
    let (_, plate) = painted(false, false, false);
    let picture = plate.expect("a default tile paints the plate behind its picture");
    assert!(painted(true, false, false).1.is_none(), "a plateless tile paints no plate");
    for (selected, hovered) in [(true, false), (false, true)] {
        let (rects, _) = painted(true, selected, hovered);
        let inside = picture.shrink(1.0);
        assert!(
            !rects.iter().any(|r| overlap(r, &inside)),
            "selected {selected}, hovered {hovered}: nothing fills the picture {picture:?}: {rects:?}"
        );
        assert!(
            rects.iter().any(|r| overlap(r, &picture.expand(2.0)) && !picture.contains_rect(*r)),
            "selected {selected}, hovered {hovered}: a ring is painted around the picture"
        );
    }
}
