//! A dialog with `outer_gesture`: the edits of its fields reach the app's undo history as one step, which OK keeps and
//! Cancel or Escape takes back.

use craft_behavior::Mixed;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::edit::{GestureHost, StepName, UndoMarks};
use craft_ui::runtime::frame::{OuterEdit, take_output};
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::testing::{Fixture, trial};
use egui::{Context, Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

const TITLE: &str = "Grid Options";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Values {
    snap: bool,
    grid: bool,
}

/// A document with the history of an app: every applied edit records a step, and the history can fold or take back steps.
#[derive(Default)]
struct Doc {
    values: Values,
    steps: Vec<(Values, Option<String>)>,
}

impl Doc {
    fn set(&mut self, values: Values) {
        self.steps.push((self.values, None));
        self.values = values;
    }
}

impl UndoMarks for Doc {
    type Mark = usize;

    fn undo_mark(&self) -> usize {
        self.steps.len()
    }

    fn revert_to(&mut self, mark: usize) {
        if let Some((values, _)) = self.steps.get(mark) {
            self.values = *values;
        }
        self.steps.truncate(mark);
    }

    fn squash_since(&mut self, mark: usize, name: &StepName) {
        if self.steps.len() > mark {
            self.steps.truncate(mark + 1);
            self.steps[mark].1 = Some(name.label.to_string());
        }
    }
}

/// The app: its document and gesture host, the outer steps it read from the frame output, the dialog's open state and
/// the values its fields showed last.
#[derive(Default)]
struct App {
    doc: Doc,
    host: GestureHost<Doc>,
    outer: Vec<OuterEdit>,
    open: bool,
    shown: Vec<Option<Mixed<Arc<bool>>>>,
}

impl App {
    /// The app's end of frame: the frame output goes to its gesture host.
    fn drain(&mut self, ctx: &Context) {
        let output = take_output(ctx);
        self.outer.extend(output.outer_edits.iter().cloned());
        self.host.handle_output(&output, &mut self.doc);
    }
}

/// An open dialog with two check boxes over the document, whose events go to the app's gesture host.
fn dialog() -> (Fixture, Rc<RefCell<App>>) {
    let app = Rc::new(RefCell::new(App { open: true, ..App::default() }));
    let state = app.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut app = state.borrow_mut();
        app.drain(ui.ctx());
        let App { doc, host, open, shown, .. } = &mut *app;
        let values = doc.values;
        Dialog::new(TITLE)
            .opened(Bind::bound(open))
            .outer_gesture(true)
            .buttons([FooterButton::new("OK", DialogRole::Accept), FooterButton::new("Cancel", DialogRole::Cancel)])
            .show(ui, |ui, _| {
                shown.clear();
                let snap = Checkbox::new(Bind::given(values.snap)).label("Snap").show(ui);
                shown.push(snap.shown.clone());
                for event in snap.events {
                    host.handle(doc, event, |doc, snap| doc.set(Values { snap, ..doc.values }));
                }
                let grid = Checkbox::new(Bind::given(values.grid)).label("Show Grid").show(ui);
                shown.push(grid.shown.clone());
                for event in grid.events {
                    host.handle(doc, event, |doc, grid| doc.set(Values { grid, ..doc.values }));
                }
            });
    });
    fixture.settle();
    (fixture, app)
}

/// Clicks both check boxes: two field edits inside the dialog.
fn edit_both(fixture: &mut Fixture, app: &Rc<RefCell<App>>) {
    for label in ["Snap", "Show Grid"] {
        let target = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} check box is shown"));
        fixture.click(&target).settle();
    }
    let app = app.borrow();
    assert_eq!(app.doc.values, Values { snap: true, grid: true }, "each click edits the document");
    assert!(app.host.is_active(), "the dialog's gesture runs while it is open");
}

/// The outer steps in order, without their ids, and whether they all name one gesture.
fn outer_steps(app: &App) -> (Vec<&'static str>, bool) {
    let kind = |edit: &OuterEdit| match edit {
        OuterEdit::Begin { .. } => "Begin",
        OuterEdit::Commit { .. } => "Commit",
        OuterEdit::Cancel { .. } => "Cancel",
    };
    let id = |edit: &OuterEdit| match edit {
        OuterEdit::Begin { id, .. } | OuterEdit::Commit { id, .. } | OuterEdit::Cancel { id, .. } => Some(*id),
    };
    let one = app.outer.windows(2).all(|pair| id(&pair[0]) == id(&pair[1]));
    (app.outer.iter().map(kind).collect(), one)
}

/// Two field edits and OK: one outer `Begin` and `Commit`, and one undo step named by the dialog's title.
#[trial]
fn two_field_edits_and_ok_leave_one_undo_step_for_the_dialog() {
    let (mut fixture, app) = dialog();
    edit_both(&mut fixture, &app);
    let ok = fixture.by_label("OK").expect("the OK button is shown");
    fixture.click(&ok).settle();
    let mut app = app.borrow_mut();
    app.drain(fixture.ctx());
    assert!(!app.open, "OK closes the dialog");
    assert_eq!(outer_steps(&app), (vec!["Begin", "Commit"], true), "one outer gesture: {:?}", app.outer);
    assert!(!app.host.is_active(), "OK ends the dialog's gesture");
    assert_eq!(app.doc.steps, vec![(Values::default(), Some(TITLE.to_owned()))], "one step for the dialog, named by its title");
    assert_eq!(app.doc.values, Values { snap: true, grid: true }, "OK keeps both edits");
}

type Close<'a> = (&'a str, fn(&mut Fixture));

/// The same edits closed by Cancel or by Escape: the outer gesture is cancelled, the document is as before, and the
/// reopened dialog's fields show the values from before under a new outer gesture.
#[trial]
fn cancel_or_escape_takes_the_dialogs_field_edits_back() {
    let closes: [Close<'_>; 2] = [
        ("Cancel", |fixture| {
            let cancel = fixture.by_label("Cancel").expect("the Cancel button is shown");
            fixture.click(&cancel);
        }),
        ("Escape", |fixture| {
            fixture.key(Key::Escape, Modifiers::NONE);
        }),
    ];
    for (name, close) in closes {
        let (mut fixture, app) = dialog();
        edit_both(&mut fixture, &app);
        close(&mut fixture);
        fixture.settle();
        app.borrow_mut().drain(fixture.ctx());
        {
            let app = app.borrow();
            assert!(!app.open, "{name} closes the dialog");
            assert_eq!(outer_steps(&app), (vec!["Begin", "Cancel"], true), "{name}: one cancelled outer gesture: {:?}", app.outer);
            assert!(app.doc.steps.is_empty(), "{name} leaves no undo step: {:?}", app.doc.steps);
            assert_eq!(app.doc.values, Values::default(), "{name} restores both values");
        }
        app.borrow_mut().open = true;
        fixture.settle();
        let app = app.borrow();
        let before = Some(Mixed::Same(Arc::new(false)));
        assert_eq!(app.shown, vec![before.clone(), before], "{name}: the reopened fields show the values from before");
        assert_eq!(outer_steps(&app).0, vec!["Begin", "Cancel", "Begin"], "{name}: the reopened dialog begins a new gesture");
        assert!(matches!(app.outer[..], [OuterEdit::Begin { id: first, .. }, _, OuterEdit::Begin { id: second, .. }] if first != second));
    }
}
