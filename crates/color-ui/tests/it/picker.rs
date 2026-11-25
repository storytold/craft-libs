//! `ColorPicker`, its hex pair field, `ColorReadout` and `ColorChoice`: the edit rules the picker owns.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::Mixed;
use craft_color_ui::ui::{
    ColorPicker, ColorPickerDialog, ColorPickerDialogLayout, ColorPickerLayout, ColorPickerTab, ColorPopover, ColorReadout, ColorWarning,
    PickedColor, PickerLibrary, PickerSwatches,
};
use craft_host::{AppInfo, Host};
use craft_theme::{ColorModel, HsbModel, LabModel, ModelValues, OklchModel, RgbModel, Rgba, Size, Srgba, ThemeId};
use craft_ui::contract::{Bind, Role, ShowContainer as _};
use craft_ui::dialog::{DialogHost, DialogResult, StackDialog, dialog_id};
use craft_ui::layout::RowAlign;
use craft_ui::layout_components::Row;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Rect, vec2};

fn type_into(fixture: &mut Fixture, label: &str, text: &str) {
    let field = fixture.by_label(label).unwrap_or_else(|| panic!("the picker shows a {label} field"));
    fixture.click(&field);
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text(text);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
}

/// Two items with different G, B and alpha, shown by a readout the way a multi-selection host does.
fn mixed_readout(items: Rc<RefCell<[Rgba; 2]>>) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let mut items = items.borrow_mut();
        let out = ColorReadout::new("Color", Bind::given(Mixed::<Rgba>::Mixed)).show(ui);
        for edit in &out.channel {
            if let Some(Some(byte)) = edit.event.value() {
                for item in items.iter_mut() {
                    item.0[edit.channel.index()] = *byte;
                }
            }
        }
        if let Some(color) = out.events.value() {
            items.fill(*color);
        }
    });
    fixture.room(vec2(480.0, 60.0));
    fixture.settle();
    fixture
}

/// A channel typed while the colour is mixed sets that channel of every item and keeps each item's other channels
/// and alpha; a hex value sets the whole colour of both.
#[trial]
fn a_channel_typed_into_a_mixed_readout_keeps_each_items_other_channels() {
    let items = Rc::new(RefCell::new([Rgba([10, 20, 30, 100]), Rgba([40, 50, 60, 200])]));
    let mut fixture = mixed_readout(items.clone());
    type_into(&mut fixture, "Red", "255");
    assert_eq!(*items.borrow(), [Rgba([255, 20, 30, 100]), Rgba([255, 50, 60, 200])]);
    type_into(&mut fixture, "Hex color", "336699");
    let whole = items.borrow()[0];
    assert_eq!(&whole.0[..3], &[0x33, 0x66, 0x99], "the hex sets the colour");
    assert_eq!(items.borrow()[1], whole, "and sets it for both items");
}

