//! Stories of the buttons.

use super::{Arrangement, Button, ControlPlace, GroupLevel, IconButton, Link, SplitButton, SplitMenu, ToggleButton};
use crate::contract::{BodyCx, Entry, LeastWidth, Props as _, Show as _, ShowContainer as _, Sizing, Validity};
use crate::display::{Picture, Shade, Text};
use crate::layout_components::{Grid, Row, Stack};
use crate::list::ThumbAspect;
use crate::menu::MenuItem;
use crate::primitives::{AppIcons, HAlign, IconMark, IconSpec, register_app_icons};
use crate::runtime::commands::EditCommands;
use crate::runtime::env::{Message, with_env};
use crate::style::{DataColor, SurfaceKind};
use crate::testing::pictures::sample_picture;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{ColumnWidth, IconId, Mixed, Proposal};
use craft_theme::{ControlSize, Variant};
use egui::{Color32, Key, Modifiers, Ui};

const SIZES: [(&str, ControlSize); 2] = [("Sm", ControlSize::Sm), ("Md", ControlSize::Md)];

/// Every variant at each size (rows, each under its name) at rest, disabled and loading (columns). Where the full grid
/// is wider than the room, the loading column is left out.
#[story(Button<'static>, covers = [], drive = Drive::Never, group = "Variants", title = "Variants")]
fn button_variants(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    const ALL: &[ColumnWidth] = &[ColumnWidth::Content; 4];
    const STATES: [(&str, bool, bool); 3] = [("rest", true, false), ("disabled", false, false), ("loading", true, true)];
    let variants = [("Primary", Variant::PRIMARY), ("Secondary", Variant::SECONDARY), ("Ghost", Variant::GHOST), ("Danger", Variant::DANGER)];
    let label = data.value("label", String::from("Button")).clone();
    let rows = |ui: &mut Ui, states: &[(&str, bool, bool)]| {
        for (name, variant) in variants {
            for (size_name, size) in SIZES {
                Text::new(format!("{name} {size_name}").to_lowercase()).shade(Shade::Muted).show(ui);
                for &(state, enabled, loading) in states {
                    Button::new(label.clone())
                        .variant(variant)
                        .size(size)
                        .enabled(enabled)
                        .loading(loading)
                        .id_salt((name, size_name, state))
                        .show(ui);
                }
            }
        }
    };
    let full = Grid::new(ALL).measure_in(ui, Proposal::Ideal, |ui, _| rows(ui, &STATES));
    let (columns, states) = if full.x <= ui.available_width() { (ALL, &STATES[..]) } else { (&ALL[..3], &STATES[..2]) };
    Grid::new(columns).show(ui, |ui, _| rows(ui, states)).into()
}

#[story(Button<'static>, covers = [Default], group = "Playground", title = "Playground")]
fn button(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let variants = [("Primary", Variant::PRIMARY), ("Secondary", Variant::SECONDARY), ("Ghost", Variant::GHOST), ("Danger", Variant::DANGER)];
    let variant = *data.choice("variant", Variant::PRIMARY, &variants);
    let size = *data.choice("size", ControlSize::Md, &SIZES);
    let icon = *data.choice("icon", Some("save"), &[("None", None), ("Save", Some("save")), ("chevron_right", Some("chevron_right"))]);
    let disabled = *data.value("disabled", false);
    let loading = *data.value("loading", false);
    let label = data.value("label", String::from("Save the document as a copy")).clone();
    let mut button = Button::new(label).variant(variant).size(size).enabled(!disabled).loading(loading);
    if let Some(icon) = icon {
        button = button.icon(IconId::new(icon));
    }
    let shown = button.show(ui);
    shown.into()
}

#[story(Button<'static>, covers = [], disabled_text = "Nothing to save", group = "Playground", title = "With reason")]
fn button_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Button::new("Save").tooltip(data.tooltip("Save the document")).show(ui).into()
}

#[story(Button<'static>, covers = [], drive = Drive::Never, group = "Playground", title = "Loading")]
fn button_loading(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Button::new("Saving").icon(IconId::new("save")).loading(true).show(ui).into()
}

#[story(IconButton<'static>, covers = [Default])]
fn icon_button(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    IconButton::new(IconId::new("settings")).label("Settings").show(ui).into()
}

/// A tool strip: the first tool selected, as the gallery card shows it.
#[story(IconButton<'static>, covers = [], drive = Drive::Never)]
fn icon_button_selected(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Row::new().show(ui, |ui, _| {
        for (i, (icon, label)) in
            [("undo", "Undo"), ("redo", "Redo"), ("copy", "Copy"), ("trash", "Delete"), ("settings", "Settings")].into_iter().enumerate()
        {
            IconButton::new(IconId::new(icon)).label(label).selected(i == 0).id_salt(icon).show(ui);
        }
    });
    shown.into()
}

