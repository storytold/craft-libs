//! A tile's badge bar never draws one part over another: at every width the stars, the flag and the colour dot it
//! shows are apart and inside the tile, and a part that does not fit is left out.

use craft_theme::Srgba;
use craft_ui::contract::{Props as _, Show as _};
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
fn summary_card_paints_title_description_and_action() {
    let mut frames = Frames::new();
    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = frames.frame_shapes(|ui| {
            Tile::new("Arrange").icon(craft_behavior::IconId::new("grid")).detail("Move pages").summary_action("Use now").card().show(ui);
        });
    }
    let texts: Vec<_> = shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text(text) => Some(text.galley.job.text.as_str()),
            _ => None,
        })
        .collect();
    for text in ["Arrange", "Move pages", "Use now"] {
        assert!(texts.contains(&text), "missing {text}: {texts:?}");
    }
}

/// A summary card's description wraps onto a second line, as production's recommended-tool cards do, instead of being
/// cut with an ellipsis; the action stays below it.
#[trial]
fn a_summary_cards_long_description_wraps_instead_of_eliding() {
    const DETAIL: &str = "Sticky note \u{b7} Highlight text \u{b7} Underline";
    let mut frames = Frames::new();
    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = frames.frame_shapes(|ui| {
            Tile::new("Add comments").icon(craft_behavior::IconId::new("grid")).detail(DETAIL).summary_action("Use now").card().show(ui);
        });
    }
    let text = |wanted: &str| {
        shapes.iter().find_map(|shape| match shape {
            Shape::Text(text) if text.galley.job.text.starts_with(wanted) => Some((text.galley.rows.len(), text.visual_bounding_rect())),
            _ => None,
        })
    };
    let (lines, detail) = text("Sticky note").expect("the description is painted");
    let (_, action) = text("Use now").expect("the action is painted");
    assert!(lines >= 2, "the description wraps onto {lines} line(s) instead of being cut");
    assert!(detail.max.y <= action.min.y + 0.5, "the description {detail:?} stays above the action {action:?}");
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

/// A selected band tile (the organize grid's) is outlined as a whole: it paints an edge that the same tile unselected
/// does not.
#[trial]
fn a_selected_band_tile_is_outlined() {
    let edges = |selected: bool| {
        let mut frames = Frames::new();
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frames.frame_shapes(|ui| {
                Tile::new("Page 1").action_place(craft_ui::tile_grid::TileActionPlace::Band).selected(selected).show(ui);
            });
        }
        shapes.iter().filter(|shape| matches!(shape, Shape::Rect(rect) if rect.stroke.width > 0.0 && rect.stroke.color.a() > 0)).count()
    };
    assert!(edges(true) > edges(false), "selected {} against unselected {}", edges(true), edges(false));
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

/// Hostile or extreme icon_zoom values (huge, infinity, NaN, negative) are clamped and do not crash or panic.
#[trial]
fn card_tile_icon_zoom_hostile_values_handled_safely() {
    let mut frames = Frames::new();
    let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 300.0));

    // Test extreme zoom (10000.0) -> capped at picture rect, does not abort
    let shapes = frames.frame_shapes(|ui| {
        in_region(ui, region, |ui| {
            Tile::new("Huge Zoom").card().paper(Color32::WHITE).icon(craft_behavior::IconId::new("file")).icon_zoom(10000.0).show(ui);
        });
    });
    let paper_rect = shapes
        .iter()
        .find_map(|s| match s {
            Shape::Rect(r) if r.fill == Color32::WHITE => Some(r.rect),
            _ => None,
        })
        .expect("paper fill exists");
    let glyphs: Vec<Rect> = shapes.iter().filter(|s| matches!(s, Shape::Mesh(_))).map(Shape::visual_bounding_rect).collect();
    let icon_bounds = glyphs.into_iter().fold(Rect::NOTHING, |a, b| a.union(b));
    assert!(paper_rect.expand(0.5).contains_rect(icon_bounds), "extreme zoom icon is bounded within paper rect: {icon_bounds:?} in {paper_rect:?}");

    // Test capacity overflow candidate (1e9)
    let _ = frames.frame_shapes(|ui| {
        in_region(ui, region, |ui| {
            Tile::new("Overflow Zoom").card().paper(Color32::WHITE).icon(craft_behavior::IconId::new("file")).icon_zoom(1e9).show(ui);
        });
    });

    // Test NaN and negative zoom on card tile
    let _ = frames.frame_shapes(|ui| {
        in_region(ui, region, |ui| {
            Tile::new("NaN Zoom").card().paper(Color32::WHITE).icon(craft_behavior::IconId::new("file")).icon_zoom(f32::NAN).show(ui);
        });
    });
    let shapes_neg = frames.frame_shapes(|ui| {
        in_region(ui, region, |ui| {
            Tile::new("Negative Zoom").card().paper(Color32::WHITE).icon(craft_behavior::IconId::new("file")).icon_zoom(-5.0).show(ui);
        });
    });
    let glyphs_neg: Vec<Rect> = shapes_neg.iter().filter(|s| matches!(s, Shape::Mesh(_))).map(Shape::visual_bounding_rect).collect();
    assert!(!glyphs_neg.is_empty(), "negative zoom icon is rendered at default size, not 0x0");

    // Test non-card tile with NaN and negative zoom
    let _ = frames.frame_shapes(|ui| {
        in_region(ui, region, |ui| {
            Tile::new("Non-card NaN").icon(craft_behavior::IconId::new("file")).icon_zoom(f32::NAN).show(ui);
        });
    });
    let non_card_neg = frames.frame_shapes(|ui| {
        in_region(ui, region, |ui| {
            Tile::new("Non-card Neg").icon(craft_behavior::IconId::new("file")).icon_zoom(-2.0).show(ui);
        });
    });
    let glyphs_non_card: Vec<Rect> = non_card_neg.iter().filter(|s| matches!(s, Shape::Mesh(_))).map(Shape::visual_bounding_rect).collect();
    assert!(!glyphs_non_card.is_empty(), "non-card negative zoom renders icon");
}

