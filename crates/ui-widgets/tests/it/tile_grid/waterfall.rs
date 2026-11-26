//! Waterfall placement, viewport lookup and keyboard focus through the grid.

use craft_behavior::{DisplayOrder, ItemLayout as _, ItemSource, ScrollPos, SizeHint};
use craft_i18n::Direction;
use craft_ui::contract::ShowCollection as _;
use craft_ui::testing::{Fixture, access::focused_key, trial};
use craft_ui::tile_grid::{Around, ArrangeSpacing, GridLayout, Tile, TileGrid, TileGroup, Tiling, arrange};
use egui::{Key, Modifiers, Rect, pos2, vec2};

fn waterfall(aspects: &[f32], groups: &[TileGroup<usize>], width: f32) -> GridLayout {
    let around = Around::new(6.0, 2.0, 3.0, 12.0);
    let arrangement = arrange(aspects, groups, width, Tiling::waterfall(100.0), ArrangeSpacing::new(8.0, 4.0, 30.0), around);
    GridLayout::arranged(arrangement, width, around)
}

#[trial]
fn waterfall_pictures_keep_their_shape_and_use_the_shortest_column() {
    let layout = waterfall(&[0.5, 2.0, f32::NAN, 1.0], &[], 240.0);
    let laid = layout.arrangement().expect("arrangement");
    let [tall, wide, unknown, square] = laid.tiles.as_slice() else { panic!("four pictures") };
    assert!(tall.left() < wide.left() && tall.top() == wide.top(), "the first tie goes to reading start");
    assert_eq!(wide.left(), unknown.left(), "the next tile goes below the short picture");
    assert_eq!(unknown.left(), square.left(), "the same column still has a lower bottom");
    assert!(wide.bottom() < unknown.top() && unknown.bottom() < square.top());
    assert!(laid.tiles.iter().all(|picture| (picture.width() - tall.width()).abs() < 0.01));
    for (picture, aspect) in laid.tiles.iter().zip([0.5, 2.0, 1.0, 1.0]) {
        assert!((picture.width() / picture.height() - aspect).abs() < 0.01);
    }
}

#[trial]
fn waterfall_shrinks_to_one_narrow_column() {
    let layout = waterfall(&[0.5, 2.0], &[], 62.0);
    let first = layout.rect(0, ScrollPos::default());
    let next = layout.rect(1, ScrollPos::default());
    assert!(first.right() < 62.0 && next.right() < 62.0);
    assert_eq!(first.x_range(), next.x_range());
    assert!(next.top() > first.bottom());
}

#[trial]
fn a_waterfall_heading_clears_all_columns_and_restarts_them_below() {
    let groups = [TileGroup::new(25, 0, 2, "First"), TileGroup::new(26, 2, 1, "Closed").open(false), TileGroup::new(27, 3, 2, "Last")];
    let layout = waterfall(&[0.5, 2.0, 1.0, 1.0, 0.5], &groups, 240.0);
    let laid = layout.arrangement().expect("arrangement");
    let at = |position| layout.rect(position, ScrollPos::default());
    assert!(laid.headers[1].top() > at(1).bottom().max(at(2).bottom()));
    assert_eq!(at(4), Rect::NOTHING, "closed group tiles keep their position but have no bounds");
    assert!(at(5).left() <= at(6).left() && at(5).right() >= at(7).right(), "the heading spans both columns");
    assert_eq!(at(6).top(), at(7).top(), "both columns restart together");
    assert!(at(6).top() >= at(5).bottom());
    let offset = ScrollPos { x: 0.0, y: f64::from(at(1).center().y) };
    let mut shown = Vec::new();
    layout.visible(offset, vec2(240.0, 20.0), &mut shown);
    assert_eq!(shown, [0, 1], "the visible tile and its pinned heading keep their outline positions");
    assert_eq!(layout.hit(pos2(20.0, 5.0), offset), Some(0), "the pinned heading takes the hit ahead of its tile");
}

