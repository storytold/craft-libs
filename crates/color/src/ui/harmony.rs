//! Harmony controls: `HarmonyWheel`, the colours of a harmony as markers on a hue and saturation wheel, and
//! `HarmonyPanel`, Color Guide (rule, harmony strip, variation grid). The maths is [`crate::harmony`].

use craft_behavior::{EditEvent, Selection};
use craft_theme::{Hsb, Srgba};
use craft_ui::contract::{Bind, CommandSet, Entry, Held, NoValue, Outcome, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout::RowAlign;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::PanelRow;
use craft_ui::pickers::Select;
use craft_ui::runtime::commands::{CommandRef, CommandState};
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};
use std::hash::{DefaultHasher, Hash, Hasher};

use super::picker::{ChannelRow, ChannelRule};
use super::{ColorDisc, SwatchTable, SwatchTableOutput};
use crate::harmony::{Guide, GuideOptions, Harmony, Variation};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The words of the harmony controls and of the Color Guide panel menu.
    #[non_exhaustive]
    pub struct HarmonyWords {
        rule: "harmony.rule" = "Rule";
        brightness: "harmony.brightness" = "Brightness";
        set_base: "harmony.setBase" = "Set base color to the current color";
        harmony: "harmony.harmony" = "Harmony colors";
        variations: "harmony.variations" = "Variations";
        shades: "harmony.shades" = "Shades";
        tints: "harmony.tints" = "Tints";
        cool: "harmony.cool" = "Cool";
        warm: "harmony.warm" = "Warm";
        muted: "harmony.muted" = "Muted";
        vivid: "harmony.vivid" = "Vivid";
        complementary: "harmony.ruleComplementary" = "Complementary";
        complementary2: "harmony.ruleComplementary2" = "Complementary 2";
        split_complementary: "harmony.ruleSplitComplementary" = "Split Complementary";
        left_complement: "harmony.ruleLeftComplement" = "Left Complement";
        right_complement: "harmony.ruleRightComplement" = "Right Complement";
        analogous: "harmony.ruleAnalogous" = "Analogous";
        analogous2: "harmony.ruleAnalogous2" = "Analogous 2";
        monochromatic: "harmony.ruleMonochromatic" = "Monochromatic";
        shades_rule: "harmony.ruleShades" = "Shades";
        triad: "harmony.ruleTriad" = "Triad";
        triad2: "harmony.ruleTriad2" = "Triad 2";
        triad3: "harmony.ruleTriad3" = "Triad 3";
        tetrad: "harmony.ruleTetrad" = "Tetrad";
        tetrad2: "harmony.ruleTetrad2" = "Tetrad 2";
        tetrad3: "harmony.ruleTetrad3" = "Tetrad 3";
        compound: "harmony.ruleCompound" = "Compound";
        compound2: "harmony.ruleCompound2" = "Compound 2";
        high_contrast: "harmony.ruleHighContrast" = "High Contrast";
        high_contrast2: "harmony.ruleHighContrast2" = "High Contrast 2";
        high_contrast3: "harmony.ruleHighContrast3" = "High Contrast 3";
        pentagram: "harmony.rulePentagram" = "Pentagram";
    }
}

impl HarmonyWords {
    /// The name of `rule`.
    pub fn rule_name(&self, rule: Harmony) -> &Label {
        match rule {
            Harmony::Complementary => &self.complementary,
            Harmony::Complementary2 => &self.complementary2,
            Harmony::SplitComplementary => &self.split_complementary,
            Harmony::LeftComplement => &self.left_complement,
            Harmony::RightComplement => &self.right_complement,
            Harmony::Analogous => &self.analogous,
            Harmony::Analogous2 => &self.analogous2,
            Harmony::Monochromatic => &self.monochromatic,
            Harmony::Shades => &self.shades_rule,
            Harmony::Triad => &self.triad,
            Harmony::Triad2 => &self.triad2,
            Harmony::Triad3 => &self.triad3,
            Harmony::Tetrad => &self.tetrad,
            Harmony::Tetrad2 => &self.tetrad2,
            Harmony::Tetrad3 => &self.tetrad3,
            Harmony::Compound => &self.compound,
            Harmony::Compound2 => &self.compound2,
            Harmony::HighContrast => &self.high_contrast,
            Harmony::HighContrast2 => &self.high_contrast2,
            Harmony::HighContrast3 => &self.high_contrast3,
            Harmony::Pentagram => &self.pentagram,
        }
    }
}

