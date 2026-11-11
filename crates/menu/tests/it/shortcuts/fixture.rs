//! The catalog the dialog tests use, and a rig that shows the dialog pass by pass.

use std::cell::RefCell;
use std::rc::Rc;

use craft_host::target::Os;
use craft_keymap::{Binding, CommandDef, CommandKind, CommandRef, Keymap, Overrides, Platform, ProfileError, Profiles, Scope, Scopes};
use craft_menu::shortcuts::{DialogState, Options, Outcome, ShortcutsDialog};
use craft_ui::contract::Role;
use craft_ui::testing::{Fixture, Target};
use egui::{Key, Modifiers, Ui};

fn binding(text: &str) -> Binding {
    Binding::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

/// A small app: File and Object commands, two tools on `M`, a Timeline panel command and a Text context command.
pub(crate) fn keymap(platform: Platform) -> Keymap {
    let tool = CommandKind::Tool { group: Some("Marquee".into()) };
    let path = |p: &[&str]| p.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let timeline = Scope::new("Timeline").unwrap_or_else(Scope::app);
    let catalog = vec![
        CommandDef::new("file.save", "Save", Scope::app()).with_path(path(&["File", "Save"])).with_default(binding("Cmd+S")),
        CommandDef::new("file.saveAs", "Save As…", Scope::app()).with_path(path(&["File", "Save As…"])).with_default(binding("Cmd+Shift+S")),
        CommandDef::new("file.export", "Export…", Scope::app()).with_path(path(&["File", "Export…"])).with_default(binding("Cmd+E")),
        CommandDef::new("object.front", "Bring to Front", Scope::app()).with_path(path(&["Object", "Arrange", "Bring to Front"])),
        CommandDef::new("tool.marquee", "Marquee Tool", Scope::app()).with_kind(tool.clone()).with_default(binding("M")),
        CommandDef::new("tool.ellipse", "Elliptical Marquee Tool", Scope::app()).with_kind(tool).with_default(binding("M")),
        CommandDef::new("timeline.loop", "Loop", timeline).with_default(binding("L")),
        CommandDef::new("type.bold", "Bold", Scope::app()).with_default(binding("Text: Cmd+B")),
    ];
    Keymap::new(catalog, Scopes::new().with_panel("Timeline").with_context("Text"), platform)
}

/// The options a test starts from.
pub(crate) fn options() -> Options {
    Options::default()
}

/// The built-in sets of the tests' profiles.
pub(crate) fn builtins() -> Vec<(&'static str, Overrides)> {
    vec![("Fixture Defaults", Overrides::new())]
}

/// A step the dialog takes between passes, with the pass's `Ui` to read the words from.
type Job = Box<dyn for<'a> FnOnce(ShortcutsDialog<'a>, &Ui, &mut DialogState)>;

