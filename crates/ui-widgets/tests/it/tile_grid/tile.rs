//! A tile's badge bar never draws one part over another: at every width the stars, the flag and the colour dot it
//! shows are apart and inside the tile, and a part that does not fit is left out.

use craft_theme::Srgba;
use craft_ui::contract::Show as _;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, trial};
use craft_ui::tile_grid::{BadgeVisibility, CardText, Tile, TileFlag};
use egui::{Color32, Rect, Shape, pos2, vec2};

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

/// An icon-only card tile with paper paints the paper fill, draws a crisp frame border, and scales the centered icon.
#[trial]
fn card_tile_with_paper_paints_paper_surface_frame_and_large_icon() {
    let mut frames = Frames::new();
    let mut tile_rect = Rect::NOTHING;
    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = frames.frame_shapes(|ui| {
            let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 300.0));
            tile_rect = in_region(ui, region, |ui| {
                Tile::new("Personal Template").card().paper(Color32::WHITE).icon(craft_behavior::IconId::new("file")).show(ui).response.rect
            });
        });
    }
    // Verify white paper fill exists inside the tile
    let paper_rect = shapes
        .iter()
        .find_map(|s| match s {
            Shape::Rect(r) if r.fill == Color32::WHITE => Some(r.rect),
            _ => None,
        })
        .expect("the white paper fill is painted");
    assert!(tile_rect.contains_rect(paper_rect), "paper {paper_rect:?} is inside tile {tile_rect:?}");

    // Verify frame stroke exists around the paper rect
    let has_frame = shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.rect == paper_rect && r.stroke.width > 0.0));
    assert!(has_frame, "a frame border is drawn around the paper rect");

    // Verify icon is centered on paper and sized at 44 pt (twice tokens.size.icon.lg = 22 pt in comfortable)
    let glyphs: Vec<Rect> = shapes.iter().filter(|s| matches!(s, Shape::Mesh(_))).map(Shape::visual_bounding_rect).collect();
    assert!(!glyphs.is_empty(), "icon glyph is painted");
    let icon_bounds = glyphs.into_iter().fold(Rect::NOTHING, |a, b| a.union(b));
    assert!((icon_bounds.center().x - paper_rect.center().x).abs() <= 2.0, "icon is centered horizontally");
    assert!((icon_bounds.center().y - paper_rect.center().y).abs() <= 2.0, "icon is centered vertically");
    // With 2x face side (44 pt), the visual bounding rect of the icon glyph is wider than 22 pt:
    assert!(icon_bounds.width() > 22.0 || icon_bounds.height() > 22.0, "icon is larger than 22 pt: bounds {icon_bounds:?}");
}

/// An icon-only card tile without paper paints a sunken well and a standard-size icon.
#[trial]
fn card_tile_without_paper_keeps_default_sunken_well_and_standard_icon() {
    let mut frames = Frames::new();
    let mut tile_rect = Rect::NOTHING;
    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = frames.frame_shapes(|ui| {
            let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 300.0));
            tile_rect = in_region(ui, region, |ui| Tile::new("Asset").card().icon(craft_behavior::IconId::new("file")).show(ui).response.rect);
        });
    }
    // Standard icon is clamped to tokens.size.icon.lg (22 pt max side)
    let glyphs: Vec<Rect> = shapes.iter().filter(|s| matches!(s, Shape::Mesh(_))).map(Shape::visual_bounding_rect).collect();
    assert!(!glyphs.is_empty(), "icon glyph is painted");
    let icon_bounds = glyphs.into_iter().fold(Rect::NOTHING, |a, b| a.union(b));
    assert!(icon_bounds.width() <= 22.0 && icon_bounds.height() <= 22.0, "standard icon bounds {icon_bounds:?} <= 22 pt");
}

/// Dedicated icon_zoom allows explicit scaling.
#[trial]
fn card_tile_icon_zoom_scales_icon() {
    let mut frames = Frames::new();
    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = frames.frame_shapes(|ui| {
            let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 300.0));
            in_region(ui, region, |ui| Tile::new("Asset").card().icon(craft_behavior::IconId::new("file")).icon_zoom(1.5).show(ui).response.rect);
        });
    }
    let glyphs: Vec<Rect> = shapes.iter().filter(|s| matches!(s, Shape::Mesh(_))).map(Shape::visual_bounding_rect).collect();
    let icon_bounds = glyphs.into_iter().fold(Rect::NOTHING, |a, b| a.union(b));
    // 1.5 * 22 pt = 33 pt
    assert!(icon_bounds.width() > 22.0 || icon_bounds.height() > 22.0, "icon scaled by zoom factor");
}