/// What the dialog recalls on open: Only Web Colors snaps the opening colour and the compare swatch, so OK without an
/// edit changes nothing; with it off, the model values `kept` gives stay on the current and the opening colour.
#[trial]
fn a_dialog_reopened_keeps_only_web_colors_and_recalls_the_values_kept_on_ok() {
    let dialogs = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&dialogs);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(vec2(900.0, 700.0));
    fixture.host(std::sync::Arc::new(Host::builder(AppInfo { id: "picker-test", name: "Picker test", version: "0" }).build()));
    let ctx = fixture.ctx().clone();
    let id = dialog_id(("color-picker", "fill"));
    let opened = Srgba::from_hex("#123456").expect("a colour");

    dialogs.borrow_mut().open(ColorPickerDialog::new("fill", "Fill", opened));
    fixture.idle(6);
    let set = dialogs.borrow_mut().set_json_field(&ctx, id, "webColors", serde_json::json!(true));
    assert_eq!(set, Ok(()), "Only Web Colors is a field of the dialog");
    assert!(dialogs.borrow_mut().confirm::<ColorPickerDialog>(&ctx), "OK answers");
    assert!(dialogs.borrow_mut().take::<ColorPickerDialog>().is_some());

    dialogs.borrow_mut().open(ColorPickerDialog::new("fill", "Fill", opened));
    fixture.idle(6);
    let hex = |picked: Option<PickedColor>| picked.map(|picked| picked.color.to_hex());
    let (opening, current) = dialogs.borrow().get::<ColorPickerDialog>().map(|d| (hex(d.opening()), hex(d.current()))).unwrap_or_default();
    assert_eq!(opening.as_deref(), Some("#003366"), "the recalled Only Web Colors snaps the opening colour");
    assert_eq!(current, opening, "and the colour OK would answer");
    // Turn it off so the next open starts plain; OK stores that.
    let set = dialogs.borrow_mut().set_json_field(&ctx, id, "webColors", serde_json::json!(false));
    assert_eq!(set, Ok(()));
    assert!(dialogs.borrow_mut().confirm::<ColorPickerDialog>(&ctx));
    let answered = dialogs.borrow_mut().take::<ColorPickerDialog>().and_then(DialogResult::ok).map(|result| hex(Some(result.picked)));
    assert_eq!(answered, Some(opening), "OK without an edit answers the snapped colour");

    let values = ModelValues::new("cmyk", &[10.0, 20.0, 30.0, 40.0]);
    dialogs.borrow_mut().open(ColorPickerDialog::new("fill", "Fill", opened).kept(values));
    fixture.idle(6);
    let (opening, current) = dialogs.borrow().get::<ColorPickerDialog>().map(|d| (d.opening(), d.current())).unwrap_or_default();
    assert_eq!(opening.and_then(|picked| picked.model), Some(values), "the compare swatch's colour keeps the typed values");
    assert_eq!(current.and_then(|picked| picked.model), Some(values), "and so does the current colour");

    // Current (the compare swatch's current half) and Reset return to the opening colour with its typed values.
    let edited =
        |dialogs: &Rc<RefCell<DialogHost>>| dialogs.borrow().get::<ColorPickerDialog>().and_then(|d| d.current()).and_then(|picked| picked.model);
    let set = dialogs.borrow_mut().set_json_field(&ctx, id, "color", serde_json::json!("#ff0000"));
    assert_eq!(set, Ok(()), "the colour is a field of the dialog");
    fixture.idle(4);
    assert_ne!(edited(&dialogs), Some(values), "the edit replaces the typed values");
    let restore = fixture.by_label("Restore current color").expect("the compare swatch's current half");
    fixture.click(&restore).idle(4);
    assert_eq!(edited(&dialogs), Some(values), "Current returns the typed values");

    let set = dialogs.borrow_mut().set_json_field(&ctx, id, "color", serde_json::json!("#ff0000"));
    assert_eq!(set, Ok(()));
    fixture.idle(4);
    assert_ne!(edited(&dialogs), Some(values));
    fixture.hold(Modifiers::ALT).idle(2);
    let reset = fixture.by_label("Reset").expect("Alt turns Cancel into Reset");
    fixture.click(&reset).idle(4);
    fixture.hold(Modifiers::NONE).idle(2);
    assert_eq!(edited(&dialogs), Some(values), "Reset returns the typed values");
}

/// A working RGB twice as wide as sRGB: 255 is sRGB's half.
struct WideRgb;

impl ColorModel for WideRgb {
    fn id(&self) -> &'static str {
        RgbModel.id()
    }
    fn name(&self) -> &'static str {
        RgbModel.name()
    }
    fn channels(&self) -> &[craft_theme::Channel] {
        static RGB: RgbModel = RgbModel;
        RGB.channels()
    }
    fn from_srgb(&self, c: Srgba) -> ModelValues {
        ModelValues::new(self.id(), &[c.r * 510.0, c.g * 510.0, c.b * 510.0])
    }
    fn to_srgb(&self, v: &[f32; craft_theme::MAX_CHANNELS]) -> Srgba {
        Srgba::rgb(v[0] / 510.0, v[1] / 510.0, v[2] / 510.0)
    }
    fn default_on(&self) -> craft_theme::DefaultOn {
        RgbModel.default_on()
    }
}

/// The dialog's RGB fields are the host's working RGB, in both layouts: the values `kept` gives in it stand, where the
/// built-in sRGB would show its own numbers.
#[trial]
fn the_dialogs_rgb_fields_show_the_hosts_working_values() {
    let wide = Srgba::rgb(0.3, 0.2, 0.1);
    let kept = WideRgb.from_srgb(wide);
    for layout in [ColorPickerDialogLayout::Full, ColorPickerDialogLayout::Tabbed] {
        let dialogs = Rc::new(RefCell::new(DialogHost::new()));
        let shown = Rc::clone(&dialogs);
        let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
        fixture.room(vec2(900.0, 700.0));
        let dialog = ColorPickerDialog::new("fill", "Fill", wide).layout(layout).tab(ColorPickerTab::Custom).working_rgb(WideRgb).kept(kept);
        dialogs.borrow_mut().open(dialog);
        fixture.idle(6);
        if layout == ColorPickerDialogLayout::Tabbed {
            let ctx = fixture.ctx().clone();
            let set = dialogs.borrow_mut().set_json_field(&ctx, dialog_id(("color-picker", "fill")), "model", serde_json::json!("rgb"));
            assert_eq!(set, Ok(()), "the compact picker shows RGB");
            fixture.idle(4);
        }
        let tree = fixture.tree();
        let red = tree.with_role(Role::SPIN_BUTTON).into_iter().find(|node| tree.name_of(*node) == Some("Red"));
        let shown = red.and_then(|node| tree.numeric_of(node)).and_then(|numeric| numeric.value);
        let shown = shown.unwrap_or_else(|| panic!("{layout:?}: the Red field"));
        assert!((shown - f64::from(kept.values[0])).abs() < 0.5, "{layout:?}: Red shows the working value {kept:?}, not {shown}");
    }
}