/// What a [`HarmonyWheel`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the edits"]
pub struct HarmonyWheelOutput {
    /// The wheel's response.
    pub response: Response,
    /// The colour whose marker was pressed; it becomes the selected one.
    pub pressed: Option<usize>,
    /// The edits of the colours this pass, each with the colour's index: a marker dragged (new hue and saturation) or,
    /// for the selected colour, the Brightness slider (new brightness); the value is `[h degrees, s, b]` with `None`
    /// where the edit kept the value. The host applies each value with [`move_on_wheel`](crate::harmony::move_on_wheel),
    /// linked while a rule is chosen. A pass reports the marker's edit and the slider's, in that order, when both
    /// happened. An edit with no value (a cancel) is for the selected colour.
    pub edits: Vec<(usize, EditEvent<[Option<f32>; 3]>)>,
}

impl Outcome for HarmonyWheelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A hue and saturation wheel at the selected colour's brightness (hue around, saturation outwards), a marker per
/// colour joined to the centre by a line (the selected one larger), and a Brightness slider for the selected colour
/// under it.
///
/// - Press a marker to select it; drag it to move that colour. The keys are the disc's.
/// - AccessKit: a group named by the label; each marker named by its hex.
#[must_use = "show the wheel with `show`"]
#[derive(Pattern)]
pub struct HarmonyWheel<'a> {
    label: Label,
    colors: &'a [Srgba],
    selected: Option<usize>,
}

impl<'a> HarmonyWheel<'a> {
    /// A wheel named `label` over `colors`.
    pub fn new(label: impl Into<Label>, colors: &'a [Srgba]) -> Self {
        HarmonyWheel { label: label.into(), colors, selected: None }
    }

    /// Marks the colour at this index as selected; none by default.
    pub fn selected_marker(mut self, selected: Option<usize>) -> Self {
        self.selected = selected;
        self
    }

    /// Shows the wheel and the Brightness row.
    pub fn show(self, ui: &mut Ui) -> HarmonyWheelOutput {
        let HarmonyWheel { label, colors, selected } = self;
        let words = Env::of(ui).words::<HarmonyWords>();
        let selected = selected.filter(|&i| i < colors.len());
        let hsb = selected.map(|i| colors[i].to_hsb());
        let brightness = hsb.map_or(1.0, |c| c.b);
        let names: Vec<Label> = colors.iter().map(|c| Label::from(c.to_hex())).collect();
        let mut pressed = None;
        let mut edits = Vec::new();
        let shown = Stack::new().show(ui, |ui, _| {
            edits.clear();
            pressed = None;
            let mut disc = ColorDisc::new(label.clone(), colors, Bind::kept());
            if let Some(selected) = selected {
                disc = disc.chosen(selected);
            }
            let disc = disc.brightness(brightness).names(names.clone()).show(ui);
            pressed = disc.pressed;
            let grabbed = disc.grabbed;
            for event in disc.edit.events {
                let marker = event.value().map(|m| m.marker).or(grabbed).or(selected);
                if let Some(marker) = marker {
                    edits.push((marker, event.map(|m| [Some(m.hue), Some(m.saturation), None])));
                }
            }
            let track = hsb.map(|c| vec![Srgba::from_hsb(Hsb { b: 0.0, ..c }), Srgba::from_hsb(Hsb { b: 1.0, ..c })]).unwrap_or_default();
            let value = (f64::from(brightness) * 100.0).round();
            let rule = ChannelRule { range: 0.0..=100.0, step: 1.0, decimals: 0 };
            Stack::new().enabled(selected.is_some()).show(ui, |ui, _| {
                let row = ChannelRow::new(words.brightness.clone(), Bind::given(value), rule.clone()).track(track.clone()).show(ui);
                if let (Some(i), Some(row)) = (selected, row) {
                    for event in row.events {
                        edits.push((i, event.map(|v| [None, None, Some((v / 100.0) as f32)])));
                    }
                }
            });
        });
        HarmonyWheelOutput { response: shown.output, pressed, edits }
    }
}

/// The Color Guide panel's settings, kept across runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct HarmonyPanelState {
    /// The chosen harmony rule.
    pub rule: Harmony,
    /// The variation grid's options.
    pub options: GuideOptions,
}

/// A Color Guide panel menu entry the app carries out.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum HarmonyMenuAction {
    /// Save Colors as Swatches: the selected cells' colours, in grid order.
    SaveAsSwatches(Vec<Srgba>),
    /// Color Guide Options…: the app opens its options dialog (steps 1 to 20 and the Less to More amount of
    /// [`GuideOptions`]).
    Options,
}

/// The cells of the variation grid chosen for Save Colors as Swatches. The host owns it and hands it to the panel and
/// to the panel menu; the cells belong to the guide they were picked from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HarmonyPicked {
    selection: Selection<usize>,
    guide: u64,
    colors: Vec<Srgba>,
}

impl HarmonyPicked {
    /// The chosen colours, in grid order.
    pub fn colors(&self) -> &[Srgba] {
        &self.colors
    }
}