/// A flat_thumbnail tile with paper does not paint paper fill or frame, and keeps standard icon zoom and proper contrast across themes.
#[trial]
fn flat_thumbnail_with_paper_keeps_standard_icon_and_no_paper_surface() {
    use craft_theme::{Contrast, Mode};
    use craft_ui::runtime::env::RootEnv;

    for (mode, contrast) in [(Mode::Light, Contrast::Standard), (Mode::Dark, Contrast::Standard), (Mode::Dark, Contrast::High)] {
        let mut frames = Frames::new();
        frames.root(RootEnv::default().mode(mode).contrast(contrast));
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frames.frame_shapes(|ui| {
                let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 300.0));
                in_region(ui, region, |ui| {
                    Tile::new("Flat Preset").card().flat_thumbnail().paper(Color32::WHITE).icon(craft_behavior::IconId::new("file")).show(ui);
                });
            });
        }
        // No white paper fill painted
        assert!(
            !shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.fill == Color32::WHITE)),
            "{mode:?} {contrast:?}: no paper fill painted on flat thumbnail"
        );
        // Standard icon is clamped to tokens.size.icon.lg (22 pt max side)
        let glyphs: Vec<Rect> = shapes.iter().filter(|s| matches!(s, Shape::Mesh(_))).map(Shape::visual_bounding_rect).collect();
        assert!(!glyphs.is_empty(), "{mode:?} {contrast:?}: icon glyph is painted");
        let icon_bounds = glyphs.into_iter().fold(Rect::NOTHING, |a, b| a.union(b));
        assert!(
            icon_bounds.width() <= 22.0 && icon_bounds.height() <= 22.0,
            "{mode:?} {contrast:?}: icon bounds {icon_bounds:?} <= 22 pt on flat thumbnail"
        );

        // In dark mode, icon ink must NOT be dark text chosen against white paper; it must be light against the dark surface
        if mode == Mode::Dark {
            let icon_colors: Vec<Color32> = shapes
                .iter()
                .filter_map(|s| match s {
                    Shape::Mesh(m) if !m.vertices.is_empty() => Some(m.vertices[0].color),
                    _ => None,
                })
                .collect();
            for color in icon_colors {
                let lum = f32::from(color.r()) * 0.2126 + f32::from(color.g()) * 0.7152 + f32::from(color.b()) * 0.0722;
                assert!(lum > 100.0, "{mode:?} {contrast:?}: icon color {color:?} on flat thumbnail should be light in dark mode (lum {lum})");
            }
        }
    }
}

/// In a narrow room, zoom does not paint the icon outside the paper rect.
#[trial]
fn card_tile_in_narrow_room_caps_icon_at_picture_rect() {
    let mut frames = Frames::new();
    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = frames.frame_shapes(|ui| {
            let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(60.0, 100.0));
            in_region(ui, region, |ui| {
                Tile::new("Narrow").card().paper(Color32::WHITE).icon(craft_behavior::IconId::new("file")).icon_zoom(20.0).show(ui);
            });
        });
    }
    let paper_rect = shapes
        .iter()
        .find_map(|s| match s {
            Shape::Rect(r) if r.fill == Color32::WHITE => Some(r.rect),
            _ => None,
        })
        .expect("paper fill exists");
    let glyphs: Vec<Rect> = shapes.iter().filter(|s| matches!(s, Shape::Mesh(_))).map(Shape::visual_bounding_rect).collect();
    let icon_bounds = glyphs.into_iter().fold(Rect::NOTHING, |a, b| a.union(b));
    assert!(paper_rect.expand(0.5).contains_rect(icon_bounds), "icon {icon_bounds:?} is inside narrow paper {paper_rect:?}");
}

/// A page card's name wraps onto a second line, centred, where one line would cut it, and the detail follows the name
/// inside the card, whether the name took one line or two.
#[trial]
fn a_page_card_wraps_its_name_and_sets_the_detail_under_it() {
    let shown = |name: &'static str| {
        let mut frames = Frames::new();
        let mut tile = Rect::NOTHING;
        for _ in 0..3 {
            frames.frame(Vec::new(), |ui| {
                let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 300.0));
                tile = in_region(ui, region, |ui| {
                    Tile::new(name).card().card_text(CardText::Page).detail("1080 × 1080 px").page(1080.0, 1080.0).show(ui).response.rect
                });
            });
        }
        let texts = frames.texts();
        let name = texts.iter().find(|line| line.text == name).expect("the name is painted").rect;
        let detail = texts.iter().find(|line| line.text == "1080 × 1080 px").expect("the detail is painted").rect;
        (tile, name, detail)
    };
    let (_, short, short_detail) = shown("Letter");
    let long_name = "Social Square Post 1080×1080 for every network";
    let (tile, long, detail) = shown(long_name);
    assert!(long.height() > short.height() * 1.5, "the long name takes a second line: {long:?} against {short:?}");
    assert!((long.center().x - tile.center().x).abs() <= 1.0, "the name is centred: {long:?} in {tile:?}");
    for (name, detail) in [(short, short_detail), (long, detail)] {
        assert!(
            detail.top() >= name.bottom() - 1.0 && detail.top() - name.bottom() < short.height(),
            "the detail follows the name: {detail:?} under {name:?}"
        );
        assert!(tile.contains_rect(detail), "the detail is inside the card: {detail:?} in {tile:?}");
    }
}
