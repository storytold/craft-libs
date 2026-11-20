//! `PageSetupDialog`: Word's Page Setup and InDesign's Document Setup as a form dialog over a [`PageSetup`], and
//! `PageLayoutMenus`: Layout > Margins, Orientation and Size as menus.
//!
//! ```ignore
//! let shown = PageSetupDialog::new(Bind::bound(&mut draft)).unit(prefs.unit).layout().opened(Bind::bound(&mut open)).show(ui);
//! if let Some(setup) = shown.accepted { doc.apply_page_setup(setup); } // one undo step
//! // the Layout tab:
//! if let Some(choice) = PageLayoutMenus::new(&doc.page_setup()).unit(prefs.unit).margins(ui, &margins_button) { doc.apply(choice) }
//! ```

#[cfg(feature = "suite")]
pub(crate) mod stories;

use std::ops::RangeInclusive;

use craft_behavior::{ColumnWidth, EditEvent, NumberCodec};
use craft_theme::variant::Tone;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, EditResponse, Entry, FieldLayout, Pattern, Props, Remember as _, Show as _, ShowContainer as _, SideLabel, Trigger};
use craft_ui::display::{Divider, Text};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::{Grid, Row};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::patterns::form_dialogs::{FormCx, FormDialog, FormOutput};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, UnitKind, Units};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{GroupBox, HeaderStrength, TitleRow};
use craft_ui::text::NumberField;
use egui::{Response, Ui};

use crate::setup::{
    DocumentGrid, GridKind, MARGIN_PRESETS, MAX_GRID_COUNT, MAX_PAGE_SIDE, MIN_PAGE_SIDE, MarginPreset, Orientation, PAPER_SIZES, PageSetup,
    PageVerticalAlign, PaperSize, paper_named,
};

/// The margin and paper fields' scrub step, page points (WordCraft's 0.05 in).
pub const MARGIN_SCRUB_STEP: f64 = 3.6;

/// The most pages Document Setup takes (InDesign's 9999).
const MAX_PAGES: u32 = 9999;

