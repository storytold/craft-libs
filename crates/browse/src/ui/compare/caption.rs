//! `CompareCaption`: label, name, stars, flag and colour dot.

use craft_behavior::{IconId, Proposal};
use craft_i18n::Format;
use craft_theme::Srgba;
use craft_ui::choice::Rating;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Icon, Shade, Text};
use craft_ui::layout_components::Row;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Env;
use craft_ui::style::TypeRole;
use craft_ui::tile_grid::TileFlag;
use egui::Ui;

use super::CompareWords;

/// Stars a rating has.
const STARS: u8 = 5;

/// The strip under a compare tile's picture: the label (strong), the file name (dropped when it does not fit beside
/// the rest), the stars (display only), the flag and the colour dot. It runs from the reading start, so from the
/// right in a right-to-left interface.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CompareCaption {
    /// The pane's label ("Select", "Candidate"), translated.
    pub label: String,
    /// The file name.
    pub name: String,
    /// Stars 0 to 5.
    pub rating: Option<u8>,
    /// Pick or reject.
    pub flag: Option<TileFlag>,
    /// The colour label.
    pub label_color: Option<Srgba>,
}

impl CompareCaption {
    /// A caption with a label and a name.
    pub fn new(label: impl Into<String>, name: impl Into<String>) -> Self {
        Self { label: label.into(), name: name.into(), rating: None, flag: None, label_color: None }
    }

    /// The stars.
    #[must_use]
    pub fn rating(self, rating: Option<u8>) -> Self {
        Self { rating, ..self }
    }

    /// The flag.
    #[must_use]
    pub fn flag(self, flag: Option<TileFlag>) -> Self {
        Self { flag, ..self }
    }

    /// The colour label.
    #[must_use]
    pub fn label_color(self, color: Option<Srgba>) -> Self {
        Self { label_color: color, ..self }
    }

    /// The tile's accessible name: the caption read as text.
    pub(super) fn accessible_name(&self, words: &CompareWords, format: &Format) -> String {
        let mut details = String::new();
        if let Some(n) = self.rating.filter(|n| *n > 0) {
            let n = u64::from(n.min(STARS));
            details.push_str(", ");
            details.push_str(&words.stars.text(n, &format.integer(n)));
        }
        match self.flag {
            Some(TileFlag::Picked) => details.extend([", ", words.picked.as_str()]),
            Some(TileFlag::Rejected) => details.extend([", ", words.rejected.as_str()]),
            Some(_) | None => {}
        }
        words.tile_name.fill(&[("label", &self.label), ("name", &self.name), ("details", &details)])
    }

    /// Shows the strip in `ui`, which is `room` points wide. The name stays out when it would not fit beside the
    /// label and the marks.
    pub(super) fn strip(&self, ui: &mut Ui, room: f32) {
        let mut label = Text::new(self.label.clone()).role(TypeRole::BodyStrong);
        let mut name = Text::new(self.name.clone()).role(TypeRole::Caption).shade(Shade::Muted);
        let stars = self.rating.filter(|n| *n > 0).map(|n| n.min(STARS));
        let mut marks = 0.0;
        if let Some(n) = stars {
            marks += Rating::new(Bind::given(n), STARS).enabled(false).measure_in(ui, Proposal::Ideal).x;
        }
        let flag = self.flag.and_then(|flag| match flag {
            TileFlag::Picked => Some(IconSpec::svg(IconId::new("flag"))),
            TileFlag::Rejected => Some(IconSpec::svg(IconId::new("ban"))),
            _ => None,
        });
        if let Some(flag) = &flag {
            marks += Icon::new(flag.clone()).measure_in(ui, Proposal::Ideal).x;
        }
        let dot = self.label_color.map(IconSpec::swatch);
        if let Some(dot) = &dot {
            marks += Icon::new(dot.clone()).measure_in(ui, Proposal::Ideal).x;
        }
        let name_fits = label.measure_in(ui, Proposal::Ideal).x + name.measure_in(ui, Proposal::Ideal).x + marks <= room;
        let env = Env::of(ui);
        let stars_name = stars.map(|n| env.words::<CompareWords>().stars.text(u64::from(n), &env.view().format().integer(u64::from(n))).to_string());
        Row::new().show(ui, |ui, _| {
            Text::new(self.label.clone()).role(TypeRole::BodyStrong).show(ui);
            if name_fits {
                Text::new(self.name.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            }
            if let Some(n) = stars {
                let rating = Rating::new(Bind::given(n), STARS).enabled(false);
                match &stars_name {
                    Some(name) => rating.label(name.clone()).show(ui),
                    None => rating.show(ui),
                };
            }
            if let Some(flag) = flag.clone() {
                Icon::new(flag).show(ui);
            }
            if let Some(dot) = dot.clone() {
                Icon::new(dot).show(ui);
            }
        });
    }
}
