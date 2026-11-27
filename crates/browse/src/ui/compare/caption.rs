//! `CompareCaption`: label, name, stars, flag and colour dot.

use craft_behavior::{IconId, Proposal};
use craft_i18n::Format;
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use craft_ui::choice::Rating;
use craft_ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Icon, Shade, Text};
use craft_ui::layout_components::Row;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Env;
use craft_ui::style::{DataColor, TypeRole};
use craft_ui::tile_grid::TileFlag;
use egui::{Color32, Rect, Ui};

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

    /// The compact stars of the rating, if any: only the lit stars, in the panel's ink. Read-only: the caption shows
    /// the rating, the app sets it elsewhere.
    fn stars(&self) -> Option<Rating<'static>> {
        self.rating.filter(|n| *n > 0).map(|n| Rating::new(Bind::given(n.min(STARS)), STARS).compact(true).read_only(true))
    }

    /// The flag in its tone, then the colour label's dot: the marks after the stars.
    fn marks(&self) -> Vec<Icon<'static>> {
        let flag = self.flag.and_then(|flag| match flag {
            TileFlag::Picked => Some(Icon::new(IconSpec::svg(IconId::new("flag"))).tone(Tone::Good)),
            TileFlag::Rejected => Some(Icon::new(IconSpec::svg(IconId::new("ban"))).tone(Tone::Bad)),
            _ => None,
        });
        let dot = self.label_color.map(|color| Icon::new(IconSpec::disc().data_color(DataColor::new(Color32::from(color)))));
        flag.into_iter().chain(dot).collect()
    }

    fn label_text(&self) -> Text {
        Text::new(self.label.clone()).role(TypeRole::BodyStrong)
    }

    fn name_text(&self) -> Text {
        Text::new(self.name.clone()).role(TypeRole::Caption).shade(Shade::Muted)
    }

    /// The strip's height: its tallest part.
    pub(super) fn height(&self, ui: &Ui) -> f32 {
        let mut height = self.label_text().measure_in(ui, Proposal::Ideal).y;
        if let Some(mut stars) = self.stars() {
            height = height.max(stars.measure_in(ui, Proposal::Ideal).y);
        }
        for mut mark in self.marks() {
            height = height.max(mark.measure_in(ui, Proposal::Ideal).y);
        }
        height
    }

    /// Shows the strip at `rect`: the label, then the stars, the flag and the dot while each fits, then the name in
    /// the room they leave after the label.
    pub(super) fn strip(&self, ui: &mut Ui, rect: Rect) {
        let gap = Env::of(ui).tokens.space.gap.row;
        let mut used = self.label_text().measure_in(ui, Proposal::Ideal).x;
        // The stars, then each mark, while the next one fits.
        let mut widths = Vec::new();
        if let Some(mut stars) = self.stars() {
            widths.push(stars.measure_in(ui, Proposal::Ideal).x);
        }
        widths.extend(self.marks().into_iter().map(|mut mark| mark.measure_in(ui, Proposal::Ideal).x));
        let shown = widths.iter().take_while(|width| {
            let fits = used + gap + **width <= rect.width();
            if fits {
                used += gap + **width;
            }
            fits
        });
        let shown = shown.count();
        let name_fits = used + gap + self.name_text().measure_in(ui, Proposal::Ideal).x <= rect.width();
        let shown_stars = usize::from(self.stars().is_some()).min(shown);
        let shown_marks = shown - shown_stars;
        let env = Env::of(ui);
        let stars_name = self.rating.filter(|n| *n > 0).map(|n| {
            let n = u64::from(n.min(STARS));
            env.words::<CompareWords>().stars.text(n, &env.view().format().integer(n)).to_string()
        });
        Row::new().gap(Gap::Row).show_at(ui, rect, |ui, _| {
            self.label_text().show(ui);
            if name_fits {
                self.name_text().show(ui);
            }
            if let Some(stars) = self.stars().filter(|_| shown_stars > 0) {
                match &stars_name {
                    Some(name) => stars.label(name.clone()).show(ui),
                    None => stars.show(ui),
                };
            }
            for mark in self.marks().into_iter().take(shown_marks) {
                mark.show(ui);
            }
        });
    }
}
