//! [`HyperlinkDialog`] (Insert › Link, Insert Hyperlink): link to a web address, an email address or a place in the
//! document (a slide, a heading, a bookmark), the text to display, a ScreenTip, and the Highlight click option.

use craft_behavior::{Selection, keyed};
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, NoValue, Outcome, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing};
use craft_ui::dialog::{Dialog, DialogId, DialogOutput, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::display::Text;
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::Initial;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::text::TextField;
use egui::{Context, Response, Ui};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The dialog's words, English by default.
    pub struct HyperlinkWords {
        title: "hyperlink.title" = "Insert Hyperlink";
        edit_title: "hyperlink.editTitle" = "Edit Hyperlink";
        link_to: "hyperlink.linkTo" = "Link to:";
        web_page: "hyperlink.webPage" = "Existing File or Web Page";
        place: "hyperlink.place" = "Place in This Document";
        email: "hyperlink.email" = "E-mail Address";
        text_to_display: "hyperlink.textToDisplay" = "Text to display:";
        address: "hyperlink.address" = "Address:";
        email_address: "hyperlink.emailAddress" = "E-mail address:";
        subject: "hyperlink.subject" = "Subject:";
        select_place: "hyperlink.selectPlace" = "Select a place in this document:";
        top_of_document: "hyperlink.topOfDocument" = "Top of the Document";
        screen_tip: "hyperlink.screenTip" = "ScreenTip:";
        highlight_click: "hyperlink.highlightClick" = "Highlight click";
        remove: "hyperlink.remove" = "Remove Link";
        fields: "hyperlink.fields" = "Link";
    }
}

/// Where a link goes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkTarget {
    /// A web address or file (`https://…`).
    Url(String),
    /// An email address and an optional subject, written as a `mailto:` link by the app.
    #[non_exhaustive]
    Email {
        /// The email address.
        address: String,
        /// The subject line, if any.
        subject: Option<String>,
    },
    /// The top of the document.
    Top,
    /// A place in the document: the index into the places given to [`HyperlinkDialog::places`].
    Place(usize),
}

impl LinkTarget {
    /// A link to the email address `address`, with the subject line `subject` if any.
    pub fn email(address: impl Into<String>, subject: Option<String>) -> Self {
        LinkTarget::Email { address: address.into(), subject }
    }
}

/// The link the dialog edits.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Hyperlink {
    /// Where the link goes.
    pub target: LinkTarget,
    /// The tooltip shown over the link.
    pub tooltip: String,
    /// Whether a click on the link highlights it.
    pub highlight_click: bool,
}

impl Hyperlink {
    /// A link to `target` with no tooltip and no highlight on click.
    pub fn new(target: LinkTarget) -> Self {
        Hyperlink { target, tooltip: String::new(), highlight_click: false }
    }

    /// The tooltip shown over the link.
    #[must_use]
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = tooltip.into();
        self
    }
}

/// What the selection holds, which decides what OK does.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CurrentLink {
    /// Nothing linked: OK sets a new link (a caret at a word boundary inserts the display text).
    None,
    /// One link covers the selected text or every selected shape.
    One(Hyperlink),
    /// Several different links: OK replaces none of them until a new target is chosen.
    Mixed,
    /// A link this dialog doesn't edit (next slide, a program): kept unless a new target is chosen.
    Other,
}

/// OK's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HyperlinkAnswer {
    /// Write this link; `text` is the display text when the selection was a bare caret (insert it) or the user
    /// changed it.
    #[non_exhaustive]
    Set {
        /// The link to write.
        link: Hyperlink,
        /// The display text, when the selection was a bare caret or the user changed it.
        text: Option<String>,
    },
    /// Remove Link (only with a link to edit).
    Remove,
    /// OK changed nothing (a mixed or other link with no new target).
    Keep,
}

impl HyperlinkAnswer {
    /// Write `link`, with the display text `text` when it changed.
    pub fn set(link: Hyperlink, text: Option<String>) -> Self {
        HyperlinkAnswer::Set { link, text }
    }
}

/// What a hyperlink dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct HyperlinkOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// OK's answer or Remove Link, the pass the dialog closed with one of them.
    pub answer: Option<HyperlinkAnswer>,
}

impl Outcome for HyperlinkOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Which kind of target the dialog edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Url,
    Email,
    Place,
}

/// A row of the places list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Spot {
    Top,
    /// An index into the dialog's places.
    Place(usize),
}

/// The dialog's fields.
#[derive(Clone, Debug, PartialEq)]
struct Form {
    mode: Mode,
    url: String,
    email: String,
    subject: String,
    place: Selection<Spot>,
    tooltip: String,
    highlight_click: bool,
    display: String,
}

const NEW_ADDRESS: &str = "https://";

