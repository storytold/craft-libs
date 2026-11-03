//! The panel and the right-click section. Looks follow the host's theme through [`Colors`].
use std::borrow::Cow;

use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, Ui, vec2};

use crate::review::{CHANGED, NO_ISSUE, NOT_SPELLING, OTHER_DOCUMENT, SAME_REPLACEMENT};
use crate::{Action, Host, Issue, IssueKind, Review};

/// The application's translations.
pub trait Text {
    /// A string from [`STRINGS`], by its English source text.
    fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str>;
    /// A label put into display order (the app's bidi reordering).
    fn visual(&self, s: &str) -> String {
        s.to_string()
    }
}

/// Every string looked up with [`Text::ui`], for the shared catalog.
pub const STRINGS: &[&str] = &[
    "Spelling",
    "Preferred Form",
    "Suggestions",
    "(no suggestions)",
    "Ignore Once",
    "Ignore All",
    "Add to Dictionary",
    "Change…",
    "Change to:",
    "Change",
    "Check Spelling",
    "Check Spelling…",
    "Check Again",
    "Check This Document",
    "Check the spelling of the whole document.",
    "No issues found.",
    "The spelling check is complete. Some issues were ignored.",
    NO_ISSUE,
    NOT_SPELLING,
    CHANGED,
    OTHER_DOCUMENT,
    SAME_REPLACEMENT,
];

/// The host's theme.
#[derive(Clone, Copy, Debug)]
pub struct Colors {
    pub text: Color32,
    pub dim: Color32,
    pub field: Color32,
    pub field_border: Color32,
    pub hover: Color32,
    /// The underline of a misspelled word.
    pub spelling: Color32,
    /// The underline of a preferred form.
    pub grammar: Color32,
    pub error: Color32,
}

fn label(ui: &mut Ui, text: &dyn Text, source: &str, size: f32, color: Color32) -> egui::Response {
    ui.label(RichText::new(text.visual(&text.ui(source))).size(size).color(color))
}

fn link(ui: &mut Ui, text: &dyn Text, source: &str, colors: &Colors) -> bool {
    ui.add(egui::Button::new(RichText::new(text.visual(&text.ui(source))).color(colors.text).underline()).frame(false)).clicked()
}

fn context(ui: &mut Ui, review: &Review, issue: &Issue, colors: &Colors) {
    let format = egui::TextFormat { font_id: FontId::proportional(13.0), color: colors.text, ..Default::default() };
    let mark = if issue.kind == IssueKind::Spelling { colors.spelling } else { colors.grammar };
    let mut job = egui::text::LayoutJob::default();
    job.append(&review.before, 0.0, format.clone());
    job.append(&issue.word, 0.0, egui::TextFormat { underline: Stroke::new(1.5, mark), ..format.clone() });
    job.append(&review.after, 0.0, format);
    let width = ui.available_width();
    egui::Frame::NONE.fill(colors.field).stroke(Stroke::new(1.0, colors.field_border)).corner_radius(2.0).inner_margin(8).show(ui, |ui| {
        ui.set_min_width((width - 18.0).max(0.0));
        ui.add(egui::Label::new(job).wrap());
    });
}

/// A full-width row that applies its suggestion when clicked.
fn suggestion_row(ui: &mut Ui, suggestion: &str, colors: &Colors) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 25.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, suggestion));
    if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, colors.hover);
    }
    ui.painter().with_clip_rect(rect.shrink2(vec2(8.0, 0.0))).text(
        rect.left_center() + vec2(8.0, 0.0),
        Align2::LEFT_CENTER,
        suggestion,
        FontId::proportional(13.0),
        colors.text,
    );
    response.clicked()
}

/// What to do after the panel has been drawn, so that nothing changes under its borrows.
enum After {
    Start,
    Act(Action),
}

/// Draws the review. Starts one from its own button if there is none yet.
pub fn panel(ui: &mut Ui, review: &mut Review, host: &mut dyn Host, text: &dyn Text, colors: &Colors) {
    let after = egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| body(ui, review, &*host, text, colors)).inner;
    match after {
        Some(After::Start) => review.start(host),
        Some(After::Act(action)) => {
            // A failure stays in the review for the next frame.
            let _ = review.act(host, action);
        }
        None => {}
    }
}