craft_ui::words! {
    /// The words of the dialog and the layout menus (WordCraft's strings; DesignCraft's for the pages row).
    pub struct PageSetupDialogWords {
        title: "pageSetupDialog.title" = "Page Setup";
        document_setup: "pageSetupDialog.documentSetup" = "Document Setup";
        margins: "pageSetupDialog.margins" = "Margins";
        top: "pageSetupDialog.top" = "Top";
        bottom: "pageSetupDialog.bottom" = "Bottom";
        left: "pageSetupDialog.left" = "Left";
        right: "pageSetupDialog.right" = "Right";
        inside: "pageSetupDialog.inside" = "Inside";
        outside: "pageSetupDialog.outside" = "Outside";
        gutter: "pageSetupDialog.gutter" = "Gutter";
        orientation: "pageSetupDialog.orientation" = "Orientation";
        portrait: "pageSetupDialog.portrait" = "Portrait";
        landscape: "pageSetupDialog.landscape" = "Landscape";
        pages: "pageSetupDialog.pages" = "Pages";
        mirror_margins: "pageSetupDialog.mirrorMargins" = "Mirror margins";
        facing_pages: "pageSetupDialog.facingPages" = "Facing Pages";
        number_of_pages: "pageSetupDialog.numberOfPages" = "Number of Pages";
        paper: "pageSetupDialog.paper" = "Paper";
        paper_size: "pageSetupDialog.paperSize" = "Paper size";
        custom_size: "pageSetupDialog.customSize" = "Custom size";
        width: "pageSetupDialog.width" = "Width";
        height: "pageSetupDialog.height" = "Height";
        more_paper_sizes: "pageSetupDialog.morePaperSizes" = "More Paper Sizes\u{2026}";
        vertical_alignment: "pageSetupDialog.verticalAlignment" = "Vertical alignment";
        align_top: "pageSetupDialog.alignTop" = "Top";
        align_center: "pageSetupDialog.alignCenter" = "Center";
        align_justified: "pageSetupDialog.alignJustified" = "Justified";
        align_bottom: "pageSetupDialog.alignBottom" = "Bottom";
        headers_and_footers: "pageSetupDialog.headersAndFooters" = "Headers and footers";
        different_odd_even: "pageSetupDialog.differentOddEven" = "Different Odd & Even Pages";
        different_first: "pageSetupDialog.differentFirst" = "Different First Page";
        section_direction: "pageSetupDialog.sectionDirection" = "Section direction";
        left_to_right: "pageSetupDialog.leftToRight" = "Left-to-right";
        right_to_left: "pageSetupDialog.rightToLeft" = "Right-to-left";
        grid: "pageSetupDialog.grid" = "Document Grid";
        grid_none: "pageSetupDialog.gridNone" = "No grid";
        grid_lines: "pageSetupDialog.gridLines" = "Lines only";
        grid_lines_and_chars: "pageSetupDialog.gridLinesAndChars" = "Lines and characters";
        grid_snap_to_chars: "pageSetupDialog.gridSnapToChars" = "Text snaps to characters";
        chars_per_line: "pageSetupDialog.charsPerLine" = "Characters per line";
        lines_per_page: "pageSetupDialog.linesPerPage" = "Lines per page";
        margins_dont_fit: "pageSetupDialog.marginsDontFit" = "The margins don't fit on a page this size; make the margins smaller first.";
        /// `{name}` is the preset's name and `{size}` its margin on every side.
        uniform_preset: template "pageSetupDialog.uniformPreset" = "{name}  {size} all";
        custom_margins: "pageSetupDialog.customMargins" = "Custom Margins\u{2026}";
        /// `{width}` and `{height}` are the paper's sides with their unit.
        paper_dimensions: template "pageSetupDialog.paperDimensions" = "{width} \u{d7} {height}";
        size: "pageSetupDialog.size" = "Size";
        paper_letter: "pageSetupDialog.paperLetter" = "Letter";
        paper_legal: "pageSetupDialog.paperLegal" = "Legal";
        paper_executive: "pageSetupDialog.paperExecutive" = "Executive";
        paper_a3: "pageSetupDialog.paperA3" = "A3";
        paper_a4: "pageSetupDialog.paperA4" = "A4";
        paper_a5: "pageSetupDialog.paperA5" = "A5";
        paper_b4_jis: "pageSetupDialog.paperB4Jis" = "B4 (JIS)";
        paper_b5_jis: "pageSetupDialog.paperB5Jis" = "B5 (JIS)";
        paper_tabloid: "pageSetupDialog.paperTabloid" = "Tabloid";
        paper_statement: "pageSetupDialog.paperStatement" = "Statement";
        paper_envelope_10: "pageSetupDialog.paperEnvelope10" = "Envelope #10";
        paper_envelope_dl: "pageSetupDialog.paperEnvelopeDl" = "Envelope DL";
        paper_a0: "pageSetupDialog.paperA0" = "A0";
        paper_a1: "pageSetupDialog.paperA1" = "A1";
        paper_a2: "pageSetupDialog.paperA2" = "A2";
        paper_a6: "pageSetupDialog.paperA6" = "A6";
        paper_b5: "pageSetupDialog.paperB5" = "B5";
        paper_letter_half: "pageSetupDialog.paperLetterHalf" = "Letter - Half";
        paper_legal_half: "pageSetupDialog.paperLegalHalf" = "Legal - Half";
        paper_us_business_card: "pageSetupDialog.paperUsBusinessCard" = "US Business Card";
        preset_normal: "pageSetupDialog.presetNormal" = "Normal";
        preset_narrow: "pageSetupDialog.presetNarrow" = "Narrow";
        preset_moderate: "pageSetupDialog.presetModerate" = "Moderate";
        preset_wide: "pageSetupDialog.presetWide" = "Wide";
        preset_mirrored: "pageSetupDialog.presetMirrored" = "Mirrored";
        preset_office2003: "pageSetupDialog.presetOffice2003" = "Office 2003 Default";
    }
}

impl PageSetupDialogWords {
    /// The word for `paper`, matched on its English name; a paper this table has no word for (an app's own list) keeps
    /// its own name.
    pub fn paper_name(&self, paper: &PaperSize) -> Label {
        let word = match paper.name {
            "Letter" => &self.paper_letter,
            "Legal" => &self.paper_legal,
            "Executive" => &self.paper_executive,
            "A3" => &self.paper_a3,
            "A4" => &self.paper_a4,
            "A5" => &self.paper_a5,
            "B4 (JIS)" => &self.paper_b4_jis,
            "B5 (JIS)" => &self.paper_b5_jis,
            "Tabloid" => &self.paper_tabloid,
            "Statement" => &self.paper_statement,
            "Envelope #10" => &self.paper_envelope_10,
            "Envelope DL" => &self.paper_envelope_dl,
            "A0" => &self.paper_a0,
            "A1" => &self.paper_a1,
            "A2" => &self.paper_a2,
            "A6" => &self.paper_a6,
            "B5" => &self.paper_b5,
            "Letter - Half" => &self.paper_letter_half,
            "Legal - Half" => &self.paper_legal_half,
            "US Business Card" => &self.paper_us_business_card,
            other => return Label::from(other.to_owned()),
        };
        word.clone()
    }

    /// The word for `preset`, matched on its id; a preset this table has no word for keeps its own name.
    pub fn preset_name(&self, preset: &MarginPreset) -> Label {
        let word = match preset.id {
            "normal" => &self.preset_normal,
            "narrow" => &self.preset_narrow,
            "moderate" => &self.preset_moderate,
            "wide" => &self.preset_wide,
            "mirrored" => &self.preset_mirrored,
            "office2003" => &self.preset_office2003,
            _ => return Label::from(preset.name.to_owned()),
        };
        word.clone()
    }
}

