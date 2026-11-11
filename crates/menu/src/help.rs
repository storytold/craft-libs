//! The Help menu kit: the rows every Craft app's Help menu has, in one order, with the app's own
//! links between them.
//!
//! Photoshop's Help menu: Photoshop Help…, Hands-on Tutorials…, What's New…, Updates…, System
//! Info…, About Photoshop (Windows). An app's own links come after Keyboard Shortcuts…, then
//! Report an Issue…, Check for Updates…, a separator, System Info… and About.
//! [`HelpMenu`] builds that menu as model rows; the app runs the commands it reports, opening the
//! link URLs with [`HelpMenu::url`].

use craft_keymap::CommandRef;

use crate::model::{Item, ItemRole, Menu, MenuRole, Node};

/// A row of the Help menu kit. Each has a fixed command id (`help.<camelCase>`) so shortcuts,
/// automation and the keymap catalog name it the same in every app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HelpCommand {
    /// "{app} Help": the app's online help (`help.helpCenter`), opened in the browser.
    HelpCenter,
    /// "Keyboard Shortcuts…": the app's shortcut summary or the Keyboard Shortcuts dialog
    /// (`help.keyboardShortcuts`).
    KeyboardShortcuts,
    /// "Check for Updates…" (`help.checkForUpdates`), run through craft-update. Left out on the
    /// web.
    CheckForUpdates,
    /// "Report an Issue…" (`help.reportIssue`), opened in the browser.
    ReportIssue,
    /// "System Info…" (`help.systemInfo`): the app's system information dialog.
    SystemInfo,
    /// "About {app}" (`help.about`), tagged [`crate::ItemRole::About`] so the Mac layout moves it
    /// to the app menu.
    About,
}

impl HelpCommand {
    /// The command id: `help.helpCenter`, `help.keyboardShortcuts`, `help.checkForUpdates`,
    /// `help.reportIssue`, `help.systemInfo`, `help.about`.
    pub fn command_id(self) -> &'static str {
        match self {
            HelpCommand::HelpCenter => "help.helpCenter",
            HelpCommand::KeyboardShortcuts => "help.keyboardShortcuts",
            HelpCommand::CheckForUpdates => "help.checkForUpdates",
            HelpCommand::ReportIssue => "help.reportIssue",
            HelpCommand::SystemInfo => "help.systemInfo",
            HelpCommand::About => "help.about",
        }
    }

    /// The row for a command, `None` for any other command (an app link included).
    pub fn from_command(command: &CommandRef) -> Option<HelpCommand> {
        const ALL: [HelpCommand; 6] = [
            HelpCommand::HelpCenter,
            HelpCommand::KeyboardShortcuts,
            HelpCommand::CheckForUpdates,
            HelpCommand::ReportIssue,
            HelpCommand::SystemInfo,
            HelpCommand::About,
        ];
        ALL.into_iter().find(|c| c.command_id() == command.id)
    }
}

/// An app's own Help link: a row that opens `url` in the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HelpLink {
    /// The command id the row runs (`help.discord`).
    pub id: String,
    /// The row's label, translated by the app ("Join the ArtCraft Discord…").
    pub label: String,
    /// An `https` URL.
    pub url: String,
}

impl HelpLink {
    /// A link row running `id`, labelled `label`, that opens `url`.
    pub fn new(id: impl Into<String>, label: impl Into<String>, url: impl Into<String>) -> HelpLink {
        HelpLink { id: id.into(), label: label.into(), url: url.into() }
    }
}

/// The Help menu of one app.
///
/// ```no_run
/// use craft_menu::{HelpLabels, HelpLink, HelpMenu, MenuBar};
/// # let mut bar = MenuBar::default();
/// let help = HelpMenu::new("PhotoCraft")
///     .help_center("https://getartcraft.com/apps/photocraft/help")
///     .link(HelpLink::new("help.discord", "Join the ArtCraft Discord…", "https://discord.gg/artcraft"))
///     .report_issue("https://github.com/storytold/photocraft/issues")
///     .check_for_updates(true);
/// bar.menus.push(help.menu(&HelpLabels::default()));
/// // When a click reports a command:
/// # let command: craft_menu::CommandRef = "help.discord".into();
/// if let Some(url) = help.url(&command) { /* ctx.open_url(egui::OpenUrl::new_tab(url)) */ }
/// ```
///
/// Order: Help Center, Keyboard Shortcuts…, the app's links in the order given, Report an Issue…,
/// Check for Updates…, a separator, System Info…, About. Rows the app didn't ask for are left out.
/// The Mac layout moves About to the app menu and adds the system's search field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HelpMenu {
    app_name: String,
    help_center: Option<String>,
    keyboard_shortcuts: bool,
    links: Vec<HelpLink>,
    report_issue: Option<String>,
    check_for_updates: bool,
    system_info: bool,
}

