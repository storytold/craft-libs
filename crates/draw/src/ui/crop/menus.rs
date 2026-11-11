//! The Crop options bar's two menus: the overlay menu ([`CropOverlayMenu`]) and the gear popover
//! ([`CropOptionsMenu`]).

use super::bar_words::CropBarWords;
use super::options::{CropFill, CropSettings};
use craft_behavior::geometry::crop::{CropGuide, GuideShow};
use craft_behavior::{IconId, Mixed};
use craft_keymap::{Chord, Key, Mods, Platform};
use craft_ui::buttons::IconButton;
use craft_ui::canvas::{CropShield, ShieldColor};
use craft_ui::choice::{Checkbox, Radio, RadioGroup, Swatch};
use craft_ui::contract::{
    Bind, Entry, FieldLayout, NoValue, Outcome, PopupKind, Props as _, Remember as _, Show as _, ShowContainer as _, SideLabel, Trigger, clicked,
};
use craft_ui::display::Divider;
use craft_ui::form::{Field, SliderField};
use craft_ui::layout_components::Row;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::pickers::Select;
use craft_ui::popover::Popover;
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::runtime::requests::ColorRequest;
use egui::{Response, Ui};

/// The overlay's name in `words`.
fn guide_name(words: &CropBarWords, guide: CropGuide) -> &str {
    match guide {
        CropGuide::Thirds => words.thirds.as_str(),
        CropGuide::Grid => words.grid.as_str(),
        CropGuide::Diagonal => words.diagonal.as_str(),
        CropGuide::Triangle => words.triangle.as_str(),
        CropGuide::GoldenRatio => words.golden_ratio.as_str(),
        CropGuide::GoldenSpiral => words.golden_spiral.as_str(),
        _ => words.thirds.as_str(),
    }
}

/// The show rule's name in `words`.
fn show_name(words: &CropBarWords, show: GuideShow) -> &str {
    match show {
        GuideShow::Auto => words.show_auto.as_str(),
        GuideShow::Always => words.show_always.as_str(),
        GuideShow::Never => words.show_never.as_str(),
        _ => words.show_auto.as_str(),
    }
}

/// The chord written for the platform, as a row's shortcut text.
fn chord_text(mods: Mods, key: Key) -> String {
    Chord::new(mods, key).display(Platform::current())
}

/// What a menu of the Crop bar reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct CropOverlayMenuOutput {
    /// The opener button's response.
    pub response: Response,
    /// A row changed the settings: the app persists them.
    pub changed: bool,
}

impl Outcome for CropOverlayMenuOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The overlay menu: an icon button named "Set the overlay options for the Crop tool" that opens a [`Menu`]: the six
/// overlays as radio rows, a rule, Auto, Always and Never Show Overlay as radio rows, a rule, Cycle Overlay (shortcut
/// text O) and Cycle Orientation (Shift+O; enabled for an overlay with more than one orientation: Triangle, Golden
/// Spiral). A row edits the settings and closes the menu.
#[must_use = "show the menu with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct CropOverlayMenu<'a> {
    settings: &'a mut CropSettings,
    /// The menu starts open.
    #[prop(flag)]
    opened: bool,
}

impl<'a> CropOverlayMenu<'a> {
    /// A menu that edits `settings` in place.
    pub fn new(settings: &'a mut CropSettings) -> Self {
        Self { settings, opened: false }
    }