/// Which row of the paper list is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaperChoice {
    Named(usize),
    Custom,
}

/// Whether Custom size stays chosen while the numbers match a paper name.
#[derive(Clone, Default)]
struct CustomSize(bool);

/// The setup from before the length gesture that is running, so a cancel restores everything the gesture touched (a
/// width typed past the height turns the orientation too).
#[derive(Clone, Default)]
struct Before(Option<PageSetup>);

/// `value` rounded to `places` decimals.
fn rounded(value: f64, places: usize) -> f64 {
    let scale = 10f64.powi(i32::try_from(places).unwrap_or(0));
    (value * scale).round() / scale
}

/// `points` in `unit`, treating a point as a pixel at 72 pixels per inch.
fn in_unit(unit: UnitKind, points: f64) -> f64 {
    unit.from_pixels(points, 0.0, UnitKind::POINTS_PER_INCH, UnitKind::POINTS_PER_INCH).unwrap_or(points)
}

/// `value` in `unit` as points.
fn to_points(unit: UnitKind, value: f64) -> f64 {
    unit.to_pixels(value, 0.0, UnitKind::POINTS_PER_INCH, UnitKind::POINTS_PER_INCH).unwrap_or(value)
}

/// What the dialog's rows share in one pass.
struct Rows<'a> {
    words: &'a PageSetupDialogWords,
    common: &'a CommonWords,
    units: Units,
    unit: UnitKind,
}

/// The columns of a grid of number rows: a caption and its field, twice ("Top: [ ]  Bottom: [ ]").
const PAIRS: [ColumnWidth; 4] = [ColumnWidth::Content; 4];

/// One length of the setup: where it is and which values it takes (points).
#[derive(Clone)]
struct Length {
    key: &'static str,
    label: Label,
    get: fn(&PageSetup) -> f64,
    set: fn(&mut PageSetup, f64),
    range: RangeInclusive<f64>,
}

/// Runs the events of a number field of the dialog in order: `write` puts their values into the setup. A `Begin`
/// remembers the setup; a `Cancel` puts the remembered setup back.
fn apply(form: &mut FormCx<'_, '_>, setup: &mut PageSetup, edit: &EditResponse<Option<f64>>, write: impl Fn(&mut PageSetup, f64)) {
    for event in &edit.events {
        match event {
            EditEvent::Begin { .. } => {
                form.memory::<Before>().0 = Some(setup.clone());
            }
            EditEvent::Cancel { .. } => {
                if let Some(before) = form.memory::<Before>().0.take() {
                    *setup = before;
                }
            }
            event => {
                if let Some(value) = event.value().copied().flatten() {
                    write(setup, value);
                }
                if matches!(event, EditEvent::Commit { .. } | EditEvent::Set { .. }) {
                    form.memory::<Before>().0 = None;
                }
            }
        }
    }
}