/// The densities and themes every layout trial runs in: compact and comfortable, a Pro and a Studio theme.
const LOOKS: [(Size, &str); 4] =
    [(Size::Compact, "craft.pro"), (Size::Comfortable, "craft.pro"), (Size::Compact, "craft.studio"), (Size::Comfortable, "craft.studio")];

/// Half a point: less than a pixel at any scale.
const SLACK: f32 = 0.5;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < SLACK
}

fn overlap(a: Rect, b: Rect) -> bool {
    let both = a.intersect(b);
    both.width() > SLACK && both.height() > SLACK
}

/// The rect of every node named `name` together: a warning chip and its swatch share the warning's tip, so this is the
/// whole chip, icon over swatch.
fn named(fixture: &Fixture, name: &str) -> Rect {
    let tree = fixture.tree();
    let rects = tree.keys().into_iter().filter(|key| tree.name_of(*key) == Some(name)).filter_map(|key| tree.bounds_of(key));
    rects.reduce(Rect::union).unwrap_or_else(|| panic!("the picker shows {name}"))
}

/// The Full picker's parts, as AccessKit and the painted text show them.
#[derive(Debug, PartialEq)]
struct FullParts {
    area: Rect,
    strip: Rect,
    /// The compare block with its captions and warnings column.
    block: Rect,
    /// The compare block's current half.
    current: Rect,
    new_caption: Rect,
    radios: Vec<Rect>,
    fields: Vec<Rect>,
    hex: Rect,
    hash: Rect,
    web: Rect,
    sample: Rect,
    warnings: Vec<Rect>,
    /// The unit texts ("°", "%").
    units: Vec<Rect>,
}

impl FullParts {
    fn of(fixture: &Fixture, warnings: usize) -> FullParts {
        let rect = |label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("the picker shows {label}")).rect;
        let text = |text: &str| fixture.texts().iter().filter(|run| run.text == text).map(|run| run.rect).collect::<Vec<_>>();
        let area = rect("Saturation and brightness");
        // The area's point picker reports its two axes as sliders; the strip is the slider after the area.
        let strips: Vec<Rect> = fixture.by_role(Role::SLIDER).into_iter().map(|t| t.rect).filter(|r| r.left() >= area.right() - SLACK).collect();
        assert_eq!(strips.len(), 1, "one slider, the strip, stands after the area: {strips:?}");
        let tips = ["Out of gamut for printing", "Not a web color"];
        FullParts {
            area,
            strip: strips[0],
            block: rect("New color"),
            current: rect("Restore current color"),
            new_caption: *text("new").first().expect("the new caption"),
            radios: fixture.by_role(Role::RADIO_BUTTON).into_iter().map(|t| t.rect).collect(),
            fields: fixture.by_role(Role::SPIN_BUTTON).into_iter().map(|t| t.rect).collect(),
            hex: rect("Hex color"),
            hash: *text("#").first().expect("the # before the hex field"),
            web: rect("Only Web Colors"),
            sample: rect("Sample color from the screen"),
            warnings: tips[..warnings].iter().map(|tip| named(fixture, tip)).collect(),
            units: [text("\u{b0}"), text("%")].concat(),
        }
    }

    /// Every part's rect, for the pairwise overlap check.
    fn all(&self) -> Vec<Rect> {
        let one = [self.area, self.strip, self.block, self.hex, self.hash, self.web, self.sample];
        one.into_iter().chain(self.radios.iter().copied()).chain(self.fields.iter().copied()).chain(self.units.iter().copied()).collect()
    }
}

