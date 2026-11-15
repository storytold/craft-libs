//! Trials of the full-window dialog shell's keys.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::FullWindowDialog;
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::runtime::focus::Initial;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::input::key_press;
use crate::testing::{Frames, trial};
use egui::{Key, Modifiers, Ui};
use std::cell::Cell;

/// A full-window dialog has no overlay layer; its footer letters follow its own focus scope.
#[trial]
fn a_footer_letter_activates_its_button_in_a_full_window_dialog() {
    let picked = Cell::new(None);
    let mut frames = Frames::new();
    let mut ui = |ui: &mut Ui| {
        let (mut open, mut snap) = (true, false);
        let shown = FullWindowDialog::new("Preferences")
            .opened(Bind::bound(&mut open))
            .initial_focus(Initial::First)
            .buttons([FooterButton::new("Reset", DialogRole::Destructive).letter('r'), FooterButton::new("Close", DialogRole::Cancel)])
            .show(ui, |ui, _| {
                Checkbox::new(Bind::bound(&mut snap)).label("Snap").show(ui);
            });
        if shown.output.picked.is_some() {
            picked.set(shown.output.picked);
        }
    };
    frames.idle(30, &mut ui);
    frames.frame(vec![key_press(Key::R, Modifiers::NONE)], &mut ui);
    frames.idle(2, &mut ui);
    assert_eq!(picked.get(), Some(0), "the letter picks its button");
}

/// The sample workspace on a dialog host, run once per frame.
mod hosting {
    use super::super::HostedWorkspace;
    use super::super::stories::{BrushChanged, BrushWorkspace, Flat};
    use crate::contract::Role;
    use crate::dialog::{DialogHost, DialogResult, FieldRefusal, HeldOk, StackDialog};
    use crate::runtime::env::Label;
    use crate::testing::access::AccessTree;
    use crate::testing::input::key_press;
    use crate::testing::pointer::{click, moved, press, release};
    use crate::testing::{Frames, trial};
    use egui::{Key, Modifiers, Pos2, Rect, Ui};
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;

    type Hosted = HostedWorkspace<BrushWorkspace>;

    fn workspace() -> Hosted {
        HostedWorkspace::new(BrushWorkspace { brush: 10.0, ..BrushWorkspace::default() }, Arc::new(Flat))
    }

    /// The bounds of the first node with `role`, and `name` when given.
    fn bounds(tree: &AccessTree, role: Role, name: Option<&str>) -> Option<Rect> {
        let key = tree.keys().into_iter().find(|key| tree.role_of(*key) == Some(role) && name.is_none_or(|name| tree.name_of(*key) == Some(name)));
        key.and_then(|key| tree.bounds_of(key))
    }

    /// The centre of the first node with `role`, and `name` when given.
    fn centre(tree: &AccessTree, role: Role, name: Option<&str>) -> Pos2 {
        bounds(tree, role, name).unwrap_or_else(|| panic!("no {role:?} named {name:?}")).center()
    }

    /// Whether the first button named `name` in `tree` is disabled.
    fn disabled(tree: &AccessTree, name: &str) -> bool {
        let key = tree.keys().into_iter().find(|key| tree.role_of(*key) == Some(Role::BUTTON) && tree.name_of(*key) == Some(name));
        tree.is_disabled_key(key.unwrap_or_else(|| panic!("no button named {name:?}")))
    }

    /// The bounds of the workspace dialog in the last frame's tree.
    fn dialog_bounds(frames: &mut Frames) -> Option<Rect> {
        bounds(&frames.tree(), Role::DIALOG, Some("Liquify"))
    }