/// What a [`HarmonyPanel`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the chosen colour"]
pub struct HarmonyPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// A colour of the strip or the grid was clicked: the app applies it to the selection or the current colour.
    pub chosen: Option<Srgba>,
    /// The base swatch was clicked: the app sets the base to its current colour.
    pub set_base: bool,
}

impl Outcome for HarmonyPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What the guide is made from.
#[derive(Clone, PartialEq)]
struct GuideKey {
    base: Srgba,
    rule: Harmony,
    options: GuideOptions,
    limit: Option<Vec<Srgba>>,
}

impl GuideKey {
    fn stamp(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        format!("{:?}", (self.base, self.rule, self.options, &self.limit)).hash(&mut hasher);
        hasher.finish()
    }
}

/// The guide kept until what it is made from changes: snapping every cell to a library is slow to redo each pass.
#[derive(Clone, Default)]
struct GuideCache(Option<(GuideKey, Guide)>);

impl GuideCache {
    fn of(&mut self, key: &GuideKey) -> Guide {
        if let Some((kept, guide)) = &self.0
            && kept == key
        {
            return guide.clone();
        }
        let guide = Guide::new(key.base, key.rule, &key.options);
        let guide = match &key.limit {
            Some(palette) => guide.limited(palette),
            None => guide,
        };
        self.0 = Some((key.clone(), guide.clone()));
        guide
    }
}

/// Color Guide: the base swatch (a click sets the base to the current colour) and the rule `Select`; the harmony strip
/// (one cell per colour, tooltip the hex); the variation grid, one row per harmony colour from shades to tints (or
/// cool to warm, muted to vivid); the grid's end labels. A click on any cell chooses its colour; Shift or Cmd selects
/// several for Save Colors as Swatches.
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct HarmonyPanel<'a> {
    state: Bind<'a, HarmonyPanelState>,
    picked: Bind<'a, HarmonyPicked>,
    base: Srgba,
    limit: Option<&'a [Srgba]>,
}

impl<'a> HarmonyPanel<'a> {
    /// The guide for `base`.
    pub fn new(state: Bind<'a, HarmonyPanelState>, picked: Bind<'a, HarmonyPicked>, base: Srgba) -> Self {
        HarmonyPanel { state, picked, base, limit: None }
    }

    /// Limit to Library: every colour snaps to the nearest of `palette`.
    pub fn limit(mut self, palette: &'a [Srgba]) -> Self {
        self.limit = Some(palette);
        self
    }

    /// Shows the panel.
    pub fn show(self, ui: &mut Ui) -> HarmonyPanelOutput {
        let HarmonyPanel { state, picked, base, limit } = self;
        let words = Env::of(ui).words::<HarmonyWords>();
        let mut held_state = Held::new(state);
        let mut held_picked = Held::new(picked);
        let (mut chosen, mut set_base) = (None, false);
        let mut shown = Stack::new().show(ui, |ui, cx| {
            let before = held_state.read(cx)?;
            let before_picked = held_picked.read(cx).unwrap_or_default();
            let key = GuideKey { base, rule: before.rule, options: before.options, limit: limit.map(<[Srgba]>::to_vec) };
            let stamp = key.stamp();
            let guide = cx.memory::<GuideCache>().of(&key);
            let mut now = before;

            let rules: Vec<Entry<Harmony>> = Harmony::ALL.iter().map(|rule| Entry::new(*rule, words.rule_name(*rule).clone())).collect();
            Row::new().show(ui, |ui, _| {
                set_base |= craft_ui::choice::Swatch::new(words.set_base.clone(), base).tooltip(words.set_base.clone()).show(ui).clicked;
                if let Some(rule) = Select::new(words.rule.clone(), Bind::given(before.rule), &rules).show(ui).value().copied() {
                    now.rule = rule;
                }
            });

            let strip: Vec<(Label, Srgba)> = guide.colors.iter().map(|c| (Label::from(c.to_hex()), *c)).collect();
            let mut strip_selection = Selection::default();
            let out =
                SwatchTable::new(words.harmony.clone(), &strip).per_row(strip.len().max(1)).selection(Bind::bound(&mut strip_selection)).show_all(ui);
            if let Some(i) = out.selected {
                chosen = strip.get(i).map(|(_, c)| *c);
            }

            let columns = guide.grid.first().map_or(1, Vec::len);
            let flat: Vec<(Label, Srgba)> = guide.grid.iter().flatten().map(|c| (Label::from(c.to_hex()), *c)).collect();
            let mut selection = if before_picked.guide == stamp { before_picked.selection.clone() } else { Selection::default() };
            let SwatchTableOutput { selected, chosen: cells, .. } =
                SwatchTable::new(words.variations.clone(), &flat).multiple(true).per_row(columns).selection(Bind::bound(&mut selection)).show_all(ui);
            if let Some(i) = selected
                && cells.len() == 1
            {
                chosen = flat.get(i).map(|(_, c)| *c);
            }
            let mut cells = selection.items.clone();
            cells.sort_unstable();
            cells.dedup();
            let colors: Vec<Srgba> = cells.iter().filter_map(|&i| flat.get(i).map(|(_, c)| *c)).collect();
            let after_picked = HarmonyPicked { selection, guide: stamp, colors };

            let (left, right) = match before.options.variation {
                Variation::TintsShades => (&words.shades, &words.tints),
                Variation::WarmCool => (&words.cool, &words.warm),
                Variation::VividMuted => (&words.muted, &words.vivid),
            };
            Row::new().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
                Text::new(left.clone()).show(ui);
                Text::new(right.clone()).show(ui);
            });

            Some((before, now, before_picked, after_picked))
        });
        // Once per pass, after the real run: the edited rule and the picked cells are stored.
        shown.settle(|cx, edited| {
            let Some((before, now, before_picked, after_picked)) = edited.take() else { return };
            if after_picked != before_picked {
                held_picked.write(cx, &before_picked, after_picked, words.variations.as_str());
            }
            if now != before {
                held_state.write(cx, &before, now, words.rule.as_str());
            }
        });
        HarmonyPanelOutput { response: shown.output, chosen, set_base }
    }
}