/// Every part a Full picker shows, found by role and name without requiring any: what a narrow room leaves of it.
fn fixture_parts(fixture: &Fixture) -> Vec<Rect> {
    let labelled = ["Saturation and brightness", "New color", "Hex color", "Only Web Colors", "Sample color from the screen"];
    let named = labelled.iter().filter_map(|label| fixture.by_label(label)).map(|t| t.rect);
    let roles = [Role::SLIDER, Role::RADIO_BUTTON, Role::SPIN_BUTTON].into_iter().flat_map(|role| fixture.by_role(role)).map(|t| t.rect);
    let texts = fixture.texts().iter().filter(|run| ["#", "\u{b0}", "%"].contains(&run.text.as_str())).map(|run| run.rect);
    let mut all: Vec<Rect> = named.chain(roles).chain(texts).collect();
    all.sort_by(|a, b| a.min.x.total_cmp(&b.min.x).then(a.min.y.total_cmp(&b.min.y)));
    all
}

/// A Full picker with HSB, RGB, Lab and OKLCH in a room `width` wide, at `density` in `theme`, with the first
/// `warnings` of the out-of-gamut and web-colour warnings.
fn full_picker(width: f32, (density, theme): (Size, &str), warnings: usize) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let models: [&dyn ColorModel; 4] = [&HsbModel, &RgbModel, &LabModel, &OklchModel];
        let mut color = PickedColor::rgb(Srgba::rgb(0.2, 0.4, 0.6));
        let all = [
            ColorWarning::new("Out of gamut for printing", Srgba::rgb(0.2, 0.4, 0.5)),
            ColorWarning::web_colors("Not a web color", Srgba::rgb(0.2, 0.4, 0.6)),
        ];
        let _ = ColorPicker::new("Color", Bind::bound(&mut color))
            .layout(ColorPickerLayout::Full)
            .models(models)
            .compare(PickedColor::rgb(Srgba::rgb(0.9, 0.2, 0.1)))
            .warnings(all.into_iter().take(warnings))
            .eyedropper(true)
            .show(ui);
    });
    fixture.density(density).theme_id(ThemeId::parse(theme).expect("a theme id")).room(vec2(width, 700.0));
    fixture.settle();
    fixture
}

/// The Full picker has one arrangement, Photoshop's, in every room, density and theme: nothing wraps, stacks or
/// stretches; the area is square with the strip beside it; the compare block's halves are one block taller than wide
/// with its warnings column after it from its top; both field groups start under the compare block on one row; the
/// fields of a group share x and width with their units after them; the "#" stands before the hex field in the radio
/// column; Only Web Colors and the screen-pick button share the row under the area; and no two parts overlap.
#[trial]
fn the_full_picker_keeps_photoshops_arrangement_in_every_room_density_and_theme() {
    for look in LOOKS {
        let wide = full_picker(600.0, look, 2);
        let parts = FullParts::of(&wide, 2);
        // The rooms are compared in one look, the widest: each look's own arrangement is checked below.
        if look == ROOMS_LOOK {
            the_arrangement_holds_in_other_rooms(look, &wide, &parts);
        }
        the_area_strip_and_compare_block_stand_as_in_photoshop(look, &parts);
        the_field_groups_stand_under_the_compare_block(look, &parts);
        the_hex_field_and_the_rows_under_the_area_stand_as_in_photoshop(look, &parts);
        for (i, a) in parts.all().iter().enumerate() {
            for b in &parts.all()[i + 1..] {
                assert!(!overlap(*a, *b), "{look:?}: no two parts overlap: {a:?} {b:?}");
            }
        }
    }
}