fn body(ui: &mut Ui, review: &mut Review, host: &dyn Host, text: &dyn Text, colors: &Colors) -> Option<After> {
    if !review.started() {
        label(ui, text, "Check the spelling of the whole document.", 12.0, colors.text);
        ui.add_space(8.0);
        return ui.button(text.visual(&text.ui("Check Spelling"))).clicked().then_some(After::Start);
    }
    if review.other_document(host) {
        label(ui, text, OTHER_DOCUMENT, 12.0, colors.text);
        ui.add_space(8.0);
        return ui.button(text.visual(&text.ui("Check This Document"))).clicked().then_some(After::Start);
    }
    if let Some(notice) = host.notice() {
        ui.label(RichText::new(notice).size(11.0).color(colors.dim));
        ui.add_space(8.0);
    }
    let mut after = None;
    match review.current().cloned() {
        Some(issue) => after = current_issue(ui, review, &issue, host, text, colors),
        None => {
            let done = if review.ignored_any() { "The spelling check is complete. Some issues were ignored." } else { "No issues found." };
            label(ui, text, done, 12.0, colors.text);
            ui.add_space(8.0);
            if link(ui, text, "Check Again", colors) {
                after = Some(After::Act(Action::Check));
            }
        }
    }
    if !review.message().is_empty() {
        ui.add_space(8.0);
        label(ui, text, review.message(), 12.0, colors.error);
    }
    after
}

fn current_issue(ui: &mut Ui, review: &mut Review, issue: &Issue, host: &dyn Host, text: &dyn Text, colors: &Colors) -> Option<After> {
    let spelling = issue.kind == IssueKind::Spelling;
    let heading = text.visual(&text.ui(if spelling { "Spelling" } else { "Preferred Form" }));
    ui.label(RichText::new(heading).size(14.0).strong().color(colors.text));
    if !issue.message.is_empty() {
        ui.label(RichText::new(&issue.message).size(11.0).color(colors.dim));
    }
    ui.add_space(8.0);
    context(ui, review, issue, colors);
    ui.add_space(12.0);
    if review.stale(host) {
        label(ui, text, CHANGED, 12.0, colors.text);
        ui.add_space(8.0);
        return link(ui, text, "Check Again", colors).then_some(After::Act(Action::Check));
    }
    let mut after = None;
    label(ui, text, "Suggestions", 11.0, colors.dim);
    if issue.suggestions.is_empty() {
        ui.label(RichText::new(text.visual(&text.ui("(no suggestions)"))).italics().color(colors.text));
    } else {
        egui::Frame::NONE.fill(colors.field).stroke(Stroke::new(1.0, colors.field_border)).corner_radius(2.0).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for suggestion in &issue.suggestions {
                if suggestion_row(ui, suggestion, colors) {
                    after = Some(After::Act(Action::Change(Some(suggestion.clone()))));
                }
            }
        });
    }
    ui.add_space(8.0);
    // On wrapping lines: a docked panel is often too short for a column of them to stay in view.
    ui.horizontal_wrapped(|ui| {
        if link(ui, text, "Ignore Once", colors) {
            after = Some(After::Act(Action::IgnoreOnce));
        }
        if spelling && link(ui, text, "Ignore All", colors) {
            after = Some(After::Act(Action::IgnoreAll));
        }
        if spelling && link(ui, text, "Add to Dictionary", colors) {
            after = Some(After::Act(Action::AddToDictionary));
        }
        if !review.manual && link(ui, text, "Change…", colors) {
            review.manual = true;
        }
    });
    if review.manual {
        ui.add_space(8.0);
        let name = label(ui, text, "Change to:", 12.0, colors.text);
        let field = ui.add(egui::TextEdit::singleline(&mut review.replacement).desired_width(f32::INFINITY)).labelled_by(name.id);
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let button = egui::Button::new(text.visual(&text.ui("Change")));
        if (ui.add_enabled(!review.replacement.is_empty(), button).clicked() || enter) && !review.replacement.is_empty() {
            after = Some(After::Act(Action::Change(None)));
        }
    }
    after
}

/// What was chosen in the right-click section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextChoice {
    Change(String),
    AddToDictionary,
    /// Open the panel.
    Review,
}

/// The first section of the right-click menu on `issue`, whose suggestions have been asked for.
pub fn context_section(ui: &mut Ui, issue: &Issue, text: &dyn Text) -> Option<ContextChoice> {
    let mut choice = None;
    if issue.suggestions.is_empty() {
        ui.add_enabled(false, egui::Button::new(text.visual(&text.ui("(no suggestions)"))));
    }
    for suggestion in &issue.suggestions {
        if ui.button(RichText::new(suggestion).strong()).clicked() {
            choice = Some(ContextChoice::Change(suggestion.clone()));
        }
    }
    if issue.kind == IssueKind::Spelling && ui.button(text.visual(&text.ui("Add to Dictionary"))).clicked() {
        choice = Some(ContextChoice::AddToDictionary);
    }
    if ui.button(text.visual(&text.ui("Check Spelling…"))).clicked() {
        choice = Some(ContextChoice::Review);
    }
    ui.separator();
    choice
}
