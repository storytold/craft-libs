//! The panel and the right-click section. Looks follow the host's theme through [`Colors`].
use std::borrow::Cow;

use egui::{Align, Align2, Color32, FontId, Layout, RichText, Sense, Stroke, TextStyle, TextWrapMode, Ui, vec2};

use crate::review::{CHANGED, NO_ISSUE, NOT_SPELLING, OTHER_DOCUMENT, SAME_REPLACEMENT};
use crate::{Action, Host, Issue, IssueKind, Review};

/// The application's translations.
pub trait Text {
    /// A string from [`STRINGS`], by its English source text. Anything else, such as a host's
    /// error message, comes back unchanged.
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
    "{count} left",
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
    /// The suggestion chosen in the list.
    pub selected: Color32,
    /// The main action's button, and its text.
    pub accent: Color32,
    pub on_accent: Color32,
    /// The underline of a misspelled word.
    pub spelling: Color32,
    /// The underline of a preferred form.
    pub grammar: Color32,
    pub error: Color32,
}

fn label(ui: &mut Ui, text: &dyn Text, source: &str, size: f32, color: Color32) -> egui::Response {
    ui.label(RichText::new(text.visual(&text.ui(source))).size(size).color(color))
}

fn tr(text: &dyn Text, source: &str) -> String {
    text.visual(&text.ui(source))
}

/// Tall enough for a label in Khmer or Arabic.
const BUTTON: f32 = 24.0;
const BUTTON_GAP: f32 = 6.0;
/// The least of the review left to scroll above actions kept in view.
const MIN_BODY: f32 = 48.0;
/// A row of the suggestion list.
const ROW: f32 = 25.0;
/// The suggestion list's least height, when the panel is too short to give it more.
const MIN_LIST: f32 = 2.0 * ROW;
/// Below actions kept in view, clear of the host's frame.
const BOTTOM_MARGIN: f32 = 2.0;

fn button(ui: &mut Ui, label: &str, width: f32) -> bool {
    ui.add(egui::Button::new(label).min_size(vec2(width, BUTTON)).wrap_mode(TextWrapMode::Truncate)).clicked()
}

/// Across the panel, its label centered.
fn wide_button(ui: &mut Ui, label: &str) -> bool {
    let width = ui.available_width();
    ui.with_layout(Layout::top_down(Align::Center), |ui| button(ui, label, width)).inner
}

/// The one action this state is for, across the panel.
fn primary(ui: &mut Ui, label: &str, enabled: bool, colors: &Colors) -> bool {
    let button = egui::Button::new(RichText::new(label).color(colors.on_accent))
        .fill(colors.accent)
        .min_size(vec2(ui.available_width(), BUTTON))
        .wrap_mode(TextWrapMode::Truncate);
    ui.with_layout(Layout::top_down(Align::Center), |ui| ui.add_enabled(enabled, button).clicked()).inner
}

fn context(ui: &mut Ui, review: &Review, issue: &Issue, colors: &Colors) {
    let format = egui::TextFormat { font_id: FontId::proportional(13.0), color: colors.text, ..Default::default() };
    let mark = if issue.kind == IssueKind::Spelling { colors.spelling } else { colors.grammar };
    let mut job = egui::text::LayoutJob::default();
    job.append(&review.before, 0.0, format.clone());
    job.append(&issue.word, 0.0, egui::TextFormat { underline: Stroke::new(1.5, mark), background: mark.gamma_multiply(0.22), ..format.clone() });
    job.append(&review.after, 0.0, format);
    let width = ui.available_width();
    egui::Frame::NONE.fill(colors.field).stroke(Stroke::new(1.0, colors.field_border)).corner_radius(2.0).inner_margin(8).show(ui, |ui| {
        ui.set_min_width((width - 18.0).max(0.0));
        ui.add(egui::Label::new(job).wrap());
    });
}

/// A full-width row of the suggestion list.
fn suggestion_row(ui: &mut Ui, suggestion: &str, selected: bool, colors: &Colors) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, suggestion));
    if selected {
        ui.painter().rect_filled(rect, 0.0, colors.selected);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, colors.hover);
    }
    ui.painter().with_clip_rect(rect.shrink2(vec2(8.0, 0.0))).text(
        rect.left_center() + vec2(8.0, 0.0),
        Align2::LEFT_CENTER,
        suggestion,
        FontId::proportional(13.0),
        colors.text,
    );
    response
}

/// What to do after the panel has been drawn, so that nothing changes under its borrows.
enum After {
    Start,
    Act(Action),
}

