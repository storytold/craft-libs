//! `AgentsPage`: the Agents page of Preferences, over the agent settings in the Preferences draft.

#[cfg(feature = "suite")]
pub(crate) mod stories;
mod words;

pub use words::AgentsWords;

use craft_behavior::NumberCodec;
use craft_host::{PickId, PickRequest};
use craft_theme::variant::Tone;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, FieldLayout, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::form::{Field, Help};
use craft_ui::layout_components::Stack;
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::patterns::preferences::{PathSetting, SettingsDraft};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::text::NumberField;
use egui::{Response, Ui};

use crate::agents::{AgentLinkStatus, AgentSettings, WhenAppClosed};

/// The purpose the folder picks are filed under.
const PICK_PURPOSE: &str = "agents";

/// What the page reported this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct AgentsPageOutput {
    /// The page's response.
    pub response: Response,
    /// The draft changed this pass (the window's Apply becomes enabled).
    pub changed: bool,
    /// A folder row's Browse filed this pick through the host's file service; its answer lands in the draft.
    pub browsing: Option<PickId>,
}

impl Outcome for AgentsPageOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Agents page of Preferences, drawn whole by the app as a custom page (`SettingsDocument::custom_page`) over the
/// [`AgentSettings`] saved under `page` (`"integrations"`): the keys `<page>.controlPort`, `<page>.readRoot`,
/// `<page>.writeRoot`, `<page>.whenAppClosed` and `<page>.allowAgentControl`. Each edit writes only its own key into
/// the draft, so Apply writes just that setting.
///
/// With the suite's `AI-MCP` flag, top to bottom: the line naming the ArtCraft Launcher's switch (the page has none of
/// its own), the link's status (the launch options manage it; its problem as an error; the port agents connect on; or
/// off), Control Port with its help, the two folders agents may open from and save to (typed, or picked with Browse…
/// through the host's file service, [`PathSetting`]), and what the app's CLI does while the app is closed. Without the
/// flag: Allow Agent Control and Control Port, the settings the app had before the switch. Labels name the app by the
/// host's app name.
#[must_use = "show the page with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct AgentsPage<'a> {
    page: &'a str,
    draft: &'a mut SettingsDraft,
    status: &'a AgentLinkStatus,
    /// The suite's `AI-MCP` flag is on: the launcher's switch decides, and the page shows the folders and the closed-app
    /// choice.
    #[prop]
    mcp_flag: bool,
}

impl<'a> AgentsPage<'a> {
    /// The page over the settings under `page` in the Preferences `draft`, showing the link's `status`.
    pub fn new(page: &'a str, draft: &'a mut SettingsDraft, status: &'a AgentLinkStatus) -> Self {
        AgentsPage { page, draft, status, mcp_flag: false }
    }

    /// Shows the page.
    pub fn show(self, ui: &mut Ui) -> AgentsPageOutput {
        let AgentsPage { page, draft, status, mcp_flag } = self;
        let env = Env::of(ui);
        let words = env.words::<AgentsWords>();
        let units = env.units();
        let app = env.view().host().map_or("", |host| host.app().name);
        let key = |name: &str| format!("{page}.{name}");
        let defaults = AgentSettings::default();
        let mut changed = false;
        let mut browsing = None;
        let shown = Stack::new().show(ui, |ui, _| {
            if mcp_flag {
                Text::new(words.intro.clone()).shade(Shade::Muted).show(ui);
                status_line(ui, &words, status, app);
            } else {
                let allow_key = key("allowAgentControl");
                let allowed = draft.get(&allow_key).unwrap_or(defaults.allow_agent_control);
                let mut next = None;
                Field::new(words.allow_agent_control.clone()).id_salt("allow").layout(FieldLayout::After).show(ui, |ui, _| {
                    next = Checkbox::new(Bind::given(allowed)).show(ui).events.landed().cloned();
                });
                if let Some(next) = next.filter(|next| *next != allowed) {
                    draft.set(&allow_key, &next);
                    changed = true;
                }
            }

            let port_key = key("controlPort");
            let port = draft.get::<i64>(&port_key).unwrap_or(i64::from(defaults.control_port));
            let mut next = None;
            let mut field = Field::new(words.control_port.clone()).id_salt("port");
            if mcp_flag {
                field = field.help(Help::Text(words.port_hint.clone()));
            }
            field.show(ui, |ui, _| {
                let number = NumberField::new(words.control_port.clone(), Bind::given(Some(port as f64)))
                    .codec(NumberCodec::new(units.reader(None), 0))
                    .range(0.0..=f64::from(u16::MAX))
                    .step(1.0);
                next = number.show(ui).edit.events.landed().cloned().flatten();
            });
            if let Some(next) = next.map(|v| v.round() as i64).filter(|next| *next != port) {
                draft.set(&port_key, &next);
                changed = true;
            }
            if !mcp_flag {
                return;
            }

            for (name, label) in [("readRoot", &words.read_root), ("writeRoot", &words.write_root)] {
                let folder_key = key(name);
                let out =
                    PathSetting::new(label.clone(), &folder_key, draft, PickRequest::folder(PICK_PURPOSE)).placeholder(words.none.clone()).show(ui);
                changed |= out.changed;
                browsing = browsing.or(out.browsing);
            }

            let closed_key = key("whenAppClosed");
            let current = draft.get::<String>(&closed_key).and_then(|name| WhenAppClosed::parse(&name)).unwrap_or(defaults.when_app_closed);
            let choices: Vec<Entry<WhenAppClosed>> = WhenAppClosed::ALL
                .into_iter()
                .map(|choice| {
                    let label: Label = match choice {
                        WhenAppClosed::Launch => words.launch.fill(&[("app", app)]).into(),
                        WhenAppClosed::Headless => words.headless.clone(),
                        WhenAppClosed::Error => words.ask.fill(&[("app", app)]).into(),
                    };
                    Entry::new(choice, label)
                })
                .collect();
            let label: Label = words.when_closed.fill(&[("app", app)]).into();
            let mut picked = None;
            Field::new(label.clone()).id_salt("closed").show(ui, |ui, _| {
                picked = Select::new(label.clone(), Bind::given(current), &choices).id_salt("closed").show(ui).value().copied();
            });
            if let Some(picked) = picked.filter(|picked| *picked != current) {
                draft.set(&closed_key, &picked.name());
                changed = true;
            }
        });
        AgentsPageOutput { response: shown.output, changed, browsing }
    }
}

/// One line on where the link stands: the launch options manage it, its problem, the port agents connect on, or off.
fn status_line(ui: &mut Ui, words: &AgentsWords, status: &AgentLinkStatus, app: &str) {
    if status.launch_flags {
        Text::new(words.managed.clone()).shade(Shade::Muted).show(ui);
    } else if let Some(problem) = &status.problem {
        Notice::new(Tone::Bad, Label::from(words.problem(problem, app))).layout(NoticeLayout::Inline).show(ui);
    } else if let Some(port) = status.port {
        Text::new(words.listening.fill(&[("port", &port.to_string())])).show(ui);
    } else {
        Text::new(words.off.clone()).shade(Shade::Faint).show(ui);
    }
}