/// Icon buttons with a colour badge on their icon, enabled and then disabled: a disabled button dims each badge and
/// keeps its hue.
#[story(IconButton<'static>, covers = [], drive = Drive::Never, title = "Badge")]
fn icon_button_badge(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    const BADGES: [(&str, Color32); 2] = [("Red", Color32::from_rgb(230, 51, 51)), ("Blue", Color32::from_rgb(51, 102, 230))];
    let shown = Row::new().show(ui, |ui, _| {
        for enabled in [true, false] {
            for (name, color) in BADGES {
                IconButton::new(IconSpec::svg(IconId::new("eyedropper")).badge(DataColor::new(color)))
                    .label(name)
                    .enabled(enabled)
                    .id_salt((name, enabled))
                    .show(ui);
            }
        }
    });
    shown.into()
}

/// Icon buttons with a data-colour bar across the bottom of the icon: the default thickness and a weighted one (a
/// pen's width), enabled and then disabled.
#[story(IconButton<'static>, covers = [], drive = Drive::Never, title = "Bar mark")]
fn icon_button_bar_mark(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let color = DataColor::new(Color32::from_rgb(230, 180, 30));
    let marks = [("Highlight", IconMark::bar(color)), ("Pen", IconMark::weighted_bar(color, 4.0))];
    let shown = Row::new().show(ui, |ui, _| {
        for enabled in [true, false] {
            for (name, mark) in marks {
                IconButton::new(IconSpec::svg(IconId::new("eyedropper")).with_mark(mark))
                    .label(name)
                    .enabled(enabled)
                    .id_salt((name, enabled))
                    .show(ui);
            }
        }
    });
    shown.into()
}

/// A pen drawn in the icon ink, with its tip line in the accent ink (`class="accent"`) and a cap dot in its own colour
/// (`class="fixed"`).
const TWO_INK_PEN: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M5 15 L14 6 L18 10 L9 19 L5 19 Z" fill="black"/><rect class="accent" x="3" y="21" width="18" height="2" fill="black"/><circle class="fixed" cx="18" cy="5" r="2" fill="#d04848"/></svg>"##;

/// The pen's name in [`STORY_ICONS`]; the icon-id scan reads only catalog ids written as literals, so the story names
/// its app icon through this constant.
const TWO_INK_PEN_NAME: &str = "story_two_ink_pen";

/// The stories' own app icon set, under a name no other story or app uses.
static STORY_ICONS: AppIcons = AppIcons::new("ui-widgets-stories", None, &[(TWO_INK_PEN_NAME, TWO_INK_PEN)]);

/// An app icon in two inks and a fixed colour, enabled and then disabled: a disabled button takes the disabled ink in
/// every layer.
#[story(IconButton<'static>, covers = [], drive = Drive::Never, title = "Two-ink icon")]
fn icon_button_two_ink(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    // A second registration of the same set is a no-op, so the story registers it on every run.
    register_app_icons(&STORY_ICONS);
    let shown = Row::new().show(ui, |ui, _| {
        for enabled in [true, false] {
            IconButton::new(IconId::new(TWO_INK_PEN_NAME)).label("Pen").enabled(enabled).id_salt(enabled).show(ui);
        }
    });
    shown.into()
}

/// Stacked buttons, the icon over the label, with a one-line and a two-line label at each size.
#[story(Button<'static>, covers = [], drive = Drive::Never, title = "Stacked")]
fn button_stacked(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Row::new().show(ui, |ui, _| {
        for (size_name, size) in SIZES {
            for label in ["Paste", "Insert page break"] {
                Button::new(label).icon(IconId::new("copy")).arrangement(Arrangement::Stacked).size(size).id_salt((size_name, label)).show(ui);
            }
        }
    });
    shown.into()
}

/// Buttons that keep room for their widest text: "1%" and "500%" with room for "500%" are equally wide, beside "1%"
/// without it.
#[story(Button<'static>, covers = [], drive = Drive::Never, title = "Room for the widest text")]
fn button_room_for(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Row::new().show(ui, |ui, _| {
        for label in ["1%", "500%"] {
            Button::new(label).room_for("500%").variant(Variant::GHOST).id_salt(label).show(ui);
        }
        Button::new("1%").variant(Variant::GHOST).id_salt("no room").show(ui);
    });
    shown.into()
}