impl Rows<'_> {
    /// The caption cell of a grid row: `label` with the language's colon, dimmed while the field is disabled.
    fn caption(&self, ui: &mut Ui, label: &Label, enabled: bool) {
        Text::new(Label::from(self.common.caption.fill(&[("caption", label.as_str())]))).enabled(enabled).show(ui);
    }

    /// A length in the dialog's unit over the setup's value, as two grid cells (its caption and its field), which an
    /// edit changes (clamped to the range). A value that rounds to what the field showed changes nothing, so an
    /// untouched length keeps its exact value. `false` while the field's text is not a number.
    fn length(&self, ui: &mut Ui, form: &mut FormCx<'_, '_>, setup: &mut PageSetup, length: Length) -> bool {
        let Length { key, label, get, set, range } = length;
        let unit = self.unit;
        let places = *unit.decimals().end();
        let exact = get(setup);
        let shown = rounded(in_unit(unit, exact), places);
        let codec = NumberCodec::new(self.units.reader(Some(unit)), places);
        self.caption(ui, &label, true);
        let field = NumberField::new(label.clone(), Bind::given(Some(shown)))
            .id_salt(key)
            .codec(codec)
            .range(in_unit(unit, *range.start())..=in_unit(unit, *range.end()))
            .step(in_unit(unit, MARGIN_SCRUB_STEP))
            .unit(self.units.symbol(unit))
            .show(ui);
        apply(form, setup, &field.edit, |next, value| {
            let points = if rounded(value, places) == shown { exact } else { to_points(unit, value).clamp(*range.start(), *range.end()) };
            set(next, points);
        });
        !field.edit.unparsed
    }

    /// The Margins section: Top and Bottom, Left and Right, then the gutter, two to a line; the notice under them while
    /// the margins do not fit.
    fn margins(&self, ui: &mut Ui, form: &mut FormCx<'_, '_>, setup: &mut PageSetup, document_setup: bool) -> bool {
        let words = self.words;
        let (left, right) = if setup.mirrored { (&words.inside, &words.outside) } else { (&words.left, &words.right) };
        let side = 0.0..=MAX_PAGE_SIDE;
        let (left, right) = (left.clone(), right.clone());
        section(ui, &words.margins, |ui| {
            let mut valid = true;
            Grid::new(&PAIRS).show(ui, |ui, _| {
                let mut side_cells = |ui: &mut Ui, key: &'static str, label: Label, get: fn(&PageSetup) -> f64, set: fn(&mut PageSetup, f64)| {
                    valid &= self.length(ui, form, setup, Length { key, label, get, set, range: side.clone() });
                };
                side_cells(ui, "top", words.top.clone(), |s| s.margins.top, |s, v| s.margins.top = v);
                side_cells(ui, "bottom", words.bottom.clone(), |s| s.margins.bottom, |s, v| s.margins.bottom = v);
                side_cells(ui, "left", left.clone(), |s| s.margins.left, |s, v| s.margins.left = v);
                side_cells(ui, "right", right.clone(), |s| s.margins.right, |s, v| s.margins.right = v);
                if !document_setup {
                    side_cells(ui, "gutter", words.gutter.clone(), |s| s.gutter, |s, v| s.gutter = v);
                }
            });
            valid && self.fit_notice(ui, setup)
        })
        .unwrap_or(true)
    }

    /// The notice under the margins while they do not fit; `false` then.
    fn fit_notice(&self, ui: &mut Ui, setup: &PageSetup) -> bool {
        let fits = setup.margins.fit(setup.width, setup.height, setup.gutter);
        if !fits {
            Notice::new(Tone::Bad, self.words.margins_dont_fit.clone()).layout(NoticeLayout::Inline).show(ui);
        }
        fits
    }

    /// The Orientation section.
    fn orientation(&self, ui: &mut Ui, setup: &mut PageSetup) {
        let words = self.words;
        section(ui, &words.orientation, |ui| {
            let after = SideLabel::new().layout(FieldLayout::After);
            let group = RadioGroup::new(words.orientation.clone(), Bind::given(setup.orientation)).show(ui, |ui, cx| {
                Row::new().show(ui, |ui, _| {
                    cx.labelled(ui, "portrait", words.portrait.clone(), after.clone(), Radio::new(Orientation::Portrait));
                    cx.labelled(ui, "landscape", words.landscape.clone(), after.clone(), Radio::new(Orientation::Landscape));
                });
            });
            if let Some(orientation) = group.output.value().copied() {
                setup.set_orientation(orientation);
            }
        });
    }

    /// The Pages section: Mirror margins with the Paper rows under their own heading (Page Setup), or Facing Pages and
    /// Number of Pages (Document Setup, whose Paper is a section of its own).
    fn pages(&self, ui: &mut Ui, form: &mut FormCx<'_, '_>, setup: &mut PageSetup, document_setup: bool, papers: &[PaperSize]) -> bool {
        let words = self.words;
        let title = &words.pages;
        // The check box stands at the section's start, as WordCraft's dialog has it: outside the field group, whose
        // label column (Paper size:, Width:) would set it in the field column.
        GroupBox::new(title.clone(), None)
            .show(ui, |ui, _| {
                let label = if document_setup { &words.facing_pages } else { &words.mirror_margins };
                let check = Checkbox::new(Bind::given(setup.mirrored)).label(label.clone()).show(ui);
                if let Some(on) = check.value().copied() {
                    setup.mirrored = on;
                }
                if !document_setup {
                    TitleRow::new(words.paper.clone()).strength(HeaderStrength::Section).show(ui);
                }
                FieldGroup::new(title.clone())
                    .show(ui, |ui, _| {
                        if !document_setup {
                            return self.paper_rows(ui, form, setup, papers);
                        }
                        self.page_count(ui, form, setup)
                    })
                    .inner
            })
            .inner
            .flatten()
            .unwrap_or(true)
    }

    /// Document Setup's Number of Pages row; `false` while its text is not a number.
    fn page_count(&self, ui: &mut Ui, form: &mut FormCx<'_, '_>, setup: &mut PageSetup) -> bool {
        let words = self.words;
        let shown = f64::from(setup.pages.unwrap_or(1));
        let codec = NumberCodec::new(self.units.reader(None), 0);
        let field = Field::new(words.number_of_pages.clone())
            .id_salt("pages")
            .show(ui, |ui, _| {
                NumberField::new(words.number_of_pages.clone(), Bind::given(Some(shown)))
                    .codec(codec.clone())
                    .range(1.0..=f64::from(MAX_PAGES))
                    .show(ui)
            })
            .inner;
        let Some(field) = field else { return true };
        apply(form, setup, &field.edit, |next, value| {
            if value != shown {
                next.pages = Some(value.round() as u32);
            }
        });
        !field.edit.unparsed
    }

    /// The Paper section of Document Setup.
    fn paper(&self, ui: &mut Ui, form: &mut FormCx<'_, '_>, setup: &mut PageSetup, papers: &[PaperSize]) -> bool {
        section(ui, &self.words.paper, |ui| self.paper_rows(ui, form, setup, papers)).unwrap_or(true)
    }

    /// The Paper rows: the list of papers plus Custom size, then Width and Height on one line.
    fn paper_rows(&self, ui: &mut Ui, form: &mut FormCx<'_, '_>, setup: &mut PageSetup, papers: &[PaperSize]) -> bool {
        let words = self.words;
        let named = paper_named(papers, setup.width, setup.height).and_then(|paper| papers.iter().position(|other| other == paper));
        let custom = form.memory::<CustomSize>().0;
        let choice = match named {
            Some(index) if !custom => PaperChoice::Named(index),
            _ => PaperChoice::Custom,
        };
        let mut entries: Vec<Entry<PaperChoice>> =
            papers.iter().enumerate().map(|(index, paper)| Entry::new(PaperChoice::Named(index), words.paper_name(paper))).collect();
        entries.push(Entry::new(PaperChoice::Custom, words.custom_size.clone()));
        Field::new(words.paper_size.clone()).id_salt("paper").show(ui, |ui, _| {
            let picked = Select::new(words.paper_size.clone(), Bind::given(choice), &entries).show(ui);
            match picked.value().copied() {
                Some(PaperChoice::Named(index)) => {
                    if let Some(paper) = papers.get(index) {
                        let (width, height) = setup.orientation.turn((paper.width, paper.height));
                        setup.set_size(width, height);
                    }
                    form.memory::<CustomSize>().0 = false;
                }
                Some(PaperChoice::Custom) => form.memory::<CustomSize>().0 = true,
                None => {}
            }
        });
        let sides = MIN_PAGE_SIDE..=MAX_PAGE_SIDE;
        let width = Length { key: "width", label: words.width.clone(), get: |s| s.width, set: |s, v| s.set_size(v, s.height), range: sides.clone() };
        let height = Length { key: "height", label: words.height.clone(), get: |s| s.height, set: |s, v| s.set_size(s.width, v), range: sides };
        let mut valid = true;
        Grid::new(&PAIRS).show(ui, |ui, _| {
            let wide = self.length(ui, form, setup, width.clone());
            valid = self.length(ui, form, setup, height.clone()) && wide;
        });
        valid
    }

    /// The Document Grid section, shown while the setup has a grid: the kinds in a wrapping row, the two counts the
    /// kind enables on one line, then the section direction (with `direction`) and the vertical alignment (with
    /// `layout`). Without a grid, the section direction is a section of its own and the vertical alignment heads the
    /// Headers and footers section. `false` while a count's text is not a number.
    fn grid(&self, ui: &mut Ui, form: &mut FormCx<'_, '_>, setup: &mut PageSetup, layout: bool, direction: bool) -> bool {
        let words = self.words;
        let Some(grid) = setup.grid else {
            if direction {
                section(ui, &words.section_direction, |ui| self.direction_row(ui, setup));
            }
            return true;
        };
        section(ui, &words.grid, |ui| {
            let after = SideLabel::new().layout(FieldLayout::After);
            let group = RadioGroup::new(words.grid.clone(), Bind::given(grid.kind)).show(ui, |ui, cx| {
                Row::new().wrap(true).show(ui, |ui, _| {
                    cx.labelled(ui, "none", words.grid_none.clone(), after.clone(), Radio::new(GridKind::None));
                    cx.labelled(ui, "lines", words.grid_lines.clone(), after.clone(), Radio::new(GridKind::Lines));
                    cx.labelled(ui, "linesAndChars", words.grid_lines_and_chars.clone(), after.clone(), Radio::new(GridKind::LinesAndChars));
                    cx.labelled(ui, "snapToChars", words.grid_snap_to_chars.clone(), after.clone(), Radio::new(GridKind::SnapToChars));
                });
            });
            if let (Some(kind), Some(grid)) = (group.output.value().copied(), setup.grid.as_mut()) {
                grid.kind = kind;
            }
            let kind = setup.grid.map_or(grid.kind, |grid| grid.kind);
            let mut valid = true;
            Grid::new(&PAIRS).show(ui, |ui, _| {
                let chars = self.count(
                    ui,
                    form,
                    setup,
                    ("chars", words.chars_per_line.clone(), kind.chars_enabled()),
                    |grid| grid.chars_per_line,
                    |grid, count| grid.chars_per_line = count,
                );
                let lines = self.count(
                    ui,
                    form,
                    setup,
                    ("lines", words.lines_per_page.clone(), kind.lines_enabled()),
                    |grid| grid.lines_per_page,
                    |grid, count| grid.lines_per_page = count,
                );
                valid = chars && lines;
            });
            if direction {
                self.direction_row(ui, setup);
            }
            if layout {
                self.vertical_row(ui, setup);
            }
            valid
        })
        .unwrap_or(true)
    }

    /// One count of the grid as two grid cells (its caption and its field), a whole number from 1 to
    /// [`MAX_GRID_COUNT`] that `enabled` lets the user edit; `key`, `label` and `enabled` are given as one tuple.
    /// `false` while the field's text is not a number.
    fn count(
        &self,
        ui: &mut Ui,
        form: &mut FormCx<'_, '_>,
        setup: &mut PageSetup,
        (key, label, enabled): (&'static str, Label, bool),
        get: fn(&DocumentGrid) -> u32,
        set: fn(&mut DocumentGrid, u32),
    ) -> bool {
        let Some(grid) = setup.grid else { return true };
        let shown = f64::from(get(&grid));
        let codec = NumberCodec::new(self.units.reader(None), 0);
        self.caption(ui, &label, enabled);
        let field = NumberField::new(label.clone(), Bind::given(Some(shown)))
            .id_salt(key)
            .codec(codec)
            .range(1.0..=f64::from(MAX_GRID_COUNT))
            .enabled(enabled)
            .show(ui);
        apply(form, setup, &field.edit, |next, value| {
            if let Some(grid) = next.grid.as_mut() {
                set(grid, value.round().clamp(1.0, f64::from(MAX_GRID_COUNT)) as u32);
            }
        });
        !field.edit.unparsed
    }

    /// The Vertical alignment row.
    fn vertical_row(&self, ui: &mut Ui, setup: &mut PageSetup) {
        let words = self.words;
        let entries = [
            Entry::new(PageVerticalAlign::Top, words.align_top.clone()),
            Entry::new(PageVerticalAlign::Center, words.align_center.clone()),
            Entry::new(PageVerticalAlign::Justify, words.align_justified.clone()),
            Entry::new(PageVerticalAlign::Bottom, words.align_bottom.clone()),
        ];
        Field::new(words.vertical_alignment.clone()).id_salt("vertical").show(ui, |ui, _| {
            let picked = Select::new(words.vertical_alignment.clone(), Bind::given(setup.vertical_align), &entries).show(ui);
            if let Some(align) = picked.value().copied() {
                setup.vertical_align = align;
            }
        });
    }

    /// The Headers and footers section: odd and even pages and first page, under the vertical alignment when
    /// `vertical` (a setup without a grid, whose Document Grid section does not show it).
    fn layout(&self, ui: &mut Ui, setup: &mut PageSetup, vertical: bool) {
        let words = self.words;
        section(ui, &words.headers_and_footers, |ui| {
            if vertical {
                self.vertical_row(ui, setup);
            }
            let odd_even = Checkbox::new(Bind::given(setup.different_odd_even)).label(words.different_odd_even.clone()).show(ui);
            if let Some(on) = odd_even.value().copied() {
                setup.different_odd_even = on;
            }
            let first = Checkbox::new(Bind::given(setup.different_first)).label(words.different_first.clone()).show(ui);
            if let Some(on) = first.value().copied() {
                setup.different_first = on;
            }
        });
    }

    /// The Section direction row: its caption, then Left-to-right and Right-to-left on the same line.
    fn direction_row(&self, ui: &mut Ui, setup: &mut PageSetup) {
        let words = self.words;
        Row::new().show(ui, |ui, _| {
            self.caption(ui, &words.section_direction, true);
            let after = SideLabel::new().layout(FieldLayout::After);
            let group = RadioGroup::new(words.section_direction.clone(), Bind::given(setup.right_to_left)).show(ui, |ui, cx| {
                Row::new().show(ui, |ui, _| {
                    cx.labelled(ui, "ltr", words.left_to_right.clone(), after.clone(), Radio::new(false));
                    cx.labelled(ui, "rtl", words.right_to_left.clone(), after.clone(), Radio::new(true));
                });
            });
            if let Some(on) = group.output.value().copied() {
                setup.right_to_left = on;
            }
        });
    }
}

