//! `PageSetupDialog`: Word's Page Setup and InDesign's Document Setup as a form dialog over a [`PageSetup`], and
//! `PageLayoutMenus`: Layout > Margins, Orientation and Size as menus.
//!
//! ```ignore
//! let shown = PageSetupDialog::new(Bind::bound(&mut draft)).unit(prefs.unit).layout().opened(Bind::bound(&mut open)).show(ui);
//! if let Some(setup) = shown.accepted { doc.apply_page_setup(setup); } // one undo step
//! // the Layout tab:
//! if let Some(choice) = PageLayoutMenus::new(&doc.page_setup()).unit(prefs.unit).margins(ui, &margins_button) { doc.apply(choice) }
//! ```

#[cfg(feature = "screen-testing")]
mod stories;

use std::ops::RangeInclusive;

use craft_behavior::{EditEvent, NumberCodec};
use craft_theme::variant::Tone;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, EditResponse, Entry, FieldLayout, Pattern, Props, Remember as _, Show as _, ShowContainer as _, SideLabel, Trigger};
use craft_ui::display::Divider;
use craft_ui::form::{Field, FieldGroup};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::patterns::form_dialogs::{FormCx, FormDialog, FormOutput};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, UnitKind, Units};
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::text::NumberField;
use egui::{Response, Ui};

use crate::setup::{
    MARGIN_PRESETS, MAX_PAGE_SIDE, MIN_PAGE_SIDE, MarginPreset, Orientation, PAPER_SIZES, PageSetup, PageVerticalAlign, PaperSize, paper_named,
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
    units: Units,
    unit: UnitKind,
}

/// One length of the setup: where it is and which values it takes (points).
struct Length {
    key: &'static str,
    label: Label,
    get: fn(&PageSetup) -> f64,
    set: fn(&mut PageSetup, f64),
    range: RangeInclusive<f64>,
    first: bool,
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
    /// A length field in the dialog's unit over the setup's value, which an edit changes (clamped to the range). A value
    /// that rounds to what the field showed changes nothing, so an untouched length keeps its exact value. `false`
    /// while the field's text is not a number.
    fn length(&self, ui: &mut Ui, form: &mut FormCx<'_, '_>, setup: &mut PageSetup, length: Length) -> bool {
        let Length { key, label, get, set, range, first } = length;
        let unit = self.unit;
        let places = *unit.decimals().end();
        let exact = get(setup);
        let shown = rounded(in_unit(unit, exact), places);
        let codec = NumberCodec::new(self.units.reader(Some(unit)), places);
        let field = Field::new(label.clone())
            .id_salt(key)
            .show(ui, |ui, _| {
                NumberField::new(label.clone(), Bind::given(Some(shown)))
                    .codec(codec.clone())
                    .range(in_unit(unit, *range.start())..=in_unit(unit, *range.end()))
                    .step(in_unit(unit, MARGIN_SCRUB_STEP))
                    .unit(self.units.symbol(unit))
                    .show(ui)
            })
            .inner;
        let Some(field) = field else { return true };
        if first && form.first() {
            focus::request(ui.ctx(), FocusTarget::Id(field.response.id));
        }
        apply(form, setup, &field.edit, |next, value| {
            let points = if rounded(value, places) == shown { exact } else { to_points(unit, value).clamp(*range.start(), *range.end()) };
            set(next, points);
        });
        !field.edit.unparsed
    }