/// Stacked buttons with a rendered picture in the icon's place: square and wide.
#[story(Button<'static>, covers = [], drive = Drive::Never, title = "Stacked with a picture")]
fn button_stacked_picture(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Row::new().show(ui, |ui, _| {
        for (label, key, aspect, pixels) in [("Equation", 3, ThumbAspect::SQUARE, [64, 64]), ("Fraction", 5, ThumbAspect::new(2, 1), [128, 64])] {
            let image = move || sample_picture(key, pixels);
            let picture = Picture::new("", key, &image);
            Button::new(label).picture(aspect, picture).arrangement(Arrangement::Stacked).id_salt(label).show(ui);
        }
    });
    shown.into()
}

/// The same inline buttons at full size and inside a compact group, where they drop their labels and keep them as
/// their names and tooltips.
#[story(Button<'static>, covers = [], drive = Drive::Never, title = "Compact in a group")]
fn button_compact_in_group(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let entries = presets();
    let shown = Row::new().show(ui, |ui, _| {
        Button::new("Save").icon(IconId::new("save")).id_salt("full").show(ui);
        SplitButton::new("Trace", &entries).icon(IconId::new("image")).id_salt("full split").show(ui);
        with_env(
            ui,
            |overrides| {
                overrides.values.set(GroupLevel::Compact);
            },
            |ui| {
                Button::new("Save").icon(IconId::new("save")).id_salt("compact").show(ui);
                SplitButton::new("Trace", &entries).icon(IconId::new("image")).id_salt("compact split").show(ui);
            },
        );
    });
    shown.into()
}

/// A page's side menu: full-width ghost buttons on the brand surface, in the `SideMenu` place, at least the side
/// menu's width, one of them current, as a File page's list of pages shows them.
#[story(Button<'static>, covers = [], drive = Drive::Never, title = "In a side menu")]
fn button_in_side_menu(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shown = Stack::new().surface(SurfaceKind::Brand).least_width(LeastWidth::SideMenu).show(ui, |ui, _| {
        with_env(
            ui,
            |overrides| {
                overrides.values.set(ControlPlace::SideMenu);
            },
            |ui| {
                for (name, icon) in [("Home", IconId::new("home")), ("Open", IconId::new("open")), ("Save", IconId::new("save"))] {
                    Button::new(name)
                        .icon(icon)
                        .variant(Variant::GHOST)
                        .sizing(Sizing::FillWidth)
                        .align(HAlign::Start)
                        .selected(name == "Open")
                        .show(ui);
                }
            },
        );
    });
    shown.into()
}

/// One toggle off and one on with an icon, as the gallery card shows them.
#[story(ToggleButton<'static>, covers = [], drive = Drive::Never)]
fn toggle_button_on(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Row::new().show(ui, |ui, _| {
        ToggleButton::new("Snap to grid", data.bind("snap", false)).id_salt("snap").show(ui);
        ToggleButton::new("Guides", data.bind("on", true)).icon(IconId::new("eye")).id_salt("guides").show(ui);
    });
    shown.into()
}

#[story(ToggleButton<'static>, covers = [Default])]
fn toggle_button(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &SIZES);
    let disabled = *data.value("disabled", false);
    let label = data.value("label", String::from("Snap to grid")).clone();
    ToggleButton::new(label, data.bind("on", false)).icon(IconId::new("eye")).size(size).enabled(!disabled).show(ui).into()
}

/// A stacked toggle, off and on.
#[story(ToggleButton<'static>, covers = [], drive = Drive::Never, title = "Stacked")]
fn toggle_button_stacked(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Row::new().show(ui, |ui, _| {
        ToggleButton::new("Gridlines", data.bind("grid", false)).icon(IconId::new("eye")).arrangement(Arrangement::Stacked).id_salt("grid").show(ui);
        ToggleButton::new("Show rulers", data.bind("rulers", true))
            .icon(IconId::new("eye"))
            .arrangement(Arrangement::Stacked)
            .id_salt("rulers")
            .show(ui);
    });
    shown.into()
}

/// Stacked buttons in a ribbon group: flat at rest, one height for a one-line label and a label that names its break
/// (`"Page\nNumber"`, its chevron on the second line), icons on one line, and toggles flat off and on the plate on.
#[story(ToggleButton<'static>, covers = [], drive = Drive::Never, title = "Stacked in a ribbon")]
fn toggle_button_stacked_in_ribbon(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = with_env(
        ui,
        |overrides| {
            overrides.values.set(super::ControlPlace::Ribbon);
        },
        |ui| {
            Row::new().show(ui, |ui, _| {
                Button::new("Pictures").icon(IconId::new("image")).arrangement(Arrangement::Stacked).id_salt("pictures").show(ui);
                Button::new("Page\nNumber")
                    .icon(IconId::new("copy"))
                    .trailing_icon(IconId::new("chevron_down"))
                    .arrangement(Arrangement::Stacked)
                    .id_salt("page number")
                    .show(ui);
                ToggleButton::new("Track\nChanges", data.bind("track", false))
                    .icon(IconId::new("eye"))
                    .arrangement(Arrangement::Stacked)
                    .id_salt("track")
                    .show(ui);
                ToggleButton::new("Select", data.bind("select", true))
                    .icon(IconId::new("eye"))
                    .arrangement(Arrangement::Stacked)
                    .id_salt("select")
                    .show(ui);
            })
        },
    );
    shown.into()
}