type Look = (Size, &'static str);

/// The look the other rooms are compared in: the comfortable density is the widest, so a narrow room presses it most.
const ROOMS_LOOK: Look = (Size::Comfortable, "craft.studio");

/// Wider and narrower rooms leave the arrangement alone, and the warnings move no other part.
fn the_arrangement_holds_in_other_rooms(look: Look, wide: &Fixture, parts: &FullParts) {
    assert_eq!(&FullParts::of(&full_picker(1200.0, look, 2), 2), parts, "{look:?}: the same arrangement in a room 1200 wide");
    // A room narrower than the picker clips it: every part it shows stands where it stands in a wide room.
    let inside = |rects: Vec<Rect>| rects.into_iter().filter(|r| r.right() <= 320.0).collect::<Vec<_>>();
    let narrow = inside(fixture_parts(&full_picker(320.0, look, 2)));
    assert!(!narrow.is_empty(), "{look:?}: a narrow room shows the picker's start");
    assert_eq!(narrow, inside(fixture_parts(wide)), "{look:?}: the same arrangement in a room 320 wide, clipped");
    let without = FullParts::of(&full_picker(600.0, look, 0), 0);
    assert_eq!(&FullParts { warnings: parts.warnings.clone(), ..without }, parts, "{look:?}: the warnings move no other part");
}

fn the_area_strip_and_compare_block_stand_as_in_photoshop(look: Look, parts: &FullParts) {
    let FullParts { area, strip, block, current, new_caption, ref fields, ref warnings, .. } = *parts;
    assert!(near(area.width(), area.height()), "{look:?}: the area is square: {area:?}");
    assert!(near(strip.top(), area.top()) && near(strip.height(), area.height()), "{look:?}: the strip is as tall as the area: {strip:?}");
    assert!(strip.left() >= area.right() - SLACK, "{look:?}: the strip stands after the area");

    // The compare block: the current half is the lower of two halves of one block, wider than a control is high.
    let control = fields[0].height();
    assert!(current.width() > control, "{look:?}: the block is wider than one control: {current:?}");
    assert!(2.0 * current.height() > current.width(), "{look:?}: two halves make it taller than wide: {current:?}");
    assert!(near(block.top(), area.top()), "{look:?}: the new caption stands level with the area's top: {block:?} {area:?}");
    assert!(new_caption.bottom() <= current.top() - current.height() + SLACK, "{look:?}: the new caption is over the new half");
    let new_top = current.top() - current.height();
    assert!(near(warnings[0].top(), new_top), "{look:?}: the first warning is level with the block's top: {:?}", warnings[0]);
    assert!(warnings.iter().all(|w| w.left() >= current.right() - SLACK), "{look:?}: the warnings stand after the block");
    assert!(warnings[0].bottom() <= warnings[1].top() + SLACK, "{look:?}: one warning under the other");
}

/// Two groups: HSB and RGB at one x, Lab and OKLCH at another, starting on one row under the compare block.
fn the_field_groups_stand_under_the_compare_block(look: Look, parts: &FullParts) {
    let FullParts { current, ref fields, ref units, .. } = *parts;
    let mut starts: Vec<f32> = Vec::new();
    for field in fields {
        if !starts.iter().any(|start| near(*start, field.left())) {
            starts.push(field.left());
        }
    }
    starts.sort_by(f32::total_cmp);
    assert_eq!(starts.len(), 2, "{look:?}: the fields start in two groups: {fields:?}");
    let group = |at: usize| fields.iter().filter(|f| near(f.left(), starts[at])).copied().collect::<Vec<_>>();
    let (first, second) = (group(0), group(1));
    let top = |rects: &[Rect]| rects.iter().map(|r| r.top()).fold(f32::MAX, f32::min);
    assert!(near(top(&first), top(&second)), "{look:?}: the second group starts on the first group's first row");
    assert!(top(fields) >= current.bottom(), "{look:?}: the groups start under the compare block");
    for group in [&first, &second] {
        assert!(group.iter().all(|f| near(f.width(), group[0].width())), "{look:?}: a group's fields share a width: {group:?}");
        assert!(group[0].width() < parts.hex.width(), "{look:?}: the fields are narrower than the hex field");
    }
    for unit in units {
        assert!(
            fields.iter().any(|f| near(f.center().y, unit.center().y) && unit.left() >= f.right() - SLACK),
            "{look:?}: a unit stands after its field, outside it: {unit:?}"
        );
    }
}

fn the_hex_field_and_the_rows_under_the_area_stand_as_in_photoshop(look: Look, parts: &FullParts) {
    let FullParts { area, ref radios, ref fields, hex, hash, web, sample, .. } = *parts;
    let first_left = fields.iter().map(|f| f.left()).fold(f32::MAX, f32::min);
    let first_radio = radios.iter().map(|r| r.left()).fold(f32::MAX, f32::min);
    assert!(hash.left() >= first_radio - SLACK && hash.right() <= first_left, "{look:?}: the # stands in the radio column: {hash:?}");
    assert!(hex.left() > hash.right() && hex.left() < first_left, "{look:?}: the hex field starts in the letter column: {hex:?}");

    assert!(web.top() >= area.bottom() && near(web.left(), area.left()), "{look:?}: Only Web Colors stands under the area at its start");
    assert!(sample.left() > web.right() && near(sample.center().y, web.center().y), "{look:?}: the screen-pick button ends Only Web Colors' row");
}

/// The Color Picker dialog takes the picker's size, its buttons in the picker's actions cell over the second field
/// group, no end column beside it: in Photoshop's proportions, about twice as wide as the area and one and a half
/// times as tall.
#[trial]
fn the_color_picker_dialog_stands_its_buttons_over_the_second_group_in_photoshops_proportions() {
    for look in LOOKS {
        let dialogs = Rc::new(RefCell::new(DialogHost::new()));
        let shown = Rc::clone(&dialogs);
        let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
        fixture.density(look.0).theme_id(ThemeId::parse(look.1).expect("a theme id")).room(vec2(1200.0, 900.0));
        dialogs.borrow_mut().open(ColorPickerDialog::new("fill", "Fill", Srgba::rgb(0.2, 0.4, 0.6)));
        fixture.idle(6);
        let rect = |label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("the dialog shows {label}")).rect;
        let dialog = fixture.by_role(Role::DIALOG).first().expect("the dialog").rect;
        let (area, ok, cancel, add) = (rect("Saturation and brightness"), rect("OK"), rect("Cancel"), rect("Add to Swatches"));
        let radios: Vec<Rect> = fixture.by_role(Role::RADIO_BUTTON).into_iter().map(|t| t.rect).collect();
        let fields: Vec<Rect> = fixture.by_role(Role::SPIN_BUTTON).into_iter().map(|t| t.rect).collect();
        let second = radios.iter().map(|r| r.left()).fold(f32::MIN, f32::max);
        assert!(ok.bottom() <= cancel.top() && cancel.bottom() <= add.top(), "{look:?}: OK, Cancel, then Add to Swatches");
        assert!(near(ok.left(), second), "{look:?}: the buttons start at the second group's edge: {ok:?} against {second}");
        assert!(fields.iter().all(|f| f.top() >= add.bottom()), "{look:?}: the groups start under the buttons");
        assert!(ok.right() <= dialog.right() && dialog.right() - ok.right() < ok.width(), "{look:?}: no button column stands beside the body");
        let wide = dialog.width() / area.width();
        let tall = dialog.height() / area.width();
        assert!((1.9..=2.3).contains(&wide), "{look:?}: the dialog is about twice as wide as the area: {wide}");
        assert!((1.4..=1.8).contains(&tall), "{look:?}: and about one and a half times as tall: {tall}");
    }
}

