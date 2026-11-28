//! [`CropPresetList`]: the crop preset list as one cell for the crop bar and the task bar.

use super::bar_words::CropBarWords;
use super::options::{CROP_RATIOS, CROP_SIZES, CropPresetChoice, CropRatio, CropSizeFields};
use super::{CropDocument, CropRatioChange, CropSettings, CropUserPreset};
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Show as _};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Units};
use egui::{Response, Ui};

/// The preset list: Free, Size, the ratios, the sizes, the user presets, Front Image, New, Delete,
/// and Selection when [`selection_entry`](Self::selection_entry) is set. Picks go through
/// `CropSettings::choose`.
#[must_use = "show the list with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct CropPresetList<'a> {
    settings: &'a mut CropSettings,
    document: &'a CropDocument,
    /// The user's saved crop presets, oldest first.
    #[prop]
    user_presets: &'a [CropUserPreset],
    /// Offers Selection as the first entry.
    #[prop]
    selection_entry: bool,
    /// Shows Selection as the chosen entry (`CropTool::shows_selection_entry`); it takes effect with the entry offered.
    #[prop]
    selection_chosen: bool,
}

/// What [`CropPresetList::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies what the list reports"]
pub struct CropPresetListOutput {
    /// The list.
    pub response: Response,
    /// The ratio changed.
    pub ratio: Option<CropRatioChange>,
    /// New Crop Preset… was picked.
    pub new_preset: bool,
    /// Delete Crop Preset… was picked.
    pub delete_preset: bool,
    /// Selection was picked: the app calls `CropTool::take_selection_frame`.
    pub selection: bool,
}

impl Outcome for CropPresetListOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The name of size preset `index` of [`CROP_SIZES`]: its values as New Crop Preset… would name them.
fn size_name(words: &CropBarWords, index: usize, units: &Units) -> String {
    let Some(preset) = CROP_SIZES.get(index) else { return String::new() };
    let size = CropSizeFields {
        width: Some(preset.width),
        height: Some(preset.height),
        resolution: preset.resolution.to_owned(),
        ..CropSizeFields::default()
    };
    let settings = CropSettings { ratio: CropRatio::Size, size, ..CropSettings::default() };
    settings.preset_name(words, units).unwrap_or_default()
}

impl<'a> CropPresetList<'a> {
    /// The list editing `settings` over `document`.
    pub fn new(settings: &'a mut CropSettings, document: &'a CropDocument) -> Self {
        CropPresetList { settings, document, user_presets: &[], selection_entry: false, selection_chosen: false }
    }

    /// Shows the list.
    pub fn show(self, ui: &mut Ui) -> CropPresetListOutput {
        let CropPresetList { settings, document, user_presets, selection_entry, selection_chosen } = self;
        let env = Env::of(ui);
        let words = env.words::<CropBarWords>();
        let units = env.units();
        let has_document = document.size.width > 0.0 && document.size.height > 0.0;
        let mut entries = Vec::new();
        if selection_entry {
            entries.push(Entry::new(CropPresetChoice::Selection, words.selection.clone()));
        }
        entries.push(Entry::new(CropPresetChoice::Free, words.ratio.clone()));
        entries.push(Entry::new(CropPresetChoice::Size, words.size_mode.clone()));
        for (i, ratio) in CROP_RATIOS.iter().enumerate() {
            let entry = Entry::new(CropPresetChoice::Ratio(i), words.ratio_name(i).to_owned());
            entries.push(if *ratio == CropRatio::Original { entry.enabled(has_document) } else { entry });
        }
        for i in 0..CROP_SIZES.len() {
            entries.push(Entry::new(CropPresetChoice::SizePreset(i), size_name(&words, i, &units)));
        }
        for (i, preset) in user_presets.iter().enumerate() {
            entries.push(Entry::new(CropPresetChoice::User(i), preset.name.clone()));
        }
        entries.push(Entry::new(CropPresetChoice::FrontImage, words.front_image.clone()).enabled(has_document));
        entries.push(Entry::separator());
        entries.push(Entry::new(CropPresetChoice::NewPreset, words.new_preset.clone()).enabled(settings.preset_name(&words, &units).is_some()));
        entries.push(Entry::new(CropPresetChoice::DeletePreset, words.delete_preset.clone()).enabled(!user_presets.is_empty()));
        let current = if selection_chosen { CropPresetChoice::Selection } else { settings.choice(user_presets) };
        let pick = Select::new(words.presets.clone(), Bind::given(current), &entries).show(ui);
        let response = pick.response().clone();
        let mut out = CropPresetListOutput { response, ratio: None, new_preset: false, delete_preset: false, selection: false };
        let Some(choice) = pick.value().copied() else { return out };
        match choice {
            CropPresetChoice::NewPreset => out.new_preset = true,
            CropPresetChoice::DeletePreset => out.delete_preset = true,
            choice => {
                out.selection = choice == CropPresetChoice::Selection;
                if settings.choose(choice, document, user_presets) {
                    out.ratio = Some(CropRatioChange { editing: false });
                }
            }
        }
        out
    }
}
