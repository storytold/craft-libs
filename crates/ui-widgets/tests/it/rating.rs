//! `Rating`: an app's colour for the lit stars and its own star size.

use craft_theme::Rgba;
use craft_ui::choice::{Rating, StarSize};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, trial};
use egui::{Color32, Rect, Shape, pos2, vec2};

const GOLD: Rgba = Rgba([0xF5, 0xC2, 0x42, 0xFF]);

/// The colours and bounds of the stars a rating of 3 out of 5 paints, and its rect.
fn stars(rating: impl Fn() -> Rating<'static>) -> (Rect, Vec<(Rect, Vec<Color32>)>) {
    let mut frames = Frames::new();
    let (mut rect, mut shapes) = (Rect::NOTHING, Vec::new());
    for _ in 0..3 {
        shapes = frames.frame_shapes(|ui| {
            let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 60.0));
            rect = in_region(ui, region, |ui| rating().show(ui).response.rect);
        });
    }
    let mut stars: Vec<(Rect, Vec<Color32>)> = shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Mesh(mesh) if rect.contains(mesh.calc_bounds().center()) => {
                let mut colors: Vec<Color32> = mesh.vertices.iter().map(|vertex| vertex.color).filter(|color| color.a() > 0).collect();
                colors.dedup();
                Some((mesh.calc_bounds(), colors))
            }
            _ => None,
        })
        .collect();
    stars.sort_by(|a, b| a.0.center().x.total_cmp(&b.0.center().x));
    (rect, stars)
}

#[trial]
fn a_rating_paints_its_lit_stars_in_the_colour_it_is_given() {
    let gold = Color32::from_rgba_unmultiplied(GOLD.0[0], GOLD.0[1], GOLD.0[2], GOLD.0[3]);
    let (_, given) = stars(|| Rating::new(Bind::given(3u8), 5).lit(GOLD));
    assert_eq!(given.len(), 5, "five stars: {given:?}");
    assert!(given[..3].iter().all(|(_, colors)| colors == &vec![gold]), "the three lit stars are gold: {given:?}");
    assert!(given[3..].iter().all(|(_, colors)| !colors.contains(&gold)), "the unlit stars are not: {given:?}");
    let (_, plain) = stars(|| Rating::new(Bind::given(3u8), 5));
    assert!(plain[..3].iter().all(|(_, colors)| !colors.contains(&gold)), "without a colour the lit stars keep the host's ink: {plain:?}");
    assert!(plain[..3].iter().zip(&given[..3]).all(|(plain, given)| plain.0 == given.0), "the colour changes no star's place");
}

#[trial]
fn a_rating_takes_the_star_size_it_is_given() {
    let (rect, painted) = stars(|| Rating::new(Bind::given(3u8), 5).star_size(StarSize::new(14.0, 19.0)));
    assert!((rect.width() - 5.0 * 19.0).abs() < 0.5 && (rect.height() - 19.0).abs() < 0.5, "five 19-point cells: {rect:?}");
    assert_eq!(painted.len(), 5, "five stars: {painted:?}");
    for pair in painted.windows(2) {
        assert!((pair[1].0.center().x - pair[0].0.center().x - 19.0).abs() < 0.5, "stars 19 points apart: {painted:?}");
    }
    assert!(painted.iter().all(|(bounds, _)| bounds.width() <= 14.5 && bounds.height() <= 14.5), "each star within 14 points: {painted:?}");
}
