//! The in-ribbon gallery: its tile count follows the group's level, and its stepping stays in range.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_menu::{Gallery, GalleryFace, GalleryItem, GroupLevel};
use craft_ui::contract::Show as _;
use craft_ui::list::{ThumbAspect, ThumbSize};
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::{Label, with_env};
use craft_ui::testing::{Fixture, trial};
use craft_ui::tile_grid::CaptionPlace;
use egui::Rect;

/// The tiles of a gallery over `items`, six at the full level and three at the compact one: which of the first
/// `of` tiles show, by their labels.
fn shown(fixture: &Fixture, of: usize) -> Vec<usize> {
    (0..of).filter(|at| fixture.by_label(&format!("Tile {at}")).is_some()).collect()
}

#[trial]
fn the_tile_count_follows_the_group_level_and_stepping_stays_in_range_when_the_items_change() {
    let level = Rc::new(Cell::new(GroupLevel::Full));
    let count = Rc::new(Cell::new(10_usize));
    let picks = Rc::new(RefCell::new(Vec::<usize>::new()));
    let mut fixture = {
        let (level, count, picks) = (Rc::clone(&level), Rc::clone(&count), Rc::clone(&picks));
        Fixture::new(move |ui| {
            let paint = |_: &ContentPainterCx, _: Rect| {};
            let items: Vec<GalleryItem<'_, usize>> = (0..count.get())
                .map(|at| GalleryItem { key: at, label: Label::from(format!("Tile {at}")), face: GalleryFace::Paint(&paint) })
                .collect();
            let at_level = level.get();
            let out = with_env(
                ui,
                |set| {
                    set.values.set(at_level);
                },
                |ui| Gallery::new("Styles", &items, None).tiles(6, 3).show(ui),
            );
            if let Some(key) = out.picked {
                picks.borrow_mut().push(key);
            }
        })
    };
    fixture.settle();
    assert_eq!(shown(&fixture, 10), vec![0, 1, 2, 3, 4, 5], "six tiles at the full level");

    let down = fixture.by_label("Next row").unwrap_or_else(|| panic!("the down button is named"));
    fixture.click(&down);
    fixture.settle();
    assert_eq!(shown(&fixture, 10), vec![4, 5, 6, 7, 8, 9], "a step down moves by the shown count, to the last full run of ten");
    let up = fixture.by_label("Previous row").unwrap_or_else(|| panic!("the up button is named"));
    fixture.click(&up);
    fixture.settle();
    assert_eq!(shown(&fixture, 10), vec![0, 1, 2, 3, 4, 5], "a step up goes back");

    level.set(GroupLevel::Compact);
    fixture.settle();
    assert_eq!(shown(&fixture, 10), vec![0, 1, 2], "three tiles at the compact level");

    level.set(GroupLevel::Full);
    fixture.settle();
    let down = fixture.by_label("Next row").unwrap_or_else(|| panic!("the down button is named"));
    fixture.click(&down);
    fixture.settle();
    assert_eq!(shown(&fixture, 10), vec![4, 5, 6, 7, 8, 9], "stepped to the end");
    count.set(3);
    fixture.settle();
    assert_eq!(shown(&fixture, 3), vec![0, 1, 2], "three items under a step past their end still show, none of the strip empty");

    let tile = fixture.by_label("Tile 1").unwrap_or_else(|| panic!("the second tile shows"));
    fixture.click(&tile);
    fixture.settle();
    assert_eq!(*picks.borrow(), vec![1], "a click on a tile picks its key");
}

/// The tiles' rects and the painted texts (with their rects) of a gallery of three, square and uncaptioned or two to
/// one and named under each picture.
fn tiles(wide: bool) -> (Vec<Rect>, Vec<(String, Rect)>) {
    let mut fixture = Fixture::new(move |ui| {
        let paint = |_: &ContentPainterCx, _: Rect| {};
        let items: Vec<GalleryItem<'_, usize>> =
            (0..3).map(|at| GalleryItem { key: at, label: Label::from(format!("Tile {at}")), face: GalleryFace::Paint(&paint) }).collect();
        let gallery = Gallery::new("Styles", &items, None).tiles(3, 3);
        let gallery = if wide { gallery.aspect(ThumbAspect::new(2, 1)).caption(CaptionPlace::Below) } else { gallery };
        let _ = gallery.show(ui);
    });
    fixture.settle();
    let rects = (0..3).filter_map(|at| fixture.by_label(&format!("Tile {at}")).map(|tile| tile.rect)).collect();
    (rects, fixture.texts().iter().map(|run| (run.text.clone(), run.rect)).collect())
}