/// Draws the review. Starts one from its own button if there is none yet. The current issue's
/// actions keep their place at the bottom, so that they stay in view when a short panel has to
/// scroll the rest, and don't move from one issue to the next under a pointer going through them.
/// A panel too short for that scrolls the actions with the rest. Give it the height to fill, not a
/// scrolling parent.
pub fn panel(ui: &mut Ui, review: &mut Review, host: &mut dyn Host, text: &dyn Text, colors: &Colors) {
    let actionable = actionable_issue(review, &*host);
    let height = ui.available_height();
    let reserve = actionable.as_ref().map_or(0.0, |_| BUTTON_GAP + actions_height(ui, text) + BOTTOM_MARGIN);
    let pinned = actionable.is_some() && height.is_finite() && height >= reserve + MIN_BODY;
    let scroll = if pinned {
        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(height - reserve).min_scrolled_height(height - reserve)
    } else {
        egui::ScrollArea::vertical().auto_shrink([false, true])
    };
    let mut after = scroll
        .show(ui, |ui| {
            let after = body(ui, review, &*host, text, colors);
            match &actionable {
                Some(issue) if !pinned => {
                    ui.add_space(BUTTON_GAP);
                    actions(ui, review, issue, text, colors).or(after)
                }
                _ => after,
            }
        })
        .inner;
    if pinned && let Some(issue) = &actionable {
        ui.add_space(BUTTON_GAP);
        after = actions(ui, review, issue, text, colors).or(after);
        ui.add_space(BOTTOM_MARGIN);
    }
    match after {
        Some(After::Start) => review.start(host),
        Some(After::Act(action)) => {
            // A failure stays in the review for the next frame.
            let _ = review.act(host, action);
        }
        None => {}
    }
}

/// The current issue, if the panel can act on it.
fn actionable_issue(review: &Review, host: &dyn Host) -> Option<Issue> {
    if !review.started() || review.other_document(host) || must_check_again(review, host) {
        return None;
    }
    review.current().cloned()
}

/// The text under the current issue changed, or a fresh check no longer finds the issue.
fn must_check_again(review: &Review, host: &dyn Host) -> bool {
    review.stale(host) || (review.current().is_some() && review.message() == CHANGED)
}

fn body(ui: &mut Ui, review: &mut Review, host: &dyn Host, text: &dyn Text, colors: &Colors) -> Option<After> {
    if !review.started() {
        label(ui, text, "Check the spelling of the whole document.", 12.0, colors.text);
        ui.add_space(8.0);
        return primary(ui, &tr(text, "Check Spelling"), true, colors).then_some(After::Start);
    }
    if review.other_document(host) {
        label(ui, text, OTHER_DOCUMENT, 12.0, colors.text);
        ui.add_space(8.0);
        return primary(ui, &tr(text, "Check This Document"), true, colors).then_some(After::Start);
    }
    if let Some(notice) = &review.notice {
        ui.label(RichText::new(notice).size(11.0).color(colors.dim));
        ui.add_space(8.0);
    }
    if !review.message().is_empty() && !must_check_again(review, host) {
        label(ui, text, review.message(), 12.0, colors.error);
        ui.add_space(8.0);
    }
    let mut after = None;
    match review.current().cloned() {
        Some(issue) => after = current_issue(ui, review, &issue, host, text, colors),
        None => {
            if review.message().is_empty() {
                let done = if review.ignored_any() { "The spelling check is complete. Some issues were ignored." } else { "No issues found." };
                label(ui, text, done, 12.0, colors.text);
                ui.add_space(8.0);
            }
            if wide_button(ui, &tr(text, "Check Again")) {
                after = Some(After::Act(Action::Check));
            }
        }
    }
    after
}