/// The Color Swatches button switches the dialog to the swatch view, which marks the cell of the current colour; a click
/// on another cell is an edit that OK answers. A remembered library past the end shows the last library, and an empty
/// library shows the notice.
#[trial]
fn the_color_picker_dialogs_swatch_view_marks_the_current_colour_and_a_pick_is_an_edit() {
    let (red, blue, green) = (Srgba::rgb(1.0, 0.0, 0.0), Srgba::rgb(0.0, 0.0, 1.0), Srgba::rgb(0.0, 1.0, 0.0));
    let library = |name: &'static str, swatches: &[(&'static str, Srgba)]| {
        PickerLibrary::new(name, swatches.iter().map(|(name, color)| (Label::from(*name), *color)).collect())
    };
    let open = |libraries: Vec<PickerLibrary>| {
        let dialogs = Rc::new(RefCell::new(DialogHost::new()));
        let shown = Rc::clone(&dialogs);
        let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
        fixture.room(vec2(900.0, 700.0));
        let dialog = ColorPickerDialog::new("fill", "Fill", red).swatch_view(PickerSwatches::new(libraries, "No swatches"));
        dialogs.borrow_mut().open(dialog);
        fixture.idle(6);
        (dialogs, fixture)
    };
    let id = dialog_id(("color-picker", "fill"));

    let (dialogs, mut fixture) = open(vec![library("Basic", &[("Red", red), ("Blue", blue)])]);
    let toggle = fixture.by_label("Color Swatches").expect("the dialog shows the Color Swatches button");
    fixture.click(&toggle).idle(4);
    let selected = |fixture: &Fixture, name: &str| {
        let cell = fixture.by_label(name).unwrap_or_else(|| panic!("the swatch view shows {name}"));
        fixture.tree().selected_of(cell.node).unwrap_or(false)
    };
    assert!(selected(&fixture, "Red"), "the cell equal to the current colour is marked");
    assert!(!selected(&fixture, "Blue"), "and no other");
    let blue_cell = fixture.by_label("Blue").expect("the blue cell");
    fixture.click(&blue_cell).idle(4);
    let current = dialogs.borrow().get::<ColorPickerDialog>().and_then(|d| d.current()).map(|picked| picked.color.to_hex());
    assert_eq!(current, Some(blue.to_hex()), "a click on a cell edits the colour");
    let ctx = fixture.ctx().clone();
    assert!(dialogs.borrow_mut().confirm::<ColorPickerDialog>(&ctx));
    let answered = dialogs.borrow_mut().take::<ColorPickerDialog>().and_then(DialogResult::ok).map(|result| result.picked.color.to_hex());
    assert_eq!(answered, Some(blue.to_hex()), "OK answers the picked colour");

    let (dialogs, mut fixture) = open(vec![library("Basic", &[("Red", red)]), library("Spare", &[("Green", green)])]);
    let ctx = fixture.ctx().clone();
    assert_eq!(dialogs.borrow_mut().set_json_field(&ctx, id, "swatches", serde_json::json!(true)), Ok(()));
    assert_eq!(dialogs.borrow_mut().set_json_field(&ctx, id, "library", serde_json::json!(9)), Ok(()));
    fixture.idle(4);
    assert!(fixture.by_label("Green").is_some(), "a library past the end shows the last library");

    let (dialogs, mut fixture) = open(vec![library("Empty", &[])]);
    let ctx = fixture.ctx().clone();
    assert_eq!(dialogs.borrow_mut().set_json_field(&ctx, id, "swatches", serde_json::json!(true)), Ok(()));
    fixture.idle(4);
    assert!(fixture.texts().iter().any(|run| run.text == "No swatches"), "an empty library shows the notice");
}