    /// Shows the button, and the menu while it is open.
    pub fn show(self, ui: &mut Ui) -> CropOverlayMenuOutput {
        let Self { settings, opened } = self;
        let words = Env::of(ui).words::<CropBarWords>();
        let before = settings.clone();
        let shown = Row::new().show(ui, |ui, cx| {
            let appeared = cx.appeared();
            let open = &mut cx.memory::<Open>().0;
            if appeared && opened {
                *open = true;
            }
            let opener = IconButton::new(IconId::new("grid")).label(words.overlay.clone()).popup(PopupKind::Menu).show(ui).response;
            if clicked(&opener) {
                *open = !*open;
            }
            Menu::new(words.overlay.clone()).open(Bind::bound(open)).opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
                for guide in CropGuide::ALL {
                    let row = MenuItem::new(guide_name(&words, guide).to_owned()).radio_mark(settings.guide == guide).show(ui);
                    if row.ran {
                        settings.set_guide(guide);
                    }
                }
                Divider::horizontal().show(ui);
                for show in GuideShow::ALL {
                    let row = MenuItem::new(show_name(&words, show).to_owned()).radio_mark(settings.guide_show == show).show(ui);
                    if row.ran {
                        settings.guide_show = show;
                    }
                }
                Divider::horizontal().show(ui);
                let cycle = MenuItem::new(words.cycle_overlay.clone()).shortcut(chord_text(Mods::NONE, Key::O)).show(ui);
                if cycle.ran {
                    settings.cycle_guide();
                }
                let orientation = MenuItem::new(words.cycle_orientation.clone())
                    .shortcut(chord_text(Mods::SHIFT, Key::O))
                    .enabled(settings.guide.orientations() > 1)
                    .show(ui);
                if orientation.ran {
                    settings.cycle_orientation();
                }
            });
            opener
        });
        let response = shown.inner.unwrap_or_else(|| shown.output.clone());
        CropOverlayMenuOutput { response, changed: *settings != before }
    }
}

/// What the gear popover reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct CropOptionsMenuOutput {
    /// The gear button's response.
    pub response: Response,
    /// Custom… or the shield swatch asked for a colour: the app opens its colour picker with this request and sets
    /// `CropShield::custom_color` from the answer for [`CropOptionsMenu::COLOR_KEY`].
    pub pick_color: Option<ColorRequest>,
    /// An option changed: the app persists the settings.
    pub changed: bool,
}

impl Outcome for CropOptionsMenuOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Whether the menu or popover is open, kept per viewer.
#[derive(Clone, Copy, Default)]
struct Open(bool);

/// The gear popover: an icon button named "Set additional Crop options" that opens a [`Popover`] of form rows: Use
/// Classic Mode (shortcut text P), Show Cropped Area (H; shown on and disabled in Classic Mode), Auto Center Preview
/// (disabled in Classic Mode), Enable Crop Shield, the colour (Match Canvas, or Custom… with its swatch), Opacity (0 to
/// 100 %) and Auto Adjust Opacity, then Delete Cropped Pixels and the Fill select. The colour rows, Opacity and Auto
/// Adjust Opacity are disabled with the shield off. The popover stays open on clicks inside and closes on a click
/// outside, Escape or the button.
#[must_use = "show the menu with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct CropOptionsMenu<'a> {
    settings: &'a mut CropSettings,
    /// The popover starts open.
    #[prop(flag)]
    opened: bool,
}

impl<'a> CropOptionsMenu<'a> {
    /// The [`ColorRequest::key`] of the shield colour.
    pub const COLOR_KEY: &'static str = "crop-shield-color";

    /// A menu that edits `settings` in place.
    pub fn new(settings: &'a mut CropSettings) -> Self {
        Self { settings, opened: false }
    }

