//! Document fonts and missing fonts (F14.08): which fonts a document's text uses, which of them
//! are not installed, and the dialog that lists them with a replacement for each (Photoshop's
//! "missing fonts" warning on open; VectorCraft's and FigCraft's Document Fonts lists).

#[cfg(feature = "screen-testing")]
mod stories;

use std::borrow::Cow;
use std::sync::Arc;

use craft_behavior::{DisplayOrder, ItemSource};
use craft_font_catalog::{Catalog, Match};
use craft_ui::contract::{Bind, SaveKey, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing};
use craft_ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::display::Text;
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, ListRow};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::sections::{DialogRole, FooterButton, Section};
use egui::Ui;

use crate::picker::FontPicker;

/// The family text falls back to when a missing font gets no replacement (PhotoCraft's default
/// type family).
const DEFAULT_FAMILY: &str = "Inter";

/// Whether the Document Fonts section is open, kept across runs.
const DOCUMENT_FONTS_OPEN: SaveKey = SaveKey::new("font_manager.documentFonts");

craft_ui::words! {
    /// The dialog's words, English by default.
    #[non_exhaustive]
    pub struct MissingFontsWords {
        title: "missingFonts.title" = "Missing Fonts";
        /// `{n}` the number of missing fonts.
        message: count "missingFonts.message" = "{n} font used in this document is not installed." | "{n} fonts used in this document are not installed.";
        font: "missingFonts.font" = "Font";
        replace_with: "missingFonts.replaceWith" = "Replace With";
        default_replacement: "missingFonts.default" = "Default font";
        replace: "missingFonts.replace" = "Replace Fonts";
        dont_replace: "missingFonts.dontReplace" = "Don't Replace";
        used_fonts: "missingFonts.usedFonts" = "Document Fonts";
        installed: "missingFonts.installed" = "Installed";
        missing: "missingFonts.missing" = "Missing";
        warning: template "missingFonts.warning" = "font '{family}' is not installed; text may render using a fallback face";
        missing_font: template "missingFonts.missingFont" = "{font} ({status})";
        installed_style: template "missingFonts.installedStyle" = "{status} ({style})";
        font_status: template "missingFonts.fontStatus" = "{font}: {status}";
    }
}

/// A font a document names: family and style as stored in the document.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct DocumentFont {
    /// The family name as the document stores it.
    pub family: String,
    /// The style name as the document stores it.
    pub style: String,
    /// The PostScript name, when the document stores one (PDF, PSD).
    pub postscript_name: Option<String>,
}

impl DocumentFont {
    /// A font named `family` in `style`, with the PostScript name when the document stores one.
    pub fn new(family: &str, style: &str, postscript_name: Option<&str>) -> Self {
        DocumentFont { family: family.to_owned(), style: style.to_owned(), postscript_name: postscript_name.map(str::to_owned) }
    }
}

/// Whether a document font resolves on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FontStatus {
    /// Installed as named.
    Installed,
    /// Another style of the family stands in (`Match::Style` in the catalog).
    OtherStyle(String),
    /// Nothing matches; text shapes with the default font until replaced.
    Missing,
}

/// A document font with its status.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DocumentFontRow {
    /// The font the document names.
    pub font: DocumentFont,
    /// Whether it resolves on this machine.
    pub status: FontStatus,
}

/// Resolve each of `fonts` against the installed catalog, document order kept, duplicates merged.
/// A font with an empty family is `Installed`; one whose PostScript name an installed face has is
/// `Installed` whatever its family says; otherwise family and style go through
/// [`Catalog::resolve`].
// pc: crates/engine/src/cmd/type_extra_cmds.rs:617 `missing_fonts` (PostScript name first, then
// family + style through `Catalog::resolve`).
pub fn document_fonts(catalog: &Catalog, fonts: &[DocumentFont]) -> Vec<DocumentFontRow> {
    let mut rows: Vec<DocumentFontRow> = Vec::new();
    for font in fonts {
        if rows.iter().any(|r| r.font == *font) {
            continue;
        }
        let status = if font.family.is_empty() || font.postscript_name.as_deref().is_some_and(|ps| catalog.by_postscript_name(ps).is_some()) {
            FontStatus::Installed
        } else {
            match catalog.resolve(&font.family, &font.style) {
                Some((_, Match::Exact)) => FontStatus::Installed,
                Some((face, Match::NearestStyle)) => FontStatus::OtherStyle(face.style),
                None => FontStatus::Missing,
            }
        };
        rows.push(DocumentFontRow { font: font.clone(), status });
    }
    rows
}

/// One warning per missing family for the open job's warning list, in document order, worded by
/// `words` (PhotoCraft's `missing_font_warnings` text by default).
// pc: crates/engine/src/cmd/type_extra_cmds.rs:647 `missing_font_warnings`.
pub fn missing_font_warnings(rows: &[DocumentFontRow], words: &MissingFontsWords) -> Vec<String> {
    let mut families: Vec<&str> = Vec::new();
    for row in rows.iter().filter(|r| r.status == FontStatus::Missing) {
        if !families.contains(&row.font.family.as_str()) {
            families.push(&row.font.family);
        }
    }
    families.into_iter().map(|family| words.warning.fill(&[("family", family)])).collect()
}