    /// The Margins group: the four sides and the gutter, with the notice under them while the margins do not fit.
    fn margins(&self, ui: &mut Ui, form: &mut FormCx<'_, '_>, setup: &mut PageSetup, document_setup: bool) -> bool {
        let words = self.words;
        let (left, right) = if setup.mirrored { (&words.inside, &words.outside) } else { (&words.left, &words.right) };
        let side = 0.0..=MAX_PAGE_SIDE;
        let (left, right) = (left.clone(), right.clone());
        FieldGroup::new(words.margins.clone())
            .show(ui, |ui, _| {
                let mut valid = true;
                let mut side_row = |ui: &mut Ui,
                                    form: &mut FormCx<'_, '_>,
                                    setup: &mut PageSetup,
                                    key: &'static str,
                                    label: Label,
                                    get: fn(&PageSetup) -> f64,
                                    set: fn(&mut PageSetup, f64),
                                    first: bool| {
                    valid &= self.length(ui, form, setup, Length { key, label, get, set, range: side.clone(), first });
                };
                side_row(ui, form, setup, "top", words.top.clone(), |s| s.margins.top, |s, v| s.margins.top = v, true);
                side_row(ui, form, setup, "bottom", words.bottom.clone(), |s| s.margins.bottom, |s, v| s.margins.bottom = v, false);
                side_row(ui, form, setup, "left", left.clone(), |s| s.margins.left, |s, v| s.margins.left = v, false);
                side_row(ui, form, setup, "right", right.clone(), |s| s.margins.right, |s, v| s.margins.right = v, false);
                if !document_setup {
                    side_row(ui, form, setup, "gutter", words.gutter.clone(), |s| s.gutter, |s, v| s.gutter = v, false);
                }
                valid
            })
            .inner
            .unwrap_or(true)
            && self.fit_notice(ui, setup)
    }

    /// The notice under the margins while they do not fit; `false` then.
    fn fit_notice(&self, ui: &mut Ui, setup: &PageSetup) -> bool {
        let fits = setup.margins.fit(setup.width, setup.height, setup.gutter);
        if !fits {
            Notice::new(Tone::Bad, self.words.margins_dont_fit.clone()).layout(NoticeLayout::Inline).show(ui);
        }
        fits
    }

    /// The Orientation group.
    fn orientation(&self, ui: &mut Ui, setup: &mut PageSetup) {
        let words = self.words;
        FieldGroup::new(words.orientation.clone()).show(ui, |ui, _| {
            let after = SideLabel::new().layout(FieldLayout::After);
            let group = RadioGroup::new(words.orientation.clone(), Bind::given(setup.orientation)).show(ui, |ui, cx| {
                cx.labelled(ui, "portrait", words.portrait.clone(), after.clone(), Radio::new(Orientation::Portrait));
                cx.labelled(ui, "landscape", words.landscape.clone(), after.clone(), Radio::new(Orientation::Landscape));
            });
            if let Some(orientation) = group.output.value().copied() {
                setup.set_orientation(orientation);
            }
        });
    }

    /// The Pages group: Mirror margins, or Facing Pages and Number of Pages in Document Setup.
    fn pages(&self, ui: &mut Ui, form: &mut FormCx<'_, '_>, setup: &mut PageSetup, document_setup: bool) -> bool {
        let words = self.words;
        FieldGroup::new(words.pages.clone())
            .show(ui, |ui, _| {
                let label = if document_setup { &words.facing_pages } else { &words.mirror_margins };
                let check = Checkbox::new(Bind::given(setup.mirrored)).label(label.clone()).show(ui);
                if let Some(on) = check.value().copied() {
                    setup.mirrored = on;
                }
                if !document_setup {
                    return true;
                }
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
            })
            .inner
            .unwrap_or(true)
    }

    /// The Paper group: the list of papers plus Custom size, and Width and Height.
    fn paper(&self, ui: &mut Ui, form: &mut FormCx<'_, '_>, setup: &mut PageSetup, papers: &[PaperSize]) -> bool {
        let words = self.words;
        FieldGroup::new(words.paper.clone())
            .show(ui, |ui, _| {
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
                let width = Length {
                    key: "width",
                    label: words.width.clone(),
                    get: |s| s.width,
                    set: |s, v| s.set_size(v, s.height),
                    range: sides.clone(),
                    first: false,
                };
                let height = Length {
                    key: "height",
                    label: words.height.clone(),
                    get: |s| s.height,
                    set: |s, v| s.set_size(s.width, v),
                    range: sides,
                    first: false,
                };
                let wide = self.length(ui, form, setup, width);
                self.length(ui, form, setup, height) && wide
            })
            .inner
            .unwrap_or(true)
    }