/// The rect of the first tile of a square gallery of three at thumbnail `size`; the default without one.
fn first_tile(size: Option<ThumbSize>) -> Rect {
    let mut fixture = Fixture::new(move |ui| {
        let paint = |_: &ContentPainterCx, _: Rect| {};
        let items: Vec<GalleryItem<'_, usize>> =
            (0..3).map(|at| GalleryItem { key: at, label: Label::from(format!("Tile {at}")), face: GalleryFace::Paint(&paint) }).collect();
        let gallery = Gallery::new("Style Sets", &items, None).tiles(3, 3);
        let gallery = match size {
            Some(size) => gallery.thumb(size),
            None => gallery,
        };
        let _ = gallery.show(ui);
    });
    fixture.settle();
    fixture.by_label("Tile 0").map_or(Rect::NOTHING, |tile| tile.rect)
}

/// A gallery's tiles take the thumbnail size it names: a square large tile is much wider than the default medium one
/// (every tile is as high as the strip, so its width shows its picture's size), and `ThumbSize::None` keeps the
/// medium size, since a gallery tile always shows its picture.
#[trial]
fn a_gallery_at_the_large_thumbnail_size_shows_larger_tiles() {
    let (medium, large, none) = (first_tile(None), first_tile(Some(ThumbSize::Large)), first_tile(Some(ThumbSize::None)));
    assert!(medium.is_positive(), "the default tile shows: {medium:?}");
    assert!(large.width() > 1.5 * medium.width(), "a large tile is wider: {large:?} against {medium:?}");
    assert!(large.height() >= medium.height() - 0.5, "a large tile is no shorter: {large:?} against {medium:?}");
    assert!((none.size() - medium.size()).length() < 0.5, "no thumbnail size keeps the medium tile: {none:?} against {medium:?}");
}

/// A style gallery's tiles are twice as wide as a square one's and name their items under the pictures, inside the
/// tile; a plain gallery's tiles are square and show no names. Every tile is as high as the strip (its column of
/// buttons), as the ribbon's gallery tiles always were, so a caption takes its line inside the tile rather than
/// making it taller.
#[trial]
fn a_wide_captioned_gallery_names_its_tiles_under_wide_pictures() {
    let (square, plain) = tiles(false);
    let (wide, named) = tiles(true);
    assert_eq!((square.len(), wide.len()), (3, 3), "every tile shows");
    let (square, wide) = (square[0], wide[0]);
    assert!(wide.width() > 1.5 * square.width(), "a two to one tile is wider: {wide:?} against {square:?}");
    let name = named.iter().find(|(text, _)| text == "Tile 0").map(|(_, rect)| *rect);
    let Some(name) = name else { panic!("a captioned tile paints its name: {named:?}") };
    assert!(wide.expand(0.5).contains_rect(name), "the name {name:?} keeps its line inside its tile {wide:?}");
    // The picture is at the tile's top, at least half as high as the square tile is wide: the name is under it.
    assert!(name.min.y >= wide.min.y + square.width() / 2.0, "the name {name:?} is under the picture in {wide:?}");
    assert!(!plain.iter().any(|(text, _)| text == "Tile 0"), "a plain tile paints no name: {plain:?}");
}

/// A framed gallery paints a sunken well fill over its bounds, and a panel fill over its stepper button track.
#[trial]
fn a_framed_gallery_paints_sunken_well_and_stepper_track() {
    use craft_ui::runtime::env::Env;
    use egui::Color32;

    fn fills(shapes: &[egui::Shape], out: &mut Vec<(egui::Rect, egui::Color32)>) {
        for shape in shapes {
            match shape {
                egui::Shape::Vec(inner) => fills(inner, out),
                egui::Shape::Rect(rect) => out.push((rect.rect, rect.fill)),
                _ => {}
            }
        }
    }

    let colors_and_rect = Rc::new(Cell::new((Rect::NOTHING, Color32::TRANSPARENT, Color32::TRANSPARENT)));
    let held = colors_and_rect.clone();
    let mut fixture = Fixture::new(move |ui| {
        let [sr, sg, sb, sa] = Env::of(ui).tokens.tokens().color.surface.sunken.0;
        let [pr, pg, pb, pa] = Env::of(ui).tokens.tokens().color.surface.panel.0;
        let sunken = Color32::from_rgba_unmultiplied(sr, sg, sb, sa);
        let panel = Color32::from_rgba_unmultiplied(pr, pg, pb, pa);

        let paint = |_: &ContentPainterCx, _: Rect| {};
        let items: Vec<GalleryItem<'_, usize>> =
            (0..10).map(|at| GalleryItem { key: at, label: Label::from(format!("Tile {at}")), face: GalleryFace::Paint(&paint) }).collect();
        let out = Gallery::new("Styles", &items, None).tiles(6, 3).framed().show(ui);
        held.set((out.response.rect, sunken, panel));
    });
    fixture.settle();

    let (gallery_rect, sunken, panel) = colors_and_rect.get();
    assert!(gallery_rect.is_positive(), "the gallery paints a positive rect");

    let mut seen = Vec::new();
    fills(&fixture.frame_shapes(), &mut seen);

    assert!(
        seen.iter()
            .any(|(rect, fill)| *fill == sunken && rect.contains_rect(gallery_rect.shrink(1.0)) && gallery_rect.expand(1.0).contains_rect(*rect)),
        "a sunken fill covers the gallery {gallery_rect:?}: {seen:?}"
    );

    let up = fixture.by_label("Previous row").expect("the up button shows");
    let down = fixture.by_label("Next row").expect("the down button shows");
    let more = fixture.by_label("More").expect("the more button shows");
    let column = up.rect.union(down.rect).union(more.rect);
    let stepper_track =
        seen.iter().find(|(rect, fill)| *fill == panel && rect.contains_rect(column.shrink(1.0)) && gallery_rect.contains_rect(*rect));
    assert!(stepper_track.is_some(), "a panel fill covers the stepper track column {column:?}: {seen:?}");
}