/// Rests the pointer on the toggle, clicks it and stays: its output reports the click while the pointer stays.
fn click_and_stay(fixture: &mut Fixture) {
    if let Some(toggle) = fixture.by_label("Snap to grid") {
        fixture.click(&toggle);
    }
    fixture.idle(2);
}

/// A toggle clicked with the pointer still over it: the output's `hover` is over and clicked until the pointer leaves.
#[story(ToggleButton<'static>, covers = [], drive = Drive::Custom(click_and_stay), title = "Clicked under the pointer")]
fn toggle_button_clicked_under_pointer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = ToggleButton::new("Snap to grid", data.bind("snap_hover", false)).icon(IconId::new("eye")).show(ui);
    // The hold is the component's own state, which a measuring run does not carry: the story words it in its readout,
    // not in its data, which the purity probe compares.
    let clicked = shown.hover.clicked;
    StoryOutput::from(shown).readout(if clicked { "Clicked, pointer still over" } else { "Not clicked" })
}

#[story(ToggleButton<'static>, covers = [Mixed], drive = Drive::Never)]
fn toggle_button_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ToggleButton::new("Snap to grid", data.bind_mixed("mixed", Mixed::Mixed)).show(ui).into()
}

#[story(ToggleButton<'static>, covers = [Invalid], drive = Drive::Never)]
fn toggle_button_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_on = |on: &bool| if *on { Validity::Refuse(Message::new("Snapping is not available here")) } else { Validity::Ok };
    ToggleButton::new("Snap to grid", data.bind("invalid", true).check(refuse_on)).show(ui).into()
}

fn presets() -> [Entry<u8>; 4] {
    [
        Entry::new(0, "Black and white").icon(IconId::new("eye")).checked(true),
        Entry::new(1, "Colour"),
        Entry::separator(),
        Entry::new(2, "Grayscale").enabled(false),
    ]
}

#[story(SplitButton<'static, u8>, covers = [Default], drive = Drive::Activate)]
fn split_button(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let entries = presets();
    SplitButton::new("Trace", &entries).icon(IconId::new("image")).arrow_label("Tracing presets").show(ui).into()
}

fn open_menu(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::ArrowDown, Modifiers::ALT);
}

#[story(SplitButton<'static, u8>, covers = [Open], drive = Drive::Custom(open_menu))]
fn split_button_open(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let entries = presets();
    SplitButton::new("Trace", &entries).icon(IconId::new("image")).arrow_label("Tracing presets").show(ui).into()
}

/// A stacked split button: the icon is the main part, the label and its chevron the arrow part under it.
#[story(SplitButton<'static, u8>, covers = [], drive = Drive::Never, title = "Stacked")]
fn split_button_stacked(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let entries = presets();
    SplitButton::new("Trace", &entries).icon(IconId::new("image")).arrow_label("Tracing presets").arrangement(Arrangement::Stacked).show(ui).into()
}

/// The rows a split button's body shows.
fn paste_rows(ui: &mut Ui, _: &mut BodyCx<'_>) {
    MenuItem::new("Keep source formatting").show(ui);
    MenuItem::new("Merge formatting").show(ui);
    MenuItem::new("Keep text only").show(ui);
}

/// A split button whose arrow opens a menu the caller draws, open.
#[story(SplitButton<'static, u8>, covers = [Open], drive = Drive::Custom(open_menu), title = "Body")]
fn split_button_body(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    SplitButton::<u8>::body("Paste", paste_rows).icon(IconId::new("copy")).show(ui).into()
}

/// A split button whose main part is the Paste command, stacked as a ribbon shows it, its menu the paste options.
#[story(SplitButton<'static, u8>, covers = [], drive = Drive::Never, title = "Command")]
fn split_button_command(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    SplitButton::<u8>::command(EditCommands::Paste, SplitMenu::Body(Box::new(paste_rows)))
        .icon(IconId::new("copy"))
        .arrow_label("Paste options")
        .arrangement(Arrangement::Stacked)
        .show(ui)
        .into()
}

#[story(Link, covers = [Default])]
fn link(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Link::new("Show all your shortcuts").url("https://example.com/shortcuts").show(ui).into()
}
