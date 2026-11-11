//! Stories of the value controls: `Disclosure`.

use super::disclosure::Disclosure;
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use egui::Ui;

#[story(Disclosure<'static>, covers = [Default])]
fn disclosure(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let out = Disclosure::new(Bind::bound(data.value("open", false))).label("Details").show(ui);
    Disclosure::new(Bind::bound(&mut false)).label("Details, disabled").enabled(false).show(ui);
    out.into()
}

#[story(Disclosure<'static>, covers = [Mixed], drive = Drive::Never)]
fn disclosure_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Disclosure::new(Bind::bound_mixed(data.value("mixed", Mixed::Mixed))).label("Details").show(ui).into()
}

#[story(Disclosure<'static>, covers = [Invalid], drive = Drive::Never)]
fn disclosure_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_open = |open: &bool| if *open { Validity::Refuse(Message::new("Nothing to show")) } else { Validity::Ok };
    Disclosure::new(Bind::bound(data.value("open", true)).check(refuse_open)).label("Details").show(ui).into()
}
