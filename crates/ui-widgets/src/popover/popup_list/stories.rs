//! Stories of `PopupList`.

use super::{PopupList, PopupListKind};
use crate::buttons::Button;
use crate::contract::overlay::Anchor;
use crate::contract::{Bind, Props as _, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use crate::fonts::FontRef;
use crate::layout_components::Stack;
use crate::list::{DocumentRows, List, ListRow};
use crate::primitives::HAlign;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::{DisplayOrder, ItemSource};
use egui::Ui;

/// A list of names keyed by position.
struct Names<'a>(&'a [&'a str]);

impl DisplayOrder<usize> for Names<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.0.get(at).map(|name| std::borrow::Cow::Borrowed(*name))
    }
}

impl ItemSource<usize> for Names<'_> {}

/// The opener sits at the start edge of a row as wide as the room, so the popup's place mirrors with the direction.
#[story(PopupList<'static>, covers = [Default], drive = Drive::Never)]
fn popup_list(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let names = ["Round", "Square", "Chisel", "Fan"];
    let shown = Stack::new().fill().show(ui, |ui, _| {
        let opener = Button::new("Brush tip").show(ui);
        let list = opener.id.with("tips");
        PopupList::new("Brush tips", data.bind("open", true))
            .opened_by(&opener)
            .list(list)
            .show(ui, |ui, _| {
                List::<usize>::options("Brush tips").id(list).show_items(ui, &Names(&names), &mut |key, _| ListRow::new(names[*key]).option());
            })
            .output
            .response
            .into()
    });
    match shown.inner {
        Some(output) => output,
        None => shown.output.into(),
    }
}

/// The point size of a large document field's text.
const LARGE_DOCUMENT_SIZE: f32 = 24.0;
/// The point size of a document field's usual text.
const DOCUMENT_SIZE: f32 = 12.0;

/// A popup list of document rows hanging from a field inside a page: the page is the opener, the field's rect the
/// anchor, as a form field drawn on a document is.
fn document_popup(ui: &mut Ui, data: &mut StoryData, rows: DocumentRows) -> StoryOutput {
    let names = ["Round", "Square", "Chisel", "Fan"];
    let page = Stack::new().fill().show(ui, |ui, _| {
        let opener = Button::new("Brush tip").show(ui);
        if clicked(&opener) {
            *data.value("open", false) = true;
        }
        opener.rect
    });
    let field = page.inner.unwrap_or(page.output.rect);
    PopupList::new("Brush tips", Bind::bound(data.value("open", false)))
        .kind(PopupListKind::Document)
        .opened_by(&page.output)
        .overlay(|overlay| overlay.opener_anchor(Anchor::Rect(field)))
        .show(ui, |ui, _| {
            List::<usize>::options("Brush tips")
                .document_rows(rows.clone())
                .show_items(ui, &Names(&names), &mut |key, _| ListRow::new(names[*key]).option());
        })
        .output
        .response
        .into()
}

/// Rows in a large document font, at the start edge.
#[story(PopupList<'static>, covers = [], drive = Drive::Never)]
fn popup_list_document_large(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    document_popup(ui, data, DocumentRows::new(FontRef::proportional(), LARGE_DOCUMENT_SIZE, HAlign::Start))
}

/// Rows in the document's font, centred as the field centres its value.
#[story(PopupList<'static>, covers = [], drive = Drive::Never)]
fn popup_list_document_centred(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    document_popup(ui, data, DocumentRows::new(FontRef::proportional(), DOCUMENT_SIZE, HAlign::Center))
}
