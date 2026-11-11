//! `AppearancePage`: the Appearance group of Preferences over the suite's shared selection.

use super::chip::ThemeChip;
use crate::{SelectionChange, SuiteClient};
use craft_theme::{ContrastChoice, ModeChoice, MotionChoice, Size, TextSize, ThemeDef, ThemeId, TypeRole, Variants, variant::Tone};
use craft_ui::choice::RadioGroup;
use craft_ui::contract::{Bind, Entry, Held, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::menu::Menu;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};

craft_ui::words! {
    /// The words of the appearance page.
    pub struct AppearanceWords {
        appearance: "appearancePage.appearance" = "Appearance";
        mode: "appearancePage.mode" = "Appearance Mode";
        mode_system: "appearancePage.modeSystem" = "Sync with system";
        mode_dark: "appearancePage.modeDark" = "Dark";
        mode_light: "appearancePage.modeLight" = "Light";
        light_theme: "appearancePage.lightTheme" = "Light Theme";
        dark_theme: "appearancePage.darkTheme" = "Dark Theme";
        active: "appearancePage.active" = "Active";
        favorite: "appearancePage.favorite" = "Favorite";
        add_favorite: "appearancePage.addFavorite" = "Add to Favorites";
        remove_favorite: "appearancePage.removeFavorite" = "Remove from Favorites";
        size: "appearancePage.size" = "Size";
        compact: "appearancePage.compact" = "Compact";
        comfortable: "appearancePage.comfortable" = "Comfortable";
        text_size: "appearancePage.textSize" = "UI Font Size";
        text_tiny: "appearancePage.textTiny" = "Tiny";
        text_small: "appearancePage.textSmall" = "Small";
        text_medium: "appearancePage.textMedium" = "Medium";
        text_large: "appearancePage.textLarge" = "Large";
        contrast: "appearancePage.contrast" = "Contrast";
        contrast_system: "appearancePage.contrastSystem" = "Sync with system";
        contrast_standard: "appearancePage.contrastStandard" = "Standard";
        contrast_high: "appearancePage.contrastHigh" = "High";
        motion: "appearancePage.motion" = "Motion";
        motion_system: "appearancePage.motionSystem" = "Sync with system";
        motion_full: "appearancePage.motionFull" = "Full";
        motion_reduced: "appearancePage.motionReduced" = "Reduced";
    }
}

/// What the page reported this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct AppearancePageOutput {
    /// The page's response.
    pub response: Response,
    /// The draft changed this pass (the window's Apply becomes enabled).
    pub changed: bool,
    /// The favourite star of this theme was toggled; the app updates its favourites list.
    pub favorite_toggled: Option<ThemeId>,
}

impl Outcome for AppearancePageOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Appearance group of Preferences > Interface over the suite's shared selection, after PhotoCraft's page.
///
/// Rows, top to bottom, with the labels at the start: Appearance Mode (Sync with system, Dark, Light), Light Theme and
/// Dark Theme (a row of [`ThemeChip`]s each: the built-in themes, then the library's, each in the rows for the modes it
/// has; "Active" after the chips of the row for the mode the app shows now), Size (Compact, Comfortable), UI Font Size
/// (the [`TextSize`] steps), Contrast and Motion (Sync with system, Standard or Full, High or Reduced).
///
/// The page edits the bound draft, a [`SelectionChange`] the Preferences model keeps; the chips and lists show
/// [`SuiteClient::shown`] with the draft applied. Nothing reaches the suite until the window applies: the model's
/// `apply` calls `client.change(draft)`, its `cancel` clears the draft. Each chip row is one radio group: one Tab
/// stop, arrows move, Space or a click picks; a chip's context menu (the Menu key and Shift+F10 open it) has Add to
/// Favorites / Remove from Favorites, the keyboard path to the star.
#[must_use = "show the page with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct AppearancePage<'a> {
    client: &'a SuiteClient,
    draft: Bind<'a, SelectionChange>,
    /// The themes marked with a filled star.
    #[prop]
    favorites: &'a [ThemeId],
    /// Shows the UI Font Size row (default on); an app with its own text-size control turns it off.
    #[prop]
    text_size: bool,
    /// The system shows its dark mode: which row is active when the mode follows the system.
    #[prop]
    system_dark: bool,
}

impl<'a> AppearancePage<'a> {
    /// The page over the suite `client`, editing `draft`.
    pub fn new(client: &'a SuiteClient, draft: Bind<'a, SelectionChange>) -> Self {
        AppearancePage { client, draft, favorites: &[], text_size: true, system_dark: false }
    }

