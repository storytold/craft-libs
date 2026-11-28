//! A Color Picker dialog opened the way an image editor opens it from its foreground swatch, driven by keys, for the
//! picker's behaviour tests.

use std::cell::RefCell;
use std::rc::Rc;

use craft_host::{AppInfo, Host};
use craft_theme::Srgba;
use craft_ui::contract::Role;
use craft_ui::dialog::{DialogHost, dialog_id};
use craft_ui::testing::{Fixture, Target};
use egui::{Key, Modifiers, vec2};

use crate::ui::ColorPickerDialog;

/// The open dialog and the fixture that shows it.
pub(super) struct Rig {
    pub fixture: Fixture,
    pub dialogs: Rc<RefCell<DialogHost>>,
}

impl Rig {
    /// The foreground Color Picker opened on `start` (`#rrggbb`), its hex field focused.
    pub fn open(start: &str) -> Rig {
        let dialogs = Rc::new(RefCell::new(DialogHost::new()));
        let shown = Rc::clone(&dialogs);
        let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
        fixture.room(vec2(900.0, 700.0));
        fixture.host(std::sync::Arc::new(Host::builder(AppInfo { id: "picker-test", name: "Picker test", version: "0" }).build()));
        let colour = Srgba::from_hex(start).expect("a colour");
        dialogs.borrow_mut().open(ColorPickerDialog::new("foreground", "Color Picker (Foreground Color)", colour).web_colors_remembered(false));
        fixture.idle(6);
        Rig { fixture, dialogs }
    }

    /// The colour OK would answer, as `#rrggbb`.
    pub fn color(&self) -> String {
        self.dialogs.borrow().get::<ColorPickerDialog>().and_then(|d| d.current()).map(|p| p.color.to_hex()).unwrap_or_default()
    }

    /// The field named `name`: a channel's number field or the hex field (not the radio of the same name).
    pub fn field(&self, name: &str) -> Target {
        let tree = self.fixture.tree();
        [Role::SPIN_BUTTON, Role::TEXT_INPUT]
            .into_iter()
            .flat_map(|role| self.fixture.by_role(role))
            .find(|t| tree.name_of(t.node) == Some(name))
            .unwrap_or_else(|| panic!("the picker shows a {name} field"))
    }

    /// The text field `name` shows.
    pub fn text(&self, name: &str) -> String {
        let field = self.field(name);
        self.fixture.tree().value_of(field.node).unwrap_or_default().to_owned()
    }

    /// Presses `key` with `modifiers` in field `name`.
    pub fn key_in(&mut self, name: &str, key: Key, modifiers: Modifiers) {
        let field = self.field(name);
        self.fixture.click(&field);
        self.fixture.key(key, modifiers);
        self.fixture.settle();
    }

    /// Types `text` over field `name` and commits it with Tab (Enter would confirm the dialog).
    pub fn type_tab(&mut self, name: &str, text: &str) {
        let field = self.field(name);
        self.fixture.click(&field);
        self.fixture.key(Key::A, Modifiers::COMMAND);
        self.fixture.type_text(text);
        self.fixture.key(Key::Tab, Modifiers::NONE);
        self.fixture.settle();
    }

    /// Sets Only Web Colors as its check box would.
    pub fn web_colors(&mut self, on: bool) {
        let ctx = self.fixture.ctx().clone();
        let set = self.dialogs.borrow_mut().set_json_field(&ctx, dialog_id(("color-picker", "foreground")), "webColors", serde_json::json!(on));
        assert_eq!(set, Ok(()), "Only Web Colors is a field of the dialog");
        self.fixture.idle(4);
    }

    /// The range alert the dialog holds for the host, as its title and message, taken once.
    pub fn take_alert(&mut self) -> Option<(String, String)> {
        self.dialogs.borrow_mut().get_mut::<ColorPickerDialog>()?.take_alert_text()
    }
}