impl HelpMenu {
    /// System Info… and About, nothing else.
    pub fn new(app_name: impl Into<String>) -> Self {
        HelpMenu { app_name: app_name.into(), system_info: true, ..HelpMenu::default() }
    }

    /// Adds "{app} Help", opening `url`.
    pub fn help_center(mut self, url: impl Into<String>) -> Self {
        self.help_center = Some(url.into());
        self
    }

    /// Adds "Keyboard Shortcuts…".
    pub fn keyboard_shortcuts(mut self, on: bool) -> Self {
        self.keyboard_shortcuts = on;
        self
    }

    /// Adds an app link after Keyboard Shortcuts… and the links before it.
    pub fn link(mut self, link: HelpLink) -> Self {
        self.links.push(link);
        self
    }

    /// Adds "Report an Issue…", opening `url`.
    pub fn report_issue(mut self, url: impl Into<String>) -> Self {
        self.report_issue = Some(url.into());
        self
    }

    /// Adds "Check for Updates…" (never on the web build).
    pub fn check_for_updates(mut self, on: bool) -> Self {
        self.check_for_updates = on;
        self
    }

    /// Whether System Info… shows (on by default).
    pub fn system_info(mut self, on: bool) -> Self {
        self.system_info = on;
        self
    }

    /// The rows in the kit's order, with About tagged [`crate::ItemRole::About`].
    pub fn items(&self, labels: &HelpLabels) -> Vec<Node> {
        let app = self.app_name.as_str();
        let row = |command: HelpCommand, label: &str| Node::Item(Item::new(command.command_id(), label));
        let mut nodes = Vec::new();
        if self.help_center.is_some() {
            nodes.push(row(HelpCommand::HelpCenter, &labels.help_center.fill(&[("app", app)])));
        }
        if self.keyboard_shortcuts {
            nodes.push(row(HelpCommand::KeyboardShortcuts, labels.keyboard_shortcuts.as_str()));
        }
        nodes.extend(self.links.iter().map(|l| Node::Item(Item::new(l.id.as_str(), l.label.as_str()))));
        if self.report_issue.is_some() {
            nodes.push(row(HelpCommand::ReportIssue, labels.report_issue.as_str()));
        }
        if self.check_for_updates {
            nodes.push(row(HelpCommand::CheckForUpdates, labels.check_for_updates.as_str()));
        }
        if !nodes.is_empty() {
            nodes.push(Node::Separator);
        }
        if self.system_info {
            nodes.push(row(HelpCommand::SystemInfo, labels.system_info.as_str()));
        }
        nodes.push(Node::Item(Item::new(HelpCommand::About.command_id(), labels.about.fill(&[("app", app)])).role(ItemRole::About)));
        nodes
    }

    /// The Help menu ([`crate::MenuRole::Help`]) titled `labels.help`, with [`HelpMenu::items`].
    pub fn menu(&self, labels: &HelpLabels) -> Menu {
        Menu::new(labels.help.as_str(), self.items(labels)).role(MenuRole::Help)
    }

    /// The URL a Help row opens: Help Center, Report an Issue… or an app link. `None` for the
    /// rows the app runs itself (Keyboard Shortcuts…, Check for Updates…, System Info…, About)
    /// and for other commands.
    pub fn url(&self, command: &CommandRef) -> Option<&str> {
        if let Some(link) = self.links.iter().find(|l| l.id == command.id) {
            return Some(link.url.as_str());
        }
        match HelpCommand::from_command(command)? {
            HelpCommand::HelpCenter => self.help_center.as_deref(),
            HelpCommand::ReportIssue => self.report_issue.as_deref(),
            _ => None,
        }
    }
}

craft_ui::words! {
    /// The Help menu kit's labels, English by default. `{app}` is the app's name.
    #[non_exhaustive]
    pub struct HelpLabels {
        help: "help.menu" = "Help";
        help_center: template "help.helpCenter" = "{app} Help";
        keyboard_shortcuts: "help.keyboardShortcuts" = "Keyboard Shortcuts…";
        report_issue: "help.reportIssue" = "Report an Issue…";
        check_for_updates: "help.checkForUpdates" = "Check for Updates…";
        system_info: "help.systemInfo" = "System Info…";
        about: template "help.about" = "About {app}";
    }
}
