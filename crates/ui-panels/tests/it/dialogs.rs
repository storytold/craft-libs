//! The dialog host and stack.

use craft_ui::dialog::{Alert, DialogHost, DialogId, DialogResult, DialogStack, HostedAlert, StackDialog, dialog_id};
use craft_ui::testing::{Frames, trial};
use egui::Ui;

/// A dialog with no content: it records whether it was the top of the stack when shown and finishes with
/// `finish` once set.
struct Probe<const N: usize> {
    key: &'static str,
    top: bool,
    finish: Option<DialogResult<u32>>,
}

impl<const N: usize> Probe<N> {
    fn new(key: &'static str) -> Self {
        Self { key, top: false, finish: None }
    }
}

impl<const N: usize> StackDialog for Probe<N> {
    type Result = u32;

    fn id(&self) -> DialogId {
        dialog_id(self.key)
    }

    fn show(&mut self, _: &mut Ui, stack: &DialogStack) -> Option<DialogResult<u32>> {
        self.top = stack.is_top(StackDialog::id(self));
        self.finish.take()
    }
}

fn run(host: &mut DialogHost, frames: &mut Frames) {
    frames.frame(Vec::new(), |ui| host.run(ui));
}

#[trial]
fn cancelling_a_parent_cancels_its_children() {
    let mut frames = Frames::new();
    let mut host = DialogHost::new();
    let parent = host.open(Probe::<1>::new("parent"));
    host.open_child(Probe::<2>::new("child"), parent);
    run(&mut host, &mut frames);

    host.get_mut::<Probe<2>>().unwrap().finish = Some(DialogResult::Cancel);
    run(&mut host, &mut frames);
    assert!(host.is_open::<Probe<1>>() && !host.is_open::<Probe<2>>());
    assert_eq!(host.take::<Probe<2>>(), Some(DialogResult::Cancel));

    host.open_child(Probe::<2>::new("child"), parent);
    assert!(host.cancel::<Probe<1>>(&frames.ctx));
    assert!(host.is_empty() && host.stack().is_empty());
    assert_eq!(host.take::<Probe<1>>(), Some(DialogResult::Cancel));
    assert_eq!(host.take::<Probe<2>>(), Some(DialogResult::Cancel));
}

#[trial]
fn a_hosted_alert_answers_ok_however_it_closes_and_opens_again() {
    let host = std::rc::Rc::new(std::cell::RefCell::new(DialogHost::new()));
    let shown = std::rc::Rc::clone(&host);
    let mut fixture = craft_ui::testing::Fixture::new(move |ui| shown.borrow_mut().run(ui));
    for key in [egui::Key::Enter, egui::Key::Escape] {
        host.borrow_mut().open(HostedAlert::new(Alert::new("Could not open", "The file is damaged.")));
        fixture.idle(6);
        assert!(fixture.by_label("Could not open").is_some(), "the alert shows on the host before {key:?}");
        fixture.key(key, egui::Modifiers::NONE).idle(4);
        assert!(host.borrow().is_empty(), "{key:?} closes the alert");
        assert_eq!(host.borrow_mut().take::<HostedAlert>(), Some(DialogResult::Ok(())), "{key:?} answers OK");
    }
}
