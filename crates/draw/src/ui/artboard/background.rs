//! The background cells the bar and the Properties section share: White, Black, Transparent and
//! Other… with its colour request.

use super::{ArtboardBackground, ArtboardWords};
use craft_theme::Srgba;
use craft_ui::choice::Swatch;
use craft_ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::requests::ColorRequest;
use egui::Ui;

/// The key of the [`ColorRequest`] Other… files; the bar and the section share one handler in the app.
const BACKGROUND_KEY: &str = "artboard.background";

/// The background select's rows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Choice {
    White,
    Black,
    Transparent,
    Other,
}

/// What the background cells report this pass.
#[derive(Debug, Default)]
pub(super) struct BackgroundPick {
    /// The background picked.
    pub picked: Option<ArtboardBackground>,
    /// The request for the app's colour picker (Other…).
    pub pick_color: Option<ColorRequest>,
}

/// Draws the cells for `background` and reports a pick.
pub(super) fn background_cells(ui: &mut Ui, words: &ArtboardWords, background: ArtboardBackground) -> BackgroundPick {
    let custom = match background {
        ArtboardBackground::Custom(color) => Some(color),
        _ => None,
    };
    let request =
        |color: Option<Srgba>| ColorRequest::new(BACKGROUND_KEY, color.unwrap_or(Srgba::rgb(1.0, 1.0, 1.0))).title(words.background_color.clone());
    let entries = [
        Entry::new(Choice::White, words.white.clone()),
        Entry::new(Choice::Black, words.black.clone()),
        Entry::new(Choice::Transparent, words.transparent.clone()),
        Entry::new(Choice::Other, words.other.clone()),
    ];
    let now = match background {
        ArtboardBackground::White => Choice::White,
        ArtboardBackground::Black => Choice::Black,
        ArtboardBackground::Transparent => Choice::Transparent,
        _ => Choice::Other,
    };
    let mut pick = BackgroundPick::default();
    Row::new().show(ui, |ui, _| {
        match Select::new(words.background.clone(), Bind::given(now), &entries).show(ui).value().copied() {
            Some(Choice::White) => pick.picked = Some(ArtboardBackground::White),
            Some(Choice::Black) => pick.picked = Some(ArtboardBackground::Black),
            Some(Choice::Transparent) => pick.picked = Some(ArtboardBackground::Transparent),
            Some(Choice::Other) => pick.pick_color = Some(request(custom)),
            None => {}
        }
        if let Some(color) = custom
            && let Some(asked) = Swatch::new(words.background_color.clone(), color).picker(BACKGROUND_KEY).show(ui).pick_color
        {
            pick.pick_color = Some(asked);
        }
    });
    pick
}