/// A section of the dialog: a framed box titled `title` over a field group, so the section's fields share one label
/// column.
fn section<R>(ui: &mut Ui, title: &Label, mut body: impl FnMut(&mut Ui) -> R) -> Option<R> {
    GroupBox::new(title.clone(), None).show(ui, |ui, _| FieldGroup::new(title.clone()).show(ui, |ui, _| body(ui)).inner).inner.flatten()
}

/// Page Setup (or Document Setup) as a form dialog over a [`PageSetup`]. OK answers the edited setup and is refused,
/// with [`PageSetupDialogWords::margins_dont_fit`] under the fields, while the margins don't fit the paper
/// ([`Margins::fit`](crate::setup::Margins::fit)).
///
/// Each section is a framed box; lengths and counts stand two to a line, caption then field.
///
/// - Margins: Top and Bottom, Left (Inside with mirrored margins) and Right (Outside), then Gutter, in the dialog's
///   [`unit`](PageSetupDialog::unit), each a number field that reads any unit, scrubs by [`MARGIN_SCRUB_STEP`] page
///   points and shows the unit's symbol.
/// - Orientation: Portrait and Landscape on one line; switching turns the size (and the margins)
///   ([`PageSetup::set_orientation`]).
/// - Pages: Mirror margins (Word) or Facing Pages and Number of Pages (InDesign, with
///   [`document_setup`](PageSetupDialog::document_setup)).
/// - Paper (under Pages in Page Setup, its own section in Document Setup): a list of
///   [`papers`](PageSetupDialog::papers) plus Custom size, and Width and Height ([`MIN_PAGE_SIDE`] to
///   [`MAX_PAGE_SIDE`]); Custom size stays chosen while the numbers match a name; a typed size wider than tall turns the
///   orientation ([`PageSetup::set_size`]).
/// - Document Grid, while the setup has a [`grid`](PageSetup::grid): its kinds in a wrapping row and the characters per
///   line and lines per page the kind enables; then, with [`direction`](PageSetupDialog::direction), Section direction
///   and, with [`layout`](PageSetupDialog::layout), Vertical alignment.
/// - With [`layout`](PageSetupDialog::layout): Headers and footers, Different Odd & Even Pages and Different First
///   Page (under Vertical alignment when the setup has no grid; Section direction is then a section of its own).
/// - A length or count gesture previews in the setup and the cancel of the gesture restores the whole setup from
///   before it; text no number can come from keeps OK disabled until it is fixed.
/// - Keys: Enter is OK, Escape Cancel, and holding Alt turns Cancel into Reset. The Top field has the focus when the
///   dialog opens.
#[must_use = "show the dialog with `show`"]
#[derive(Pattern, Props)]
pub struct PageSetupDialog<'a> {
    setup: Bind<'a, PageSetup>,
    /// The paper sizes listed (all of [`PAPER_SIZES`] by default).
    #[prop]
    papers: &'static [PaperSize],
    /// The unit the fields show (the app's measurement unit preference).
    #[prop]
    unit: UnitKind,
    /// InDesign's File > Document Setup: titled so, with Number of Pages and Facing Pages, no gutter and no layout
    /// group.
    #[prop(flag)]
    document_setup: bool,
    /// Shows the Layout group (vertical alignment, odd and even pages, first page).
    #[prop(flag)]
    layout: bool,
    /// Shows the section direction field.
    #[prop(flag)]
    direction: bool,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// OK files the setup in the held answer and the dialog stays open until the app answers it (`FormDialog::held`).
    #[prop(into)]
    held: Option<&'a mut craft_ui::dialog::HeldAnswer<PageSetup>>,
}

