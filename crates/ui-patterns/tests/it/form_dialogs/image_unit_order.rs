//! Image Size makes Percent easy to find in both dimension menus.

use craft_behavior::PixelSize;
use craft_ui::contract::Role;
use craft_ui::dialog::{DialogHost, DialogResult};
use craft_ui::patterns::form_dialogs::{CanvasSize, CanvasSizeValues, HostedCanvasSize, HostedImageSize, ImageSize, ImageSizeValues};
use craft_ui::runtime::env::{ColumnSize, UnitKind};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, vec2};
use std::cell::RefCell;
use std::rc::Rc;

const SIZE: PixelSize = PixelSize::new(1200, 900);
const COLUMNS: ColumnSize = ColumnSize::new(180.0, 12.0);
const IMAGE_UNITS: [&str; 8] = ["Percent", "Pixels", "Inches", "Centimetres", "Millimetres", "Points", "Picas", "Columns"];
const CANVAS_UNITS: [&str; 8] = ["Pixels", "Percent", "Inches", "Centimetres", "Millimetres", "Points", "Picas", "Columns"];

fn fixture(host: &Rc<RefCell<DialogHost>>) -> Fixture {
    let host = Rc::clone(host);
    let mut fixture = Fixture::new(move |ui| host.borrow_mut().run(ui));
    fixture.room(vec2(1400.0, 1000.0)).idle(10);
    fixture
}

fn unit_order(fixture: &mut Fixture, axis: &str) -> Vec<String> {
    let field = fixture.by_label(axis).expect("the dimension's unit combobox");
    fixture.access(AccessActionKind::Focus, &field).idle(2);
    fixture.key(Key::Space, Modifiers::NONE).idle(3);
    let mut options = fixture.by_role(Role::LIST_BOX_OPTION);
    options.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
    options.iter().map(|option| fixture.tree().name_of(option.node).expect("the visible option has a name").to_owned()).collect()
}

fn image_host() -> Rc<RefCell<DialogHost>> {
    let mut host = DialogHost::new();
    #[allow(clippy::redundant_closure, reason = "the builder must be generic over the bind's lifetime")]
    let dialog = HostedImageSize::new(ImageSizeValues::new(SIZE).column_size(COLUMNS), |values| ImageSize::new(values));
    host.open(dialog);
    Rc::new(RefCell::new(host))
}

#[trial]
fn image_size_dimension_menus_put_percent_before_pixels() {
    let mut canvas = DialogHost::new();
    #[allow(clippy::redundant_closure, reason = "the builder must be generic over the bind's lifetime")]
    let dialog = HostedCanvasSize::new(CanvasSizeValues::new(SIZE).column_size(COLUMNS), |values| CanvasSize::new(values));
    canvas.open(dialog);
    let mut comparison = fixture(&Rc::new(RefCell::new(canvas)));
    assert_eq!(unit_order(&mut comparison, "Width unit"), CANVAS_UNITS);

    let host = image_host();
    let mut width = fixture(&host);
    let mut height = fixture(&image_host());
    let width_order = unit_order(&mut width, "Width unit");
    let height_order = unit_order(&mut height, "Height unit");
    assert_eq!(width_order, IMAGE_UNITS, "Image Size Width puts Percent first and Columns last");
    assert_eq!(height_order, &IMAGE_UNITS[..7], "Image Size Height puts Percent first and offers no Columns");

    let percent = width.by_label("Percent").expect("Percent is a selectable unit");
    width.click(&percent).idle(4);
    let confirm = width.by_label("OK").expect("the Image Size confirmation");
    width.access(AccessActionKind::Click, &confirm).idle(4);
    let accepted = host.borrow_mut().take::<HostedImageSize>().and_then(DialogResult::ok).expect("Image Size answers after choosing Percent");
    assert_eq!(accepted.unit, UnitKind::Percent);
    assert_eq!(accepted.size, SIZE, "switching units preserves the image size");
}