/// The Color Guide panel menu's commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "colorGuide")]
#[non_exhaustive]
pub enum HarmonyCommand {
    /// Show the variation grid from shades to tints.
    #[command(label = "Show Tints/Shades", kind = "radio", group = "colorGuide.variation")]
    TintsShades,
    /// Show the variation grid from cool to warm.
    #[command(label = "Show Warm/Cool", kind = "radio", group = "colorGuide.variation")]
    WarmCool,
    /// Show the variation grid from muted to vivid.
    #[command(label = "Show Vivid/Muted", kind = "radio", group = "colorGuide.variation")]
    VividMuted,
    /// Save the chosen cells' colours as swatches.
    #[command(label = "Save Colors as Swatches")]
    SaveAsSwatches,
    /// Open the Color Guide options.
    #[command(label = "Color Guide Options…")]
    Options,
}

/// The Color Guide panel menu: the rows for the dock's panel menu, the states the host publishes, and the carrying out
/// of a fired command.
pub struct HarmonyMenu;

impl HarmonyMenu {
    /// The rows: Show Tints/Shades, Show Warm/Cool, Show Vivid/Muted, a separator, Save Colors as Swatches and Color
    /// Guide Options….
    pub fn rows() -> Vec<PanelRow> {
        let row = |command: HarmonyCommand| PanelRow::Command(CommandRef::from(command));
        vec![
            row(HarmonyCommand::TintsShades),
            row(HarmonyCommand::WarmCool),
            row(HarmonyCommand::VividMuted),
            PanelRow::Separator,
            row(HarmonyCommand::SaveAsSwatches),
            row(HarmonyCommand::Options),
        ]
    }

    /// The state of each command: the current variation checked; Save Colors as Swatches enabled while cells are
    /// chosen.
    pub fn states(state: &HarmonyPanelState, picked: &HarmonyPicked) -> Vec<(HarmonyCommand, CommandState)> {
        let variation = |command, mine| (command, CommandState::new().checked(state.options.variation == mine));
        vec![
            variation(HarmonyCommand::TintsShades, Variation::TintsShades),
            variation(HarmonyCommand::WarmCool, Variation::WarmCool),
            variation(HarmonyCommand::VividMuted, Variation::VividMuted),
            (HarmonyCommand::SaveAsSwatches, CommandState::new().enabled(!picked.colors().is_empty())),
            (HarmonyCommand::Options, CommandState::new()),
        ]
    }

    /// Carries out `command`: a variation is set in `state`; the others come back for the app.
    pub fn run_command(state: &mut HarmonyPanelState, picked: &HarmonyPicked, command: HarmonyCommand) -> Option<HarmonyMenuAction> {
        match command {
            HarmonyCommand::TintsShades => state.options.variation = Variation::TintsShades,
            HarmonyCommand::WarmCool => state.options.variation = Variation::WarmCool,
            HarmonyCommand::VividMuted => state.options.variation = Variation::VividMuted,
            HarmonyCommand::SaveAsSwatches => {
                return (!picked.colors().is_empty()).then(|| HarmonyMenuAction::SaveAsSwatches(picked.colors().to_vec()));
            }
            HarmonyCommand::Options => return Some(HarmonyMenuAction::Options),
        }
        None
    }
}