impl<'a> PageSetupDialog<'a> {
    /// A dialog that edits `setup`, with every paper size, in inches and with no layout group.
    pub fn new(setup: Bind<'a, PageSetup>) -> Self {
        PageSetupDialog {
            setup,
            papers: PAPER_SIZES,
            unit: UnitKind::Inches,
            document_setup: false,
            layout: false,
            direction: false,
            open: None,
            held: None,
        }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> FormOutput<PageSetup> {
        let PageSetupDialog { setup, papers, unit, document_setup, layout, direction, open, held } = self;
        let env = Env::of(ui);
        let words = env.words::<PageSetupDialogWords>();
        let common = env.words::<CommonWords>();
        let rows = Rows { words: &words, common: &common, units: env.units(), unit };
        let title = if document_setup { words.document_setup.clone() } else { words.title.clone() };
        let mut dialog = FormDialog::new(title, setup).outer_gesture(true);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        if let Some(held) = held {
            dialog = dialog.held(held);
        }
        dialog.show(ui, |ui, form, setup| {
            let margins = rows.margins(ui, form, setup, document_setup);
            rows.orientation(ui, setup);
            // Page Setup's Paper rows sit in the Pages section; Document Setup's are a section of their own.
            let pages = rows.pages(ui, form, setup, document_setup, papers);
            let paper = !document_setup || rows.paper(ui, form, setup, papers);
            let gridded = setup.grid.is_some();
            let grid = rows.grid(ui, form, setup, layout, direction);
            if layout {
                rows.layout(ui, setup, !gridded);
            }
            margins && pages && paper && grid
        })
    }
}

/// A choice from one of the Layout menus.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PageLayoutChoice {
    /// A margins preset (its margins and whether it mirrors).
    Margins(MarginPreset),
    /// The orientation to apply.
    Orientation(Orientation),
    /// A paper size, in the current orientation.
    Paper(PaperSize),
    /// Custom Margins or More Paper Sizes: open Page Setup.
    PageSetup,
}

