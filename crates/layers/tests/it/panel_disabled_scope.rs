//! The `LayersPanel` inside a disabled env scope (photocraft#12545: Remove strokes wait): the rows and their parts draw
//! greyed while the scope disables the panel, and they report enabled again once the scope lifts.

use craft_layers::ui::filter::LayerFilter;
use craft_layers::ui::mock::FakeLayers;
use craft_layers::ui::options::LayersSettings;
use craft_layers::ui::panel::LayersPanel;
use craft_layers::ui::thumbs::LayerThumb;
use craft_layers::ui::tree::{LayerInfo, LayerKind};
use craft_theme::{Contrast, LabelColor, Mode, Rgba, contrast_ratio};
use craft_ui::contract::{Bind, Tracked};
use craft_ui::primitives::textures::{FakeThumbSource, ThumbnailOptions, Thumbnails};
use craft_ui::runtime::env::{Env, with_env};
use craft_ui::style::rgba_color;
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Color32, Rect, Shape, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The panel's room: wide enough for a row's parts and inside the 800 by 600 screen of a fixture.
const PANEL: Vec2 = Vec2::new(420.0, 560.0);

/// The label of the row the trials read: it carries a label colour, so its eye box is filled and its thumbnail is a
/// picture, as in a host that shows thumbnails.
const ROW: &str = "Title";

/// What the trial flips between frames, and the panel fill the row's parts read against.
struct Scope {
    enables: Cell<bool>,
    panel: Cell<Color32>,
}

/// The panel as a host runs it: settings, a thumbnail cache with pictures, the filter bind and the document key, shown
/// inside a `with_env` scope whose enabled flag the trial flips between frames.
fn rig(scope: &Rc<Scope>, mode: Mode, contrast: Contrast) -> Fixture {
    let tree = Rc::new(RefCell::new(Tracked::new({
        let mut tree = FakeLayers::default();
        let mut info = LayerInfo::new(ROW, LayerKind::Text);
        info.label = Some(LabelColor::Blue);
        tree.add(None, info, false);
        tree
    })));
    let scope = Rc::clone(scope);
    let settings = LayersSettings::default();
    let mut fixture = Fixture::new(move |ui| {
        scope.panel.set(rgba_color(Env::of(ui).tokens.color.surface.panel));
        let enables = scope.enables.get();
        let ctx = ui.ctx().clone();
        let thumbs = Thumbnails::<LayerThumb<u32>>::in_context(&ctx, "craft-layers-disabled-scope-thumbs", ThumbnailOptions::new());
        let source = FakeThumbSource::<LayerThumb<u32>>::new().opaque(true);
        let mut filter = LayerFilter::default();
        let tree = tree.borrow();
        let panel = LayersPanel::new(&tree).settings(&settings).thumbnails(&thumbs, &source).filter(Bind::bound(&mut filter)).document(7u32);
        with_env(
            ui,
            |set| set.enabled = Some(enables),
            |ui| {
                let _ = panel.show(ui);
            },
        );
    });
    fixture.room(PANEL);
    fixture.theme(mode, contrast);
    fixture.settle();
    fixture
}

/// The colours `shape` paints with: its fills, strokes, mesh vertices and glyph inks.
fn shape_colors(shape: &Shape, out: &mut Vec<Color32>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| shape_colors(shape, out)),
        Shape::Rect(rect) => out.extend([rect.fill, rect.stroke.color]),
        Shape::Circle(circle) => out.extend([circle.fill, circle.stroke.color]),
        Shape::LineSegment { stroke, .. } => out.push(stroke.color),
        Shape::Mesh(mesh) => out.extend(mesh.vertices.iter().map(|vertex| vertex.color)),
        Shape::Text(text) => out.extend(text.galley.rows.iter().flat_map(|row| row.row.visuals.mesh.vertices.iter().map(|vertex| vertex.color))),
        _ => {}
    }
}

/// The ink `ink` as it lands on the panel fill `panel`.
fn on_panel(ink: Color32, panel: Color32) -> Color32 {
    let share = u32::from(ink.a());
    let mix = |ink: u32, panel: u32| u8::try_from((ink * share + panel * (255 - share)) / 255).unwrap_or(0);
    let mixed = Color32::from_rgb(
        mix(u32::from(ink.r()), u32::from(panel.r())),
        mix(u32::from(ink.g()), u32::from(panel.g())),
        mix(u32::from(ink.b()), u32::from(panel.b())),
    );
    Color32::from_rgb(mixed.r(), mixed.g(), mixed.b())
}

/// How strongly the row reads on the panel: the largest contrast its parts have against the panel fill.
fn row_contrast(fixture: &mut Fixture, rect: Rect, panel: Color32) -> f64 {
    let ground = Rgba(panel.to_array());
    let mut colors = Vec::new();
    for shape in fixture.frame_shapes().iter().filter(|shape| rect.contains_rect(shape.visual_bounding_rect())) {
        shape_colors(shape, &mut colors);
    }
    colors.iter().filter(|color| color.a() > 0).map(|color| contrast_ratio(Rgba(on_panel(*color, panel).to_array()), ground)).fold(0.0, f64::max)
}

/// The row node of `ROW`.
fn row(fixture: &Fixture) -> Target {
    fixture.by_label(ROW).unwrap_or_else(|| panic!("no row named {ROW:?}"))
}

/// A scope that disables the panel draws its rows greyed in every theme: each part of the row reads weaker against the
/// panel than the same row drawn while the scope enables the panel.
#[trial]
fn a_disabled_scope_draws_the_layer_rows_greyed() {
    for (mode, contrast) in [(Mode::Light, Contrast::Standard), (Mode::Dark, Contrast::Standard), (Mode::Dark, Contrast::High)] {
        let scope = Rc::new(Scope { enables: Cell::new(true), panel: Cell::new(Color32::TRANSPARENT) });
        let mut fixture = rig(&scope, mode, contrast);
        let panel = scope.panel.get();
        let rect = row(&fixture).rect;
        let strong = row_contrast(&mut fixture, rect, panel);
        assert!(strong > 4.5, "{mode:?} {contrast:?}: the enabled row reads on the panel with text-strength marks: {strong:.1}");

        scope.enables.set(false);
        fixture.idle(2);
        let greyed = row_contrast(&mut fixture, rect, panel);
        assert!(greyed > 1.0, "{mode:?} {contrast:?}: the row still paints its parts while the scope disables the panel");
        assert!(greyed < strong, "{mode:?} {contrast:?}: a disabling scope draws the row greyed: {greyed:.1} against the enabled row's {strong:.1}");
    }
}

/// A row that reported disabled under the scope reports enabled again once the scope lifts: the host's answer landed,
/// and the panel takes picks again.
#[trial]
fn a_row_reports_enabled_again_once_the_disabling_scope_lifts() {
    let scope = Rc::new(Scope { enables: Cell::new(false), panel: Cell::new(Color32::TRANSPARENT) });
    let mut fixture = rig(&scope, Mode::Light, Contrast::Standard);
    assert!(fixture.tree().is_disabled_key(row(&fixture).node), "the row under a disabling scope reports disabled");
    let rect = row(&fixture).rect;
    let panel = scope.panel.get();
    let greyed = row_contrast(&mut fixture, rect, panel);

    scope.enables.set(true);
    fixture.idle(2);
    assert!(!fixture.tree().is_disabled_key(row(&fixture).node), "the row of a lifted scope reports enabled again");
    let strong = row_contrast(&mut fixture, rect, panel);
    assert!(strong > greyed, "the row of a lifted scope draws its parts strong again: {strong:.1} against the greyed {greyed:.1}");
}