    /// Shows the page.
    pub fn show(self, ui: &mut Ui) -> AppearancePageOutput {
        let AppearancePage { client, draft, favorites, text_size, system_dark } = self;
        let words = Env::of(ui).words::<AppearanceWords>();
        let themes = themes(client);
        let mut held = Held::new(draft);
        let mut toggled = None;
        let mut changed = false;
        let mut shown = FieldGroup::new(words.appearance.clone()).show(ui, |ui, cx| {
            let before = held.read(cx).unwrap_or_default();
            let mut now = before.clone();
            let shown = now.applied_to(&client.shown());
            let mode = shown.mode.unwrap_or(ModeChoice::System);
            let dark_active = match mode {
                ModeChoice::Light => false,
                ModeChoice::Dark => true,
                _ => system_dark,
            };
            let modes = [
                Entry::new(ModeChoice::System, words.mode_system.clone()),
                Entry::new(ModeChoice::Dark, words.mode_dark.clone()),
                Entry::new(ModeChoice::Light, words.mode_light.clone()),
            ];
            if let Some(mode) = pick(ui, &words.mode, "mode", mode, &modes) {
                now.mode = Some(mode);
            }
            for dark in [false, true] {
                let (title, chosen) = if dark { (&words.dark_theme, &shown.theme_dark) } else { (&words.light_theme, &shown.theme_light) };
                let row = ChipRow { title, dark, active: dark == dark_active, chosen: chosen.as_ref(), themes: &themes, favorites, words: &words };
                let (picked, flipped) = row.show(ui);
                if let Some(id) = picked {
                    *(if dark { &mut now.theme_dark } else { &mut now.theme_light }) = Some(id);
                }
                toggled = toggled.take().or(flipped);
            }
            let sizes = [Entry::new(Size::Compact, words.compact.clone()), Entry::new(Size::Comfortable, words.comfortable.clone())];
            if let Some(size) = pick(ui, &words.size, "size", shown.size.unwrap_or_default(), &sizes) {
                now.size = Some(size);
            }
            if text_size {
                let steps = [
                    Entry::new(TextSize::Tiny, words.text_tiny.clone()),
                    Entry::new(TextSize::Small, words.text_small.clone()),
                    Entry::new(TextSize::Medium, words.text_medium.clone()),
                    Entry::new(TextSize::Large, words.text_large.clone()),
                ];
                if let Some(step) = pick(ui, &words.text_size, "text_size", shown.text_size.unwrap_or_default(), &steps) {
                    now.text_size = Some(step);
                }
            }
            let contrasts = [
                Entry::new(ContrastChoice::System, words.contrast_system.clone()),
                Entry::new(ContrastChoice::Standard, words.contrast_standard.clone()),
                Entry::new(ContrastChoice::High, words.contrast_high.clone()),
            ];
            if let Some(contrast) = pick(ui, &words.contrast, "contrast", shown.contrast.unwrap_or_default(), &contrasts) {
                now.contrast = Some(contrast);
            }
            let motions = [
                Entry::new(MotionChoice::System, words.motion_system.clone()),
                Entry::new(MotionChoice::Full, words.motion_full.clone()),
                Entry::new(MotionChoice::Reduced, words.motion_reduced.clone()),
            ];
            if let Some(motion) = pick(ui, &words.motion, "motion", shown.motion.unwrap_or_default(), &motions) {
                now.motion = Some(motion);
            }
            (now != before).then_some((before, now))
        });
        // Once per pass, after the real run: the edit the body decided is written.
        shown.settle(|cx, edit| {
            if let Some((before, now)) = edit.take() {
                changed = true;
                held.write(cx, &before, now, words.appearance.as_str());
            }
        });
        AppearancePageOutput { response: shown.output, changed, favorite_toggled: toggled }
    }
}

/// The built-in themes, then the library's.
fn themes(client: &SuiteClient) -> Vec<ThemeDef> {
    let library = client.snapshot().index.iter().map(|entry| &entry.theme);
    craft_theme::builtins().iter().chain(library).cloned().collect()
}

/// A row of `label` and a select over `entries`; the option picked this pass.
fn pick<T: craft_behavior::Value + 'static>(ui: &mut Ui, label: &Label, key: &'static str, current: T, entries: &[Entry<T>]) -> Option<T> {
    let mut picked = None;
    Field::new(label.clone()).id_salt(key).show(ui, |ui, _| {
        picked = Select::new(label.clone(), Bind::given(current.clone()), entries).id_salt(key).show(ui).value().cloned();
    });
    picked
}

/// One Light Theme or Dark Theme row.
struct ChipRow<'a> {
    title: &'a Label,
    dark: bool,
    /// The row's mode is the one the app shows now.
    active: bool,
    chosen: Option<&'a ThemeId>,
    themes: &'a [ThemeDef],
    favorites: &'a [ThemeId],
    words: &'a AppearanceWords,
}

impl ChipRow<'_> {
    /// The theme picked and the theme whose favourite star was toggled this pass.
    fn show(self, ui: &mut Ui) -> (Option<ThemeId>, Option<ThemeId>) {
        let mut flipped = None;
        let mut picked = None;
        let fits = |theme: &&ThemeDef| {
            matches!((theme.variants, self.dark), (Variants::Both, _) | (Variants::DarkOnly, true) | (Variants::LightOnly, false))
        };
        Field::new(self.title.clone()).id_salt(("chips", self.dark)).show(ui, |ui, _| {
            let group = RadioGroup::new(self.title.clone(), Bind::given(self.chosen.cloned())).id_salt("group").show(ui, |ui, _| {
                Row::new().wrap(true).show(ui, |ui, _| {
                    for theme in self.themes.iter().filter(|theme| fits(theme)) {
                        let favorite = self.favorites.contains(&theme.id);
                        let chip = ThemeChip::new(theme, self.dark).favorite(favorite).id_salt(theme.id.as_str()).show(ui);
                        if chip.favorite_clicked {
                            flipped = Some(theme.id.clone());
                        }
                        let action = if favorite { &self.words.remove_favorite } else { &self.words.add_favorite };
                        let entries = [Entry::new((), action.clone())];
                        if Menu::context(action.clone(), &chip.response).id_salt(("menu", theme.id.as_str())).pick(ui, &entries).picked.is_some() {
                            flipped = Some(theme.id.clone());
                        }
                    }
                    if self.active {
                        Text::new(self.words.active.clone()).role(TypeRole::Caption).tone(Tone::Accent).show(ui);
                    }
                });
            });
            if let Some(Some(id)) = group.output.value()
                && Some(id) != self.chosen
            {
                picked = Some(id.clone());
            }
        });
        (picked, flipped)
    }
}