#[trial]
fn waterfall_lookup_culls_both_axes_and_keeps_outline_order() {
    let aspects: Vec<_> = [0.25, 2.0, 1.0, 1.0].into_iter().cycle().take(160).collect();
    let layout = waterfall(&aspects, &[], 240.0);
    let zero = ScrollPos::default();
    let point = layout.rect(2, zero).center();
    let offset = ScrollPos { x: 0.0, y: f64::from(point.y) };
    let mut found = Vec::new();
    layout.visible(offset, vec2(240.0, 1.0), &mut found);
    assert_eq!(found, [0, 2], "the tall tile in the other column is still visible");
    let offset = ScrollPos { x: f64::from(point.x), ..offset };
    found.clear();
    layout.visible(offset, vec2(1.0, 1.0), &mut found);
    assert_eq!(found, [2]);
    assert_eq!(layout.hit(pos2(0.0, 0.0), offset), Some(2));
    assert_eq!(layout.hit(pos2(0.0, layout.rect(2, offset).bottom() + 1.0), offset), None, "a gap is not a hit");
}

const PHOTOS: [&str; 4] = ["Portrait", "Landscape", "Unknown shape", "Square"];

struct Photos;

impl DisplayOrder<usize> for Photos {
    fn len(&self) -> usize {
        PHOTOS.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < PHOTOS.len()).then_some(*key)
    }
}

impl ItemSource<usize> for Photos {
    fn size_hint(&self, at: usize) -> SizeHint {
        [SizeHint::Aspect(0.5), SizeHint::Aspect(2.0), SizeHint::Auto, SizeHint::Aspect(1.0)][at]
    }
}

#[trial]
fn waterfall_arrows_follow_mirrored_geometry_and_focus_the_same_logical_items() {
    for (direction, toward_end, toward_start) in
        [(Direction::Ltr, Key::ArrowRight, Key::ArrowLeft), (Direction::Rtl, Key::ArrowLeft, Key::ArrowRight)]
    {
        let mut fixture = Fixture::new(|ui| {
            let _ = TileGrid::new("Library").waterfall().show_items(ui, &Photos, &mut |key, _| Tile::new(PHOTOS[*key]));
        });
        fixture.direction(direction).room(vec2(350.0, 550.0)).settle();
        let photo = |fixture: &Fixture, at: usize| fixture.by_label(PHOTOS[at]).expect("visible photo");
        let first = photo(&fixture, 0);
        let second = photo(&fixture, 1);
        assert_eq!(first.rect.center().x > second.rect.center().x, direction == Direction::Rtl);
        assert_eq!(photo(&fixture, 2).rect.x_range(), second.rect.x_range(), "third photo is in the shorter column");
        let role = fixture.tree().role_of(first.node).expect("photos have an accessibility role");
        let names: Vec<_> = fixture.by_role(role).into_iter().filter_map(|photo| fixture.tree().name_of(photo.node)).collect();
        assert_eq!(names, PHOTOS, "the accessibility tree keeps logical order");
        fixture.key(Key::Tab, Modifiers::NONE).idle(2);
        for (key, mods, at) in [
            (Key::Home, Modifiers::NONE, 0),
            (toward_end, Modifiers::NONE, 2),
            (Key::ArrowUp, Modifiers::NONE, 1),
            (Key::ArrowDown, Modifiers::NONE, 2),
            (toward_start, Modifiers::SHIFT, 0),
        ] {
            fixture.key(key, mods).idle(2);
            let focus = focused_key(fixture.tree());
            assert_eq!(focus, photo(&fixture, at).node, "{direction:?} {key:?} focused {:?}", fixture.tree().name_of(focus));
        }
        let selected: Vec<_> = fixture.by_role(role).into_iter().filter(|photo| fixture.tree().selected_of(photo.node) == Some(true)).collect();
        assert_eq!(selected.iter().map(|photo| photo.node).collect::<Vec<_>>(), (0..3).map(|at| photo(&fixture, at).node).collect::<Vec<_>>());
        fixture.key(Key::End, Modifiers::NONE).idle(2);
        assert_eq!(focused_key(fixture.tree()), photo(&fixture, 3).node, "End follows the logical order");
    }
}