    /// The Headers and footers group: vertical alignment, odd and even pages, first page.
    fn layout(&self, ui: &mut Ui, setup: &mut PageSetup) {
        let words = self.words;
        FieldGroup::new(words.headers_and_footers.clone()).show(ui, |ui, _| {
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

    /// The section direction group.
    fn direction(&self, ui: &mut Ui, setup: &mut PageSetup) {
        let words = self.words;
        FieldGroup::new(words.section_direction.clone()).show(ui, |ui, _| {
            let after = SideLabel::new().layout(FieldLayout::After);
            let group = RadioGroup::new(words.section_direction.clone(), Bind::given(setup.right_to_left)).show(ui, |ui, cx| {
                cx.labelled(ui, "ltr", words.left_to_right.clone(), after.clone(), Radio::new(false));
                cx.labelled(ui, "rtl", words.right_to_left.clone(), after.clone(), Radio::new(true));
            });
            if let Some(on) = group.output.value().copied() {
                setup.right_to_left = on;
            }
        });
    }
}

/// Page Setup (or Document Setup) as a form dialog over a [`PageSetup`]. OK answers the edited setup and is refused,
/// with [`PageSetupDialogWords::margins_dont_fit`] under the fields, while the margins don't fit the paper
/// ([`Margins::fit`](crate::setup::Margins::fit)).
///
/// - Margins: Top, Bottom, Left (Inside with mirrored margins), Right (Outside) and Gutter, in the dialog's
///   [`unit`](PageSetupDialog::unit), each a number field that reads any unit, scrubs by [`MARGIN_SCRUB_STEP`] page
///   points and shows the unit's symbol.
/// - Orientation: Portrait and Landscape; switching turns the size (and the margins)
///   ([`PageSetup::set_orientation`]).
/// - Pages: Mirror margins (Word) or Facing Pages and Number of Pages (InDesign, with
///   [`document_setup`](PageSetupDialog::document_setup)).
/// - Paper: a list of [`papers`](PageSetupDialog::papers) plus Custom size, and Width and Height
///   ([`MIN_PAGE_SIDE`] to [`MAX_PAGE_SIDE`]); Custom size stays chosen while the numbers match a name; a typed size
///   wider than tall turns the orientation ([`PageSetup::set_size`]).
/// - With [`layout`](PageSetupDialog::layout): Vertical alignment, Different Odd & Even Pages, Different First Page;
///   with [`direction`](PageSetupDialog::direction): Section direction.
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
}

impl<'a> PageSetupDialog<'a> {
    /// A dialog that edits `setup`, with every paper size, in inches and with no layout group.
    pub fn new(setup: Bind<'a, PageSetup>) -> Self {
        PageSetupDialog { setup, papers: PAPER_SIZES, unit: UnitKind::Inches, document_setup: false, layout: false, direction: false, open: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> FormOutput<PageSetup> {
        let PageSetupDialog { setup, papers, unit, document_setup, layout, direction, open } = self;
        let env = Env::of(ui);
        let words = env.words::<PageSetupDialogWords>();
        let rows = Rows { words: &words, units: env.units(), unit };
        let title = if document_setup { words.document_setup.clone() } else { words.title.clone() };
        let mut dialog = FormDialog::new(title, setup).outer_gesture(true);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        dialog.show(ui, |ui, form, setup| {
            let margins = rows.margins(ui, form, setup, document_setup);
            rows.orientation(ui, setup);
            let pages = rows.pages(ui, form, setup, document_setup);
            let paper = rows.paper(ui, form, setup, papers);
            if layout {
                rows.layout(ui, setup);
            }
            if direction {
                rows.direction(ui, setup);
            }
            margins && pages && paper
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