/// What the user chose for the missing fonts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MissingFontsResult {
    /// (missing font, replacement family) for every row the user set; the default font where unset.
    pub replacements: Vec<(DocumentFont, String)>,
}

/// The Missing Fonts dialog on the dialog host, opened by the app when a document with missing
/// fonts opens or when the user picks Type › Manage Missing Fonts.
///
/// A message with the count, then a row per missing font: its name (marked missing) and the
/// [`FontPicker`] that picks its replacement, a Document Fonts section listing every font with its status, and
/// the footer Replace Fonts (default) / Don't Replace (Cancel). Tab walks the pickers; Enter is Replace Fonts
/// unless a picker holds it. Result: [`MissingFontsResult`] on Replace; Cancel leaves the text on the default font
/// with the names kept.
#[derive(craft_ui::contract::Pattern)]
pub struct MissingFontsDialog {
    id: DialogId,
    rows: Vec<DocumentFontRow>,
    catalog: Arc<Catalog>,
    replacements: Vec<Option<String>>,
    default_family: String,
}

impl MissingFontsDialog {
    /// Over `rows` from [`document_fonts`]; only `Missing` rows get a replacement picker.
    pub fn new(catalog: Arc<Catalog>, rows: Vec<DocumentFontRow>) -> Self {
        let n = rows.len();
        MissingFontsDialog { id: dialog_id("missing-fonts"), rows, catalog, replacements: vec![None; n], default_family: DEFAULT_FAMILY.to_owned() }
    }

    /// The family a missing font gets when the user sets none (default `Inter`, PhotoCraft's
    /// default type family).
    pub fn default_family(mut self, family: impl Into<String>) -> Self {
        self.default_family = family.into();
        self
    }

    /// How many rows are missing.
    pub fn missing_count(&self) -> usize {
        self.rows.iter().filter(|r| r.status == FontStatus::Missing).count()
    }
}

/// The text rows of the Document Fonts list as a collection source.
struct Listed<'a>(&'a [String]);

impl DisplayOrder<usize> for Listed<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.0[at].as_str()))
    }
}

impl ItemSource<usize> for Listed<'_> {}

impl StackDialog for MissingFontsDialog {
    type Result = MissingFontsResult;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<MissingFontsResult>> {
        let env = Env::of(ui);
        let words = env.words::<MissingFontsWords>();
        let Self { rows, catalog, replacements, default_family, .. } = self;
        let missing = rows.iter().filter(|r| r.status == FontStatus::Missing).count() as u64;
        let message = words.message.text(missing, &env.format().integer(missing));
        let listed: Vec<String> = rows.iter().map(|row| status_line(row, &words)).collect();
        let shown = Dialog::new(words.title.clone())
            .buttons([
                FooterButton::new(words.replace.clone(), DialogRole::Accept),
                FooterButton::new(words.dont_replace.clone(), DialogRole::Cancel),
            ])
            .show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    Text::new(message.clone()).show(ui);
                    FieldGroup::new(words.replace_with.clone()).show(ui, |ui, _| {
                        for (i, row) in rows.iter().enumerate().filter(|(_, r)| r.status == FontStatus::Missing) {
                            let name = font_name(&row.font);
                            let label = Label::from(words.missing_font.fill(&[("font", &name), ("status", words.missing.as_str())]));
                            Field::new(label.clone()).show(ui, |ui, _| {
                                let mut current = replacements[i].clone().unwrap_or_else(|| default_family.clone());
                                let out = FontPicker::new(label.clone(), Bind::bound(&mut current), catalog).show(ui);
                                if let Some(picked) = out.picked {
                                    replacements[i] = Some(picked);
                                }
                            });
                        }
                    });
                    Section::new(words.used_fonts.clone()).initially_open(false).remember(DOCUMENT_FONTS_OPEN).show(ui, |ui, _| {
                        List::<usize>::new(words.font.clone()).select(SelectMode::Single).sizing(Sizing::Content).show_items(
                            ui,
                            &Listed(&listed),
                            &mut |key, _| ListRow::new(listed[*key].clone()),
                        );
                    });
                });
            });
        shown.output.result(|| {
            let replacements = rows
                .iter()
                .zip(replacements.iter())
                .filter(|(r, _)| r.status == FontStatus::Missing)
                .map(|(r, to)| (r.font.clone(), to.clone().unwrap_or_else(|| default_family.clone())))
                .collect();
            MissingFontsResult { replacements }
        })
    }
}

fn font_name(font: &DocumentFont) -> String {
    if font.style.is_empty() { font.family.clone() } else { format!("{} {}", font.family, font.style) }
}

/// The Document Fonts line of a row: the font and its status.
fn status_line(row: &DocumentFontRow, words: &MissingFontsWords) -> String {
    let status = match &row.status {
        FontStatus::Installed => words.installed.as_str().to_owned(),
        FontStatus::OtherStyle(style) => words.installed_style.fill(&[("status", words.installed.as_str()), ("style", style.as_str())]),
        FontStatus::Missing => words.missing.as_str().to_owned(),
    };
    words.font_status.fill(&[("font", &font_name(&row.font)), ("status", &status)])
}