fn current_issue(ui: &mut Ui, review: &mut Review, issue: &Issue, host: &dyn Host, text: &dyn Text, colors: &Colors) -> Option<After> {
    let spelling = issue.kind == IssueKind::Spelling;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr(text, if spelling { "Spelling" } else { "Preferred Form" })).size(14.0).strong().color(colors.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let left = text.ui("{count} left").replace("{count}", &review.remaining().to_string());
            ui.label(RichText::new(text.visual(&left)).size(11.0).color(colors.dim));
        });
    });
    if !issue.message.is_empty() {
        ui.label(RichText::new(&issue.message).size(11.0).color(colors.dim));
    }
    ui.add_space(8.0);
    context(ui, review, issue, colors);
    ui.add_space(12.0);
    if must_check_again(review, host) {
        label(ui, text, CHANGED, 12.0, colors.text);
        ui.add_space(8.0);
        return primary(ui, &tr(text, "Check Again"), true, colors).then_some(After::Act(Action::Check));
    }
    let mut after = None;
    let name = label(ui, text, "Change to:", 11.0, colors.dim);
    let field =
        egui::TextEdit::singleline(&mut review.replacement).font(FontId::proportional(13.0)).margin(vec2(8.0, 5.0)).desired_width(f32::INFINITY);
    let field = ui.add(field).labelled_by(name.id);
    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && can_change(review, issue) {
        after = Some(After::Act(Action::Change(None)));
    }
    ui.add_space(8.0);
    label(ui, text, "Suggestions", 11.0, colors.dim);
    // Down to the actions, scrolling inside itself: nothing between the list and them.
    let room = ui.available_height();
    let natural = ROW * issue.suggestions.len().max(1) as f32;
    // Short of the very bottom, or the area scrolling this would clip the list's own border.
    let height = if room.is_finite() { (room - 2.0).max(MIN_LIST) } else { natural };
    egui::Frame::NONE.fill(colors.field).stroke(Stroke::new(1.0, colors.field_border)).corner_radius(2.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical()
            .id_salt(("suggestions", issue.story, issue.start, &issue.word))
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_height(height)
            .show(ui, |ui| {
                if issue.suggestions.is_empty() {
                    egui::Frame::NONE.inner_margin(8).show(ui, |ui| {
                        ui.label(RichText::new(tr(text, "(no suggestions)")).italics().color(colors.text));
                    });
                }
                for suggestion in &issue.suggestions {
                    let row = suggestion_row(ui, suggestion, *suggestion == review.replacement, colors);
                    if row.clicked() {
                        review.replacement.clone_from(suggestion);
                    }
                    if row.double_clicked() {
                        after = Some(After::Act(Action::Change(Some(suggestion.clone()))));
                    }
                }
            });
    });
    after
}

fn can_change(review: &Review, issue: &Issue) -> bool {
    !review.replacement.is_empty() && review.replacement != issue.word
}

/// Ignore Once and Ignore All don't fit side by side, in a narrow panel or a long translation.
fn ignores_stacked(ui: &Ui, text: &dyn Text) -> bool {
    let font = TextStyle::Button.resolve(ui.style());
    let pad = 2.0 * ui.spacing().button_padding.x + 4.0;
    let width = |source: &str| ui.fonts_mut(|fonts| fonts.layout_no_wrap(tr(text, source), font.clone(), Color32::WHITE).size().x) + pad;
    (ui.available_width() - BUTTON_GAP) / 2.0 < width("Ignore Once").max(width("Ignore All"))
}

/// The same for either kind of issue, so that no button lands under the pointer on another's place.
fn actions_height(ui: &Ui, text: &dyn Text) -> f32 {
    let rows: u8 = if ignores_stacked(ui, text) { 4 } else { 3 };
    f32::from(rows) * BUTTON + f32::from(rows - 1) * BUTTON_GAP
}

/// Change, then what else can be done with the issue. A preferred form's word is known, so it
/// can't be ignored for good or join the dictionary, and those rows stay empty.
fn actions(ui: &mut Ui, review: &Review, issue: &Issue, text: &dyn Text, colors: &Colors) -> Option<After> {
    let mut after = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = vec2(BUTTON_GAP, BUTTON_GAP);
        if primary(ui, &tr(text, "Change"), can_change(review, issue), colors) {
            after = Some(After::Act(Action::Change(None)));
        }
        if issue.kind != IssueKind::Spelling {
            if wide_button(ui, &tr(text, "Ignore Once")) {
                after = Some(After::Act(Action::IgnoreOnce));
            }
            ui.add_space(actions_height(ui, text) - 2.0 * BUTTON - 2.0 * BUTTON_GAP);
            return;
        }
        if ignores_stacked(ui, text) {
            if wide_button(ui, &tr(text, "Ignore Once")) {
                after = Some(After::Act(Action::IgnoreOnce));
            }
            if wide_button(ui, &tr(text, "Ignore All")) {
                after = Some(After::Act(Action::IgnoreAll));
            }
        } else {
            ui.horizontal(|ui| {
                let half = (ui.available_width() - BUTTON_GAP) / 2.0;
                if button(ui, &tr(text, "Ignore Once"), half) {
                    after = Some(After::Act(Action::IgnoreOnce));
                }
                if button(ui, &tr(text, "Ignore All"), half) {
                    after = Some(After::Act(Action::IgnoreAll));
                }
            });
        }
        if wide_button(ui, &tr(text, "Add to Dictionary")) {
            after = Some(After::Act(Action::AddToDictionary));
        }
    });
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