/// Layout > Margins, Orientation and Size as menus, each item checked when it matches the page setup at the caret
/// (Word's check marks). Each method shows its menu from `opener` (a ribbon's menu button) and returns the choice that
/// ran.
#[must_use = "show a menu with `margins`, `orientation` or `size`"]
#[derive(Pattern, Props)]
pub struct PageLayoutMenus<'a> {
    setup: &'a PageSetup,
    /// The unit the lengths in the menus are shown in.
    #[prop]
    unit: UnitKind,
    /// The margin presets the Margins menu lists.
    #[prop]
    presets: &'a [MarginPreset],
    /// The paper sizes the Size menu lists.
    #[prop]
    papers: &'a [PaperSize],
    /// Binds the open state of the menu shown to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
}

impl<'a> PageLayoutMenus<'a> {
    /// The menus over `setup`, with lengths in inches.
    pub fn new(setup: &'a PageSetup) -> Self {
        PageLayoutMenus { setup, unit: UnitKind::Inches, presets: &MARGIN_PRESETS, papers: PAPER_SIZES, open: None }
    }

    fn length(&self, units: &Units, points: f64) -> String {
        units.show(self.unit, in_unit(self.unit, points))
    }

    /// The menu named `label` that hangs from `opener`.
    fn menu(&mut self, label: Label, opener: &Response) -> Menu<'a> {
        let menu = Menu::new(label).opened_by(opener, Trigger::Press);
        match self.open.take() {
            Some(open) => menu.open(open),
            None => menu,
        }
    }

    /// The presets (a uniform one reads "Normal  2.54 cm all" in the unit), a separator and Custom Margins; Mirrored
    /// is checked when the document mirrors margins.
    pub fn margins(mut self, ui: &mut Ui, opener: &Response) -> Option<PageLayoutChoice> {
        let env = Env::of(ui);
        let (words, units) = (env.words::<PageSetupDialogWords>(), env.units());
        let current = MarginPreset::matching(self.presets, &self.setup.margins, self.setup.mirrored);
        let mut chosen = None;
        self.menu(words.margins.clone(), opener).show(ui, |ui, _| {
            for preset in self.presets {
                let m = preset.margins;
                let uniform = m.top == m.bottom && m.top == m.left && m.top == m.right;
                let name = words.preset_name(preset);
                let label = if uniform {
                    words.uniform_preset.fill(&[("name", name.as_str()), ("size", self.length(&units, m.top).as_str())]).into()
                } else {
                    name
                };
                if MenuItem::new(label).check(current == Some(preset)).show(ui).ran {
                    chosen = Some(PageLayoutChoice::Margins(*preset));
                }
            }
            Divider::horizontal().show(ui);
            if MenuItem::new(words.custom_margins.clone()).show(ui).ran {
                chosen = Some(PageLayoutChoice::PageSetup);
            }
        });
        chosen
    }

    /// Portrait and Landscape, the current one checked.
    pub fn orientation(mut self, ui: &mut Ui, opener: &Response) -> Option<PageLayoutChoice> {
        let words = Env::of(ui).words::<PageSetupDialogWords>();
        let mut chosen = None;
        self.menu(words.orientation.clone(), opener).show(ui, |ui, _| {
            for (orientation, label) in [(Orientation::Portrait, &words.portrait), (Orientation::Landscape, &words.landscape)] {
                if MenuItem::new(label.clone()).check(self.setup.orientation == orientation).show(ui).ran {
                    chosen = Some(PageLayoutChoice::Orientation(orientation));
                }
            }
        });
        chosen
    }

    /// The paper sizes with their size in the unit on the detail line, a separator and More Paper Sizes.
    pub fn size(mut self, ui: &mut Ui, opener: &Response) -> Option<PageLayoutChoice> {
        let env = Env::of(ui);
        let (words, units) = (env.words::<PageSetupDialogWords>(), env.units());
        let current = paper_named(self.papers, self.setup.width, self.setup.height);
        let mut chosen = None;
        self.menu(words.size.clone(), opener).show(ui, |ui, _| {
            for paper in self.papers {
                let (width, height) = (self.length(&units, paper.width), self.length(&units, paper.height));
                let detail = words.paper_dimensions.fill(&[("width", width.as_str()), ("height", height.as_str())]);
                if MenuItem::new(words.paper_name(paper)).detail(detail).check(current == Some(paper)).show(ui).ran {
                    chosen = Some(PageLayoutChoice::Paper(*paper));
                }
            }
            Divider::horizontal().show(ui);
            if MenuItem::new(words.more_paper_sizes.clone()).show(ui).ran {
                chosen = Some(PageLayoutChoice::PageSetup);
            }
        });
        chosen
    }
}