impl Form {
    /// The fields a new link starts with, and the ones `current` fills in.
    fn of(current: &CurrentLink, text: Option<&str>) -> Form {
        let mut form = Form {
            mode: Mode::Url,
            url: NEW_ADDRESS.to_owned(),
            email: String::new(),
            subject: String::new(),
            place: Selection::default(),
            tooltip: String::new(),
            highlight_click: false,
            display: text.unwrap_or_default().to_owned(),
        };
        match current {
            CurrentLink::None => {}
            CurrentLink::Mixed | CurrentLink::Other => form.url.clear(),
            CurrentLink::One(link) => {
                form.tooltip.clone_from(&link.tooltip);
                form.highlight_click = link.highlight_click;
                match &link.target {
                    LinkTarget::Url(url) => form.url.clone_from(url),
                    LinkTarget::Email { address, subject } => {
                        form.mode = Mode::Email;
                        form.email.clone_from(address);
                        form.subject = subject.clone().unwrap_or_default();
                    }
                    LinkTarget::Top => {
                        form.mode = Mode::Place;
                        form.place = Selection::single(Spot::Top);
                    }
                    LinkTarget::Place(place) => {
                        form.mode = Mode::Place;
                        form.place = Selection::single(Spot::Place(*place));
                    }
                }
            }
        }
        form
    }

    fn place(&self) -> Option<Spot> {
        self.place.lead
    }
}

/// What the app gave the dialog and the fields it started from.
#[derive(Clone, Debug, PartialEq)]
struct Given {
    current: CurrentLink,
    text: Option<String>,
    places: Vec<String>,
    highlight: bool,
    seed: Form,
}

impl Given {
    /// The target the fields name, if they name one.
    fn target(&self, form: &Form) -> Option<LinkTarget> {
        match form.mode {
            Mode::Url => Some(form.url.trim()).filter(|u| !u.is_empty() && *u != NEW_ADDRESS).map(|u| LinkTarget::Url(u.to_owned())),
            Mode::Email => Some(form.email.trim()).filter(|e| !e.is_empty()).map(|address| LinkTarget::Email {
                address: address.to_owned(),
                subject: Some(form.subject.trim()).filter(|s| !s.is_empty()).map(str::to_owned),
            }),
            Mode::Place => match form.place()? {
                Spot::Top => Some(LinkTarget::Top),
                Spot::Place(place) => (place < self.places.len()).then_some(LinkTarget::Place(place)),
            },
        }
    }

    /// Whether OK writes a link: always for a new link or one the dialog edits; for a link it holds as it is
    /// (several, or one it doesn't edit), only once a target differs from the seed.
    fn applies(&self, form: &Form) -> bool {
        if matches!(self.current, CurrentLink::None | CurrentLink::One(_)) {
            return true;
        }
        let seed = &self.seed;
        form.mode != seed.mode
            || match form.mode {
                Mode::Url => form.url != seed.url,
                Mode::Email => form.email != seed.email || form.subject != seed.subject,
                Mode::Place => form.place() != seed.place(),
            }
    }

    /// What OK answers; `None` while the fields name no target.
    fn answer(&self, form: &Form) -> Option<HyperlinkAnswer> {
        if !self.applies(form) {
            return Some(HyperlinkAnswer::Keep);
        }
        let link = Hyperlink { target: self.target(form)?, tooltip: form.tooltip.clone(), highlight_click: form.highlight_click };
        Some(HyperlinkAnswer::set(link, self.changed_text(form)))
    }

    /// The display text when the user typed or changed it.
    fn changed_text(&self, form: &Form) -> Option<String> {
        let original = self.text.as_deref()?;
        (form.display != original && !form.display.is_empty()).then(|| form.display.clone())
    }
}

/// The dialog's buttons: OK, Cancel, and Remove Link while a link is edited.
const OK: usize = 0;
const REMOVE: usize = 2;

/// Insert Hyperlink on the dialog stack.
///
/// - Link to: the three targets as a radio group at the left; a new link's address starts at `https://`. Address,
///   Email address, Subject, Text to display and ScreenTip are text fields.
/// - Place in This Document lists Top of the Document, then [`places`](Self::places), in a list (one Tab stop, the
///   list keys, type-ahead). Top is [`LinkTarget::Top`]; `places` holds only the named places.
/// - With [`CurrentLink::Mixed`] or [`CurrentLink::Other`] the fields start empty and OK answers
///   [`HyperlinkAnswer::Keep`] until a target differs from the seed.
/// - Keys: Enter is OK unless a field owns it, Escape is Cancel. Remove Link shows while a link is edited.
#[derive(Clone, Debug, PartialEq, craft_ui::contract::Pattern)]
pub struct HyperlinkDialog {
    given: Given,
    form: Form,
    open: bool,
}

impl HyperlinkDialog {
    /// The dialog over what the selection holds; `text` is the selected text shown in Text to display (`None` hides
    /// the field: a shape's click action).
    pub fn new(current: CurrentLink, text: Option<String>) -> Self {
        let form = Form::of(&current, text.as_deref());
        HyperlinkDialog { given: Given { current, text, places: Vec::new(), highlight: false, seed: form.clone() }, form, open: true }
    }