    #[trial]
    fn a_hosted_workspace_opens_full_window_and_returns_its_result() {
        let host = RefCell::new(DialogHost::new());
        let screen = Cell::new(Rect::NOTHING);
        let mut frames = Frames::new();
        let mut ui = |ui: &mut Ui| {
            screen.set(ui.ctx().content_rect());
            host.borrow_mut().run(ui);
        };
        host.borrow_mut().open(workspace());
        frames.idle(30, &mut ui);
        let tree = frames.tree();
        assert_eq!(bounds(&tree, Role::DIALOG, Some("Liquify")), Some(screen.get()), "the workspace fills the window");

        let slider = centre(&tree, Role::SLIDER, None);
        click(&mut frames, slider, &mut ui);
        let events = host.borrow_mut().get_mut::<Hosted>().expect("still open").take_events();
        let Some(&BrushChanged(size)) = events.last() else { panic!("a click on the slider reports the brush change") };
        assert_ne!(size, 10.0);
        assert!(host.borrow_mut().get_mut::<Hosted>().is_some_and(|hosted| hosted.take_events().is_empty()), "taken events are gone");

        let ok = centre(&tree, Role::BUTTON, Some("OK"));
        click(&mut frames, ok, &mut ui);
        assert_eq!(host.borrow_mut().take::<Hosted>(), Some(DialogResult::Ok(size)), "OK answers the brush size");
        assert!(dialog_bounds(&mut frames).is_none(), "the workspace closed");

        for _ in 0..2 {
            host.borrow_mut().open(workspace());
            frames.idle(10, &mut ui);
            assert!(dialog_bounds(&mut frames).is_some(), "a reopened workspace shows");
            frames.frame(vec![key_press(Key::Escape, Modifiers::NONE)], &mut ui);
            frames.idle(2, &mut ui);
            assert_eq!(host.borrow_mut().take::<Hosted>(), Some(DialogResult::Cancel), "Escape cancels");
        }

        host.borrow_mut().open(workspace());
        frames.idle(10, &mut ui);
        let cancel = centre(&frames.tree(), Role::BUTTON, Some("Cancel"));
        click(&mut frames, cancel, &mut ui);
        assert_eq!(host.borrow_mut().take::<Hosted>(), Some(DialogResult::Cancel), "Cancel cancels");
        assert!(host.borrow().stack().is_empty(), "the cancelled workspace leaves the stack");

        // The automation surface reaches the workspace through the host.
        let ctx = frames.ctx.clone();
        let id = host.borrow_mut().open(workspace());
        frames.idle(10, &mut ui);
        assert_eq!(host.borrow_mut().set_json_field(&ctx, id, "brush", json!(42.0)), Ok(()), "the workspace takes its brush");
        frames.idle(2, &mut ui);
        let events = host.borrow_mut().get_mut::<Hosted>().expect("still open").take_events();
        assert_eq!(events, vec![BrushChanged(42.0)], "the next pass reports the automation change once");
        assert_eq!(
            host.borrow_mut().set_json_field(&ctx, id, "size", json!(1.0)),
            Err(FieldRefusal::Unknown.message("size")),
            "an unknown field is refused"
        );
        let fields = host.borrow().open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields);
        assert_eq!(fields, Some(json!({ "brush": 42.0 })), "the fields list the new brush");
        assert!(host.borrow_mut().confirm::<Hosted>(&ctx), "automation answers OK");
        assert_eq!(host.borrow_mut().take::<Hosted>(), Some(DialogResult::Ok(42.0)), "OK answers the brush automation set");
    }

    #[trial]
    fn a_held_ok_files_the_result_until_the_app_answers() {
        let host = RefCell::new(DialogHost::new());
        let rerun = Cell::new(false);
        let mut frames = Frames::new();
        let mut ui = |ui: &mut Ui| {
            host.borrow_mut().run(ui);
            if rerun.get() && ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("the OK frame runs twice");
            }
        };
        let take_ok = |host: &RefCell<DialogHost>| host.borrow_mut().get_mut::<Hosted>().expect("open").take_ok();
        host.borrow_mut().open(workspace().held_ok());
        frames.idle(30, &mut ui);
        let ok = centre(&frames.tree(), Role::BUTTON, Some("OK"));

        // The release runs two passes; only the first has the click.
        frames.frame(vec![moved(ok)], &mut ui);
        frames.frame(vec![press(ok)], &mut ui);
        rerun.set(true);
        let passes = frames.frame(vec![release(ok)], &mut ui);
        rerun.set(false);
        frames.idle(2, &mut ui);
        assert_eq!(passes, 2, "the OK frame ran twice");
        assert_eq!(take_ok(&host), Some(10.0), "OK files its result");
        assert_eq!(take_ok(&host), None, "the filed OK is taken once");
        // `Frames::tree` takes the last frame's tree, so both reads share one.
        let tree = frames.tree();
        assert!(bounds(&tree, Role::DIALOG, Some("Liquify")).is_some(), "the workspace stays shown while the app checks");
        assert_eq!(host.borrow_mut().take::<Hosted>(), None, "a held OK does not finish the workspace");
        assert!(disabled(&tree, "OK"), "OK is disabled while the answer waits");
        click(&mut frames, ok, &mut ui);
        assert_eq!(take_ok(&host), None, "a click on the waiting OK files nothing");

        let reason = Label::from("Nothing to fill");
        host.borrow_mut().get_mut::<Hosted>().expect("open").answer_ok(Err(reason.clone()));
        frames.idle(2, &mut ui);
        assert_eq!(
            host.borrow().get::<Hosted>().and_then(|hosted| hosted.workspace().failed.clone()),
            Some(reason),
            "the refusal reaches the workspace"
        );
        assert!(!disabled(&frames.tree(), "OK"), "a refused OK is enabled again");
        click(&mut frames, ok, &mut ui);
        assert_eq!(take_ok(&host), Some(10.0), "a click on the enabled OK files the result again");

        host.borrow_mut().get_mut::<Hosted>().expect("open").answer_ok(Err(Label::from("Still nothing")));
        let id = host.borrow().get::<Hosted>().map(StackDialog::id).expect("open");
        assert_eq!(host.borrow().held_state(id), Some(HeldOk::Refused(Label::from("Still nothing"))), "the refusal is the held OK's state");
        let ctx = frames.ctx.clone();
        assert!(!host.borrow_mut().confirm::<Hosted>(&ctx), "automation's OK on a held workspace does not close it");
        assert_eq!(host.borrow().held_state(id), Some(HeldOk::Waiting), "automation's OK waits for the app's answer");
        assert_eq!(take_ok(&host), Some(10.0), "automation's OK files the result");

        let revision = || host.borrow().get::<Hosted>().map(|hosted| hosted.workspace().revision).expect("open");
        assert_eq!(revision(), 0);
        host.borrow_mut().get_mut::<Hosted>().expect("open").set_source(Arc::new(Flat));
        frames.idle(2, &mut ui);
        assert_eq!(revision(), 1, "a new source moves the workspace's revision");

        host.borrow_mut().get_mut::<Hosted>().expect("open").answer_ok(Ok(()));
        assert_eq!(host.borrow().held_state(id), Some(HeldOk::Accepted));
        assert!(host.borrow_mut().confirm::<Hosted>(&ctx), "automation's OK finishes an accepted held OK at once");
        assert_eq!(host.borrow_mut().take::<Hosted>(), Some(DialogResult::Ok(10.0)), "the accepted OK finishes with its result");
        frames.idle(2, &mut ui);
        assert!(dialog_bounds(&mut frames).is_none(), "the workspace closed");
    }

    #[trial]
    fn view_commands_fire_inside_a_hosted_workspace() {
        let host = RefCell::new(DialogHost::new());
        let mut frames = Frames::new();
        let mut ui = |ui: &mut Ui| host.borrow_mut().run(ui);
        host.borrow_mut().open(workspace());
        frames.idle(30, &mut ui);
        let zoom = || host.borrow().get::<Hosted>().map(|hosted| hosted.workspace().view.zoom_percent()).expect("open");
        let fitted = zoom();
        frames.frame(vec![key_press(Key::Equals, Modifiers::COMMAND)], &mut ui);
        frames.idle(2, &mut ui);
        assert!(zoom() > fitted, "Cmd+= zooms the workspace's view in: {fitted} -> {}", zoom());
    }
}