/// The Compact picker's width is set by the area and the strip, and its rows take that width; its popover is as wide
/// wherever the well stands and whatever the room.
#[trial]
fn the_compact_picker_is_as_wide_as_its_area_and_strip_wherever_it_opens() {
    let span = |width: f32, align: RowAlign| {
        let mut fixture = Fixture::new(move |ui| {
            let mut color = PickedColor::rgb(Srgba::rgb(0.2, 0.4, 0.6));
            Row::new().fill_width().align(align).show(ui, |ui, _| {
                let _ = ColorPopover::new("Fill", Bind::bound(&mut color)).picker(|picker| picker.eyedropper(true)).show(ui);
            });
        });
        fixture.room(vec2(width, 700.0));
        fixture.settle();
        let well = fixture.by_label("Fill").expect("the well");
        fixture.click(&well).idle(4);
        let area = fixture.by_label("Saturation and brightness").expect("the popover's area").rect;
        let hex = fixture.by_label("Hex color").expect("the hex field").rect;
        let sample = fixture.by_label("Sample color from the screen").expect("the screen-pick button").rect;
        assert!(hex.left() >= area.left() - SLACK, "the hex row starts under the area");
        (area.size(), sample.right() - area.left())
    };
    let first = span(600.0, RowAlign::Start);
    for (width, align) in [(600.0, RowAlign::End), (1200.0, RowAlign::Start), (1200.0, RowAlign::End)] {
        assert_eq!(span(width, align), first, "the popover is as wide in a room {width} wide with the well at {align:?}");
    }
    assert!(first.0.x > first.0.y, "the compact area is wider than tall");
    assert!(first.1 > first.0.x, "the rows reach past the area, to the strip's end");
}

/// A field of the open dialog as automation reads it.
fn dialog_field(dialogs: &Rc<RefCell<DialogHost>>, name: &str) -> Option<serde_json::Value> {
    dialogs.borrow().get::<ColorPickerDialog>().and_then(|dialog| dialog.json_fields()).and_then(|fields| fields.get(name).cloned())
}

/// The current colour of the open dialog as `#rrggbb`.
fn dialog_color(dialogs: &Rc<RefCell<DialogHost>>) -> Option<String> {
    dialogs.borrow().get::<ColorPickerDialog>().and_then(|dialog| dialog.current()).map(|picked| picked.color.to_hex())
}