/// What the dialog edits, and what it reported.
pub(crate) struct Shared {
    pub km: Keymap,
    pub state: DialogState,
    pub options: Options,
    pub sets: Option<(Profiles, Vec<(&'static str, Overrides)>)>,
    pub outcomes: Vec<Outcome>,
    jobs: Vec<Job>,
}

/// The dialog shown in a fixture, one pass per frame.
pub(crate) struct Rig {
    pub fixture: Fixture,
    pub shared: Rc<RefCell<Shared>>,
}

impl Rig {
    /// A dialog over `km` with `options`, and named sets when `profiles` is given.
    pub(crate) fn new(km: Keymap, options: Options, profiles: Option<Profiles>) -> Rig {
        Rig::with_builtins(km, options, profiles.map(|p| (p, builtins())))
    }

    /// Like [`Rig::new`], with the built-in sets given too.
    pub(crate) fn with_builtins(km: Keymap, options: Options, sets: Option<(Profiles, Vec<(&'static str, Overrides)>)>) -> Rig {
        let shared = Rc::new(RefCell::new(Shared { km, state: DialogState::new(), options, sets, outcomes: Vec::new(), jobs: Vec::new() }));
        let pass = shared.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut guard = pass.borrow_mut();
            let Shared { km, state, options, sets, outcomes, jobs } = &mut *guard;
            for job in std::mem::take(jobs) {
                let mut dialog = ShortcutsDialog::new(km).options(options);
                if let Some((p, b)) = sets.as_mut() {
                    dialog = dialog.sets(p, b);
                }
                job(dialog, ui, state);
            }
            let mut dialog = ShortcutsDialog::new(km).options(options);
            if let Some((p, b)) = sets.as_mut() {
                dialog = dialog.sets(p, b);
            }
            outcomes.extend(dialog.show(ui, state).outcome);
        });
        let os = if shared.borrow().km.platform().is_mac() { Os::Mac } else { Os::Windows };
        fixture.os(os).room(egui::vec2(1000.0, 700.0));
        fixture.settle();
        Rig { fixture, shared }
    }

    /// A dialog on the fixture catalog for `platform`, with the options `tune` sets.
    pub(crate) fn tuned(platform: Platform, tune: impl FnOnce(&mut Options)) -> Rig {
        let mut options = options();
        tune(&mut options);
        Rig::new(keymap(platform), options, None)
    }

    /// A Mac dialog over the fixture catalog with default options and no sets.
    pub(crate) fn mac() -> Rig {
        Rig::new(keymap(Platform::Mac), options(), None)
    }

    /// A Mac dialog over the fixture catalog with sets.
    pub(crate) fn mac_with_sets(profiles: Profiles) -> Rig {
        Rig::new(keymap(Platform::Mac), options(), Some(profiles))
    }

    /// The control with this label; panics when there is none.
    pub(crate) fn find(&self, label: &str) -> Target {
        self.fixture.by_label(label).unwrap_or_else(|| {
            panic!("no control labelled {label:?}; buttons {:?}; rows {:?}", self.names_with(Role::BUTTON, ""), self.names_with(Role::ROW, ""))
        })
    }

    /// The names of the nodes of `role` that contain `part`, for a failure message.
    pub(crate) fn names_with(&self, role: Role, part: &str) -> Vec<String> {
        let tree = self.fixture.tree();
        self.fixture
            .by_role(role)
            .into_iter()
            .filter_map(|node| tree.name_of(node.node))
            .filter(|name| name.contains(part))
            .map(str::to_owned)
            .collect()
    }

    pub(crate) fn has(&self, label: &str) -> bool {
        self.fixture.by_label(label).is_some()
    }

    pub(crate) fn click(&mut self, label: &str) {
        let target = self.find(label);
        self.fixture.click(&target);
        self.fixture.settle();
    }

    /// Runs a set row action: its item when the row shows it, else its entry in More.
    pub(crate) fn action(&mut self, label: &str) {
        if !self.has(label) {
            self.click("More");
        }
        self.click(label);
    }

    /// Answers the open question with its action `label`: the enabled button or link of that name (the set row can carry a
    /// disabled one of the same name).
    pub(crate) fn answer(&mut self, label: &str) {
        let tree = self.fixture.tree();
        let button = [Role::BUTTON, Role::LINK]
            .into_iter()
            .flat_map(|role| self.fixture.by_role(role))
            .rfind(|node| tree.name_of(node.node) == Some(label) && !tree.is_disabled_key(node.node))
            .unwrap_or_else(|| panic!("no enabled answer named {label:?}"));
        self.fixture.click(&button);
        self.fixture.settle();
    }

    pub(crate) fn press(&mut self, modifiers: Modifiers, key: Key) {
        self.fixture.key(key, modifiers);
        self.fixture.settle();
    }

    /// The node of `role` named `label`.
    pub(crate) fn named(&self, label: &str, role: Role) -> Target {
        let tree = self.fixture.tree();
        self.fixture
            .by_role(role)
            .into_iter()
            .find(|node| tree.name_of(node.node) == Some(label))
            .unwrap_or_else(|| panic!("no {role:?} named {label:?}"))
    }

    /// Runs the frames a change made outside the dialog needs to show.
    pub(crate) fn pass(&mut self) {
        self.fixture.settle();
    }

    /// The control of `role` in the labelled row `label`: a labelled control carries its label's text as its name.
    pub(crate) fn control(&self, label: &str, role: Role) -> Target {
        let tree = self.fixture.tree();
        self.fixture
            .by_role(role)
            .into_iter()
            .find(|control| tree.name_of(control.node) == Some(label))
            .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
    }

    /// Opens the select of the row `label`.
    pub(crate) fn open(&mut self, label: &str) {
        let select = self.control(label, Role::COMBO_BOX);
        self.fixture.click(&select);
        self.fixture.settle();
    }

    /// Picks `entry` in the select of the row `label`.
    pub(crate) fn choose(&mut self, label: &str, entry: &str) {
        self.open(label);
        // A table group can carry the same name as an entry, so the entry is looked up among the select's options.
        let tree = self.fixture.tree();
        let option = [Role::LIST_BOX_OPTION, Role::MENU_ITEM, Role::MENU_ITEM_RADIO, Role::LIST_ITEM]
            .into_iter()
            .flat_map(|role| self.fixture.by_role(role))
            .find(|node| tree.name_of(node.node) == Some(entry));
        match option {
            Some(option) => {
                self.fixture.click(&option);
                self.fixture.settle();
            }
            None => self.click(entry),
        }
    }

    /// Whether the dialog is still open.
    pub(crate) fn is_open(&self) -> bool {
        self.shared.borrow().state.is_open()
    }

    /// The bindings of a command as text.
    pub(crate) fn bindings(&self, id: &str) -> Vec<String> {
        self.shared.borrow().km.bindings(&CommandRef::from(id)).iter().map(ToString::to_string).collect()
    }

    pub(crate) fn outcomes(&self) -> Vec<Outcome> {
        self.shared.borrow().outcomes.clone()
    }

    pub(crate) fn has_outcome(&self, wanted: impl Fn(&Outcome) -> bool) -> bool {
        self.shared.borrow().outcomes.iter().any(wanted)
    }

    pub(crate) fn active_set(&self) -> Option<String> {
        self.shared.borrow().sets.as_ref().map(|(p, _)| p.active().to_string())
    }

    /// Runs `job` with the dialog in the next pass and returns what it returns.
    pub(crate) fn import_json(&mut self, json: &str) -> Result<String, ProfileError> {
        let result = Rc::new(RefCell::new(None));
        let (out, text) = (result.clone(), json.to_owned());
        let job: Job = Box::new(move |dialog, ui, state| *out.borrow_mut() = Some(dialog.import_json(ui, state, &text)));
        self.shared.borrow_mut().jobs.push(job);
        self.fixture.idle(2);
        result.take().unwrap_or_else(|| panic!("the job did not run"))
    }
}