    /// The places in the document a link may go to, after Top of the Document (which the dialog always lists, as
    /// [`LinkTarget::Top`]).
    #[must_use]
    pub fn places(mut self, places: Vec<String>) -> Self {
        self.given.places = places;
        self
    }

    /// Shows the Highlight click option.
    #[must_use]
    pub fn highlight_click(mut self, on: bool) -> Self {
        self.given.highlight = on;
        self
    }

    /// Shows the dialog.
    pub fn show(&mut self, ui: &mut Ui) -> HyperlinkOutput {
        let HyperlinkDialog { given, form, open } = self;
        let env = Env::of(ui);
        let words = env.words::<HyperlinkWords>();
        let common = env.words::<CommonWords>();
        let editing = matches!(given.current, CurrentLink::One(_));
        let title = if editing { words.edit_title.clone() } else { words.title.clone() };
        let valid = std::cell::Cell::new(given.answer(form).is_some());
        let mut buttons =
            vec![FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
        if editing {
            buttons.push(FooterButton::new(words.remove.clone(), DialogRole::Destructive));
        }
        let dialog = Dialog::new(title).buttons(buttons).initial_focus(Initial::Default).opened(Bind::bound(open)).settle_buttons(|_, buttons| {
            if let Some(accept) = buttons.get_mut(OK) {
                accept.set_enabled(valid.get());
            }
        });
        let given_ref = &*given;
        let shown = dialog.show(ui, |ui, _| {
            body(ui, given_ref, form, &words);
            valid.set(given_ref.answer(form).is_some());
        });
        let DialogOutput { response, closed, picked, .. } = shown.output;
        let answer = match (closed, picked) {
            (Some(CloseReason::Activated | CloseReason::Enter), Some(REMOVE)) if editing => Some(HyperlinkAnswer::Remove),
            (Some(CloseReason::Activated | CloseReason::Enter), Some(OK)) => given.answer(form),
            _ => None,
        };
        HyperlinkOutput { response, closed, answer }
    }
}

fn body(ui: &mut Ui, given: &Given, form: &mut Form, words: &HyperlinkWords) {
    Row::new().show(ui, |ui, _| {
        RadioGroup::new(words.link_to.clone(), Bind::bound(&mut form.mode)).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Text::new(words.link_to.clone()).show(ui);
                Radio::new(Mode::Url).label(words.web_page.clone()).show(ui);
                Radio::new(Mode::Email).label(words.email.clone()).show(ui);
                Radio::new(Mode::Place).label(words.place.clone()).show(ui);
            });
        });
        Stack::new().show(ui, |ui, _| {
            FieldGroup::new(words.fields.clone()).show(ui, |ui, _| {
                if given.text.is_some() {
                    text_field(ui, "display", &words.text_to_display, &mut form.display, false);
                }
                match form.mode {
                    Mode::Url => text_field(ui, "url", &words.address, &mut form.url, true),
                    Mode::Email => {
                        text_field(ui, "email", &words.email_address, &mut form.email, true);
                        text_field(ui, "subject", &words.subject, &mut form.subject, false);
                    }
                    Mode::Place => place_list(ui, given, form, words),
                }
                text_field(ui, "tip", &words.screen_tip, &mut form.tooltip, false);
            });
            if given.highlight {
                Checkbox::new(Bind::bound(&mut form.highlight_click)).label(words.highlight_click.clone()).show(ui);
            }
        });
    });
}

/// A labelled text field; `default_stop` marks the field the dialog opens on: the address of the link's kind.
fn text_field(ui: &mut Ui, salt: &'static str, label: &Label, value: &mut String, default_stop: bool) {
    Field::new(label.clone()).id_salt(salt).show(ui, |ui, _| {
        let field = TextField::new(label.clone(), Bind::bound(value));
        if default_stop { field.default_stop() } else { field }.show(ui);
    });
}

fn place_list(ui: &mut Ui, given: &Given, form: &mut Form, words: &HyperlinkWords) {
    Text::new(words.select_place.clone()).show(ui);
    let spots: Vec<Spot> = std::iter::once(Spot::Top).chain((0..given.places.len()).map(Spot::Place)).collect();
    let source = keyed(&spots, |spot: &Spot| *spot);
    List::new(words.select_place.clone())
        .select(SelectMode::Single)
        .sizing(Sizing::Content)
        .natural_height(true)
        .selection(Bind::bound(&mut form.place))
        .show_items(ui, &source, &mut |spot, _| match spot {
            Spot::Top => ListRow::new(words.top_of_document.clone()),
            Spot::Place(place) => ListRow::new(given.places[*place].clone()),
        });
}

impl StackDialog for HyperlinkDialog {
    type Result = HyperlinkAnswer;

    fn id(&self) -> DialogId {
        dialog_id("hyperlink")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<HyperlinkAnswer>> {
        let shown = HyperlinkDialog::show(self, ui);
        shown.closed?;
        Some(shown.answer.map_or(DialogResult::Cancel, DialogResult::Ok))
    }

    fn confirm(&mut self, _: &Context) -> Option<HyperlinkAnswer> {
        self.given.answer(&self.form)
    }
}