/// The tabbed dialog opens on the tab it was given; a hex typed on Custom and a honeycomb cell picked on Standard both
/// reach the answer, and the tab is a field automation reads and sets.
#[trial]
fn the_tabbed_colors_dialog_opens_on_its_tab_and_answers_from_either_page() {
    let dialogs = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&dialogs);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(vec2(900.0, 700.0));
    fixture.host(std::sync::Arc::new(Host::builder(AppInfo { id: "picker-test", name: "Picker test", version: "0" }).build()));
    let ctx = fixture.ctx().clone();
    let id = dialog_id(("color-picker", "fill"));
    let colour = |hex: &str| Srgba::from_hex(hex).expect("a colour");
    let cells: Vec<(Label, Srgba)> = ["#ff0000", "#00ff00", "#0000ff", "#ffff00"]
        .iter()
        .enumerate()
        .map(|(at, hex)| (Label::from(format!("Standard {at}")), colour(hex)))
        .collect();
    let greys = vec![(Label::from("Gray 1"), colour("#808080"))];

    dialogs.borrow_mut().open(
        ColorPickerDialog::new("fill", "Fill", colour("#123456"))
            .layout(ColorPickerDialogLayout::Tabbed)
            .standard(cells, greys)
            .tab(ColorPickerTab::Custom)
            .web_colors_remembered(false),
    );
    fixture.idle(6);
    assert_eq!(dialog_field(&dialogs, "tab"), Some(serde_json::json!("custom")), "the dialog opens on the tab it was given");
    assert!(fixture.by_label("Hex color").is_some(), "the Custom page shows the compact picker");
    assert!(fixture.by_label("Standard 0").is_none(), "and not the honeycomb");
    assert_eq!(dialog_field(&dialogs, "model"), Some(serde_json::Value::Null), "no model is picked yet");
    assert!(fixture.by_label("Red").is_none(), "the first model (HSB) shows no red channel");
    let set = dialogs.borrow_mut().set_json_field(&ctx, id, "model", serde_json::json!(RgbModel.id()));
    assert_eq!(set, Ok(()), "the model of the Custom page is a field");
    fixture.idle(6);
    assert_eq!(dialog_field(&dialogs, "model"), Some(serde_json::json!(RgbModel.id())), "the field names the model shown");
    assert!(fixture.by_label("Red").is_some(), "and the compact rows show that model's channels");

    type_into(&mut fixture, "Hex color", "336699");
    assert_eq!(dialog_color(&dialogs).as_deref(), Some("#336699"), "a hex typed on Custom is the colour");

    let set = dialogs.borrow_mut().set_json_field(&ctx, id, "tab", serde_json::json!("standard"));
    assert_eq!(set, Ok(()), "the tab is a field of the dialog");
    fixture.idle(6);
    assert!(fixture.by_label("Hex color").is_none(), "Standard shows no hex field");
    let cell = fixture.by_label("Standard 2").expect("the honeycomb shows its cells");
    fixture.click(&cell);
    fixture.idle(4);
    assert_eq!(dialog_color(&dialogs).as_deref(), Some("#0000ff"), "a honeycomb pick is the colour");

    assert!(dialogs.borrow_mut().confirm::<ColorPickerDialog>(&ctx), "OK answers");
    let answered = dialogs.borrow_mut().take::<ColorPickerDialog>().and_then(DialogResult::ok).map(|result| result.picked.color.to_hex());
    assert_eq!(answered.as_deref(), Some("#0000ff"));
}

/// The Full dialog opens with its hex field focused and selected; digits typed there and Enter commit the colour and
/// confirm the dialog, as in the reference image editor's Color Picker (the tabbed Colors dialog keeps Enter in the
/// field instead).
#[trial]
fn enter_in_the_full_dialogs_hex_field_confirms_the_typed_colour() {
    let dialogs = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&dialogs);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(vec2(900.0, 700.0));
    fixture.host(std::sync::Arc::new(Host::builder(AppInfo { id: "picker-test", name: "Picker test", version: "0" }).build()));
    let colour = |hex: &str| Srgba::from_hex(hex).expect("a colour");
    dialogs
        .borrow_mut()
        .open(ColorPickerDialog::new("foreground", "Color Picker (Foreground Color)", colour("#000000")).web_colors_remembered(false));
    fixture.idle(6);
    assert!(dialogs.borrow().get::<ColorPickerDialog>().is_some(), "the picker is open");

    fixture.type_text("336699");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert!(dialogs.borrow().get::<ColorPickerDialog>().is_none(), "Enter in the hex field confirms the dialog");
    let answer = dialogs.borrow_mut().take::<ColorPickerDialog>();
    let Some(DialogResult::Ok(result)) = answer else { panic!("the dialog answers OK") };
    assert_eq!(result.picked.color.to_hex(), "#336699", "with the typed colour");
}

/// A request that asks for the swatches opens the picker on its Color Swatches view; one that doesn't opens it on the
/// Color Models.
#[trial]
fn a_request_for_the_swatches_opens_the_picker_on_its_swatch_view() {
    let red = Srgba::rgb(1.0, 0.0, 0.0);
    let shows_swatches = |request: craft_ui::runtime::requests::ColorRequest| {
        let dialogs = Rc::new(RefCell::new(DialogHost::new()));
        let shown = Rc::clone(&dialogs);
        let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
        fixture.room(vec2(900.0, 700.0));
        let libraries = vec![PickerLibrary::new("Basic", vec![(Label::from("Brick"), red)])];
        dialogs.borrow_mut().open(ColorPickerDialog::from_request(request).swatch_view(PickerSwatches::new(libraries, "No swatches")));
        fixture.idle(6);
        fixture.by_label("Brick").is_some()
    };
    let request = || craft_ui::runtime::requests::ColorRequest::new("fill", red);
    assert!(shows_swatches(request().on_swatches()), "the swatch view shows the library's cells");
    assert!(!shows_swatches(request()), "without it the picker opens on the Color Models");
}