/// The current tile paints an `accent.soft` fill and a 1 pt `accent.solid` edge stroked inside it, and a non-current
/// tile paints neither; nothing filled `accent.solid` lies under the tile's content (a translucent `accent.soft`
/// would show it through); the fill stays under the focus ring, so a focused current tile still reads as current.
/// This holds in light and dark, at standard and high contrast.
#[trial]
fn the_current_tile_paints_an_accent_fill_and_edge_even_when_focused() {
    use craft_theme::{Contrast, Mode};
    use craft_ui::runtime::env::Env;
    use egui::Color32;

    /// A painted rect: its bounds, fill, and the colour of its stroke when it has one.
    struct Painted {
        rect: Rect,
        fill: Color32,
        stroke: Option<Color32>,
    }
    fn rects(shapes: &[egui::Shape], out: &mut Vec<Painted>) {
        for shape in shapes {
            match shape {
                egui::Shape::Vec(inner) => rects(inner, out),
                egui::Shape::Rect(r) => out.push(Painted { rect: r.rect, fill: r.fill, stroke: (r.stroke.width > 0.0).then_some(r.stroke.color) }),
                _ => {}
            }
        }
    }

    for (mode, contrast) in
        [(Mode::Light, Contrast::Standard), (Mode::Dark, Contrast::Standard), (Mode::Light, Contrast::High), (Mode::Dark, Contrast::High)]
    {
        let colors = Rc::new(Cell::new((craft_theme::Rgba::TRANSPARENT, craft_theme::Rgba::TRANSPARENT)));
        let held = colors.clone();
        let mut fixture = Fixture::new(move |ui| {
            let accent = Env::of(ui).tokens.tokens().color.accent;
            held.set((accent.soft, accent.solid));
            let paint = |_: &ContentPainterCx, _: Rect| {};
            let items: Vec<GalleryItem<'_, usize>> =
                (0..3).map(|at| GalleryItem { key: at, label: Label::from(format!("Tile {at}")), face: GalleryFace::Paint(&paint) }).collect();
            let _ = Gallery::new("Styles", &items, Some(&0)).tiles(3, 3).show(ui);
        });
        fixture.theme(mode, contrast).settle();
        let (soft, solid) = colors.get();
        let [fill, edge] = [soft, solid].map(|c| {
            let [r, g, b, a] = c.0;
            Color32::from_rgba_unmultiplied(r, g, b, a)
        });
        assert_ne!(fill, edge, "{mode:?} {contrast:?}: the fill and the edge are different colours");
        let [current, other] = ["Tile 0", "Tile 1"].map(|name| fixture.by_label(name).expect("the tile shows"));
        let (current, other) = (current.rect, other.rect);
        let near = |p: &Painted, tile: Rect| p.rect.intersects(tile) && tile.expand(8.0).contains_rect(p.rect);
        let seen = |fixture: &mut Fixture| {
            let mut seen = Vec::new();
            rects(&fixture.frame_shapes(), &mut seen);
            seen
        };
        let shapes = seen(&mut fixture);
        let who = format!("{mode:?} {contrast:?}");
        assert!(shapes.iter().any(|p| p.fill == fill && near(p, current)), "{who}: the current tile fills accent.soft");
        assert!(shapes.iter().any(|p| p.stroke == Some(edge) && near(p, current)), "{who}: the current tile's edge is an accent.solid stroke");
        assert!(
            !shapes.iter().any(|p| p.fill == edge && p.rect.contains_rect(current.shrink(2.0))),
            "{who}: no accent.solid fill lies under the tile"
        );
        assert!(!shapes.iter().any(|p| (p.fill == fill || p.stroke == Some(edge)) && near(p, other)), "{who}: another tile paints neither");

        fixture.key(egui::Key::Tab, egui::Modifiers::NONE).settle();
        let tile0 = fixture.by_label("Tile 0").expect("the tile shows").node;
        assert_eq!(craft_ui::testing::access::focused_key(fixture.tree()), tile0, "{who}: Tab focused the current tile");
        let shapes = seen(&mut fixture);
        assert!(shapes.iter().any(|p| p.fill == fill && near(p, current)), "{who}: the focused current tile keeps its fill");
    }
}
