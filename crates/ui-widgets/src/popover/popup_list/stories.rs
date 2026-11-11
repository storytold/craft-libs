//! Stories of `PopupList`.

use super::PopupList;
use crate::buttons::Button;
use crate::contract::{Bind, Show as _, ShowCollection as _, ShowContainer as _};
use crate::layout_components::Stack;
use crate::list::{List, ListRow};
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
        PopupList::new("Brush tips", Bind::bound(data.value("open", true)))
            .opened_by(&opener)
            .show(ui, |ui, _| {
                List::<usize>::options("Brush tips").show_items(ui, &Names(&names), &mut |key, _| ListRow::new(names[*key]).option());
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