    /// Shows the button, and the popover while it is open.
    pub fn show(self, ui: &mut Ui) -> CropOptionsMenuOutput {
        let Self { settings, opened } = self;
        let env = Env::of(ui);
        let words = env.words::<CropBarWords>();
        let before = settings.clone();
        let mut pick = false;
        let shown = Row::new().show(ui, |ui, cx| {
            let appeared = cx.appeared();
            let open = &mut cx.memory::<Open>().0;
            if appeared && opened {
                *open = true;
            }
            let button =
                IconButton::new(IconId::new("settings")).label(words.options.clone()).popup(PopupKind::Dialog).expanded(*open).show(ui).response;
            if clicked(&button) {
                *open = !*open;
            }
            Popover::new(words.options.clone(), Bind::bound(open)).opened_by(&button).show(ui, |ui, cx| {
                let shield = &mut settings.shield;
                let detail = |key: Key| SideLabel::new().layout(FieldLayout::After).detail(chord_text(Mods::NONE, key));
                cx.labelled(ui, "classic", words.classic_mode.clone(), detail(Key::P), Checkbox::new(Bind::bound(&mut shield.classic_mode)));
                // Classic Mode greys the other two out; Show Cropped Area then shows as on, which is how it acts.
                let classic = shield.classic_mode;
                let mut forced = true;
                let area = if classic { Bind::bound(&mut forced) } else { Bind::bound(&mut shield.show_cropped_area) };
                cx.labelled(ui, "area", words.show_cropped_area.clone(), detail(Key::H), Checkbox::new(area).enabled(!classic));
                cx.labelled(
                    ui,
                    "center",
                    words.auto_center.clone(),
                    SideLabel::new().layout(FieldLayout::After),
                    Checkbox::new(Bind::bound(&mut shield.auto_center_preview)).enabled(!classic),
                );
                Divider::horizontal().show(ui);
                cx.labelled(
                    ui,
                    "shield",
                    words.shield.clone(),
                    SideLabel::new().layout(FieldLayout::After),
                    Checkbox::new(Bind::bound(&mut shield.enabled)),
                );
                let on = shield.enabled;
                let mut color = shield.color;
                Field::new(words.color_name.clone()).enabled(on).show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        let group = RadioGroup::new(words.color_name.clone(), Bind::bound(&mut color)).show(ui, |ui, cx| {
                            let after = SideLabel::new().layout(FieldLayout::After);
                            cx.labelled(ui, 0u8, words.match_canvas.clone(), after.clone(), Radio::new(ShieldColor::MatchCanvas));
                            let custom = cx.labelled(ui, 1u8, words.custom.clone(), after, Radio::new(ShieldColor::Custom));
                            clicked(&custom)
                        });
                        pick |= group.inner.unwrap_or(false);
                        let [r, g, b] = shield.custom_color;
                        if shield.color == ShieldColor::Custom || color == ShieldColor::Custom {
                            let swatch = Swatch::new(
                                words.shield_color.clone(),
                                Mixed::Same(craft_theme::Srgba::from_rgb_array([r, g, b].map(|c| f32::from(c) / 255.0))),
                            )
                            .picker(Self::COLOR_KEY)
                            .enabled(on)
                            .show(ui);
                            pick |= swatch.pick_color.is_some();
                        }
                    });
                });
                shield.color = color;
                if !shield.opacity.is_finite() {
                    shield.opacity = CropShield::default().opacity;
                }
                let mut opacity = f64::from(shield.opacity);
                SliderField::new(words.opacity_name.clone(), Bind::bound(&mut opacity), 0.0..=100.0)
                    .unit_kind(UnitKind::Percent)
                    .enabled(on)
                    .show(ui);
                shield.opacity = opacity as f32;
                cx.labelled(
                    ui,
                    "adjust",
                    words.auto_adjust.clone(),
                    SideLabel::new().layout(FieldLayout::After),
                    Checkbox::new(Bind::bound(&mut shield.auto_adjust)).enabled(on),
                );
                Divider::horizontal().show(ui);
                cx.labelled(
                    ui,
                    "delete",
                    words.delete_pixels.clone(),
                    SideLabel::new().layout(FieldLayout::After),
                    Checkbox::new(Bind::bound(&mut settings.delete_pixels)),
                );
                let fills = [
                    Entry::new(CropFill::Background, words.fill_background.clone()),
                    Entry::new(CropFill::ContentAware, words.fill_content_aware.clone()),
                ];
                Field::new(words.fill_name.clone()).show(ui, |ui, _| {
                    let pick = Select::new(words.fill_name.clone(), Bind::given(settings.fill), &fills).show(ui);
                    if let Some(fill) = pick.value().copied() {
                        settings.fill = fill;
                    }
                });
            });
            button
        });
        let pick_color = pick.then(|| {
            let [r, g, b] = settings.shield.custom_color;
            ColorRequest::new(Self::COLOR_KEY, craft_theme::Srgba::from_rgb_array([r, g, b].map(|c| f32::from(c) / 255.0)))
                .title(words.shield_color.clone())
        });
        let response = shown.inner.unwrap_or_else(|| shown.output.clone());
        CropOptionsMenuOutput { response, pick_color, changed: *settings != before }
    }
}
