//! `AgentsWords`: every user-visible word of the Agents page (`agents.*`).

use crate::agents::LinkProblem;

craft_ui::words! {
    /// The Agents page's words, English by default. `{app}` is the app's name, `{port}` a port number and `{error}` the
    /// system's reason.
    #[non_exhaustive]
    pub struct AgentsWords {
        /// The line over the page: where the switch is.
        intro: "agents.intro" = "AI apps such as Claude can open, edit and save documents in this window while ArtCraft Launcher › Settings › AI agents › Let AI agents use Craft apps is on. Only programs on this computer can connect.";
        /// The status while the launch options run the control server.
        managed: "agents.managed" = "Managed by the launch options (--control).";
        /// The status while agents can connect.
        listening: template "agents.listening" = "Agents can connect on 127.0.0.1:{port}.";
        /// The status while agents can't connect and nothing went wrong.
        off: "agents.off" = "Off: turn on AI agents in ArtCraft Launcher › Settings › AI agents.";
        control_port: "agents.controlPort" = "Control Port";
        /// Help under the Control Port field.
        port_hint: "agents.portHint" = "0 picks a free port; AI apps find it by themselves";
        read_root: "agents.readRoot" = "Folder agents can open from";
        write_root: "agents.writeRoot" = "Folder agents can save to";
        /// An empty folder field.
        none: "agents.none" = "None";
        when_closed: template "agents.whenClosed" = "When {app} is closed";
        launch: template "agents.launch" = "Open {app}";
        headless: "agents.headless" = "Work without a window";
        ask: template "agents.ask" = "Ask to open {app}";
        allow_agent_control: "agents.allowAgentControl" = "Allow Agent Control";
        not_allowed: "agents.notAllowed" = "Agents can't connect: Allow Agent Control is off in the preferences file.";
        no_settings_folder: template "agents.noSettingsFolder" = "Agents can't connect: {app} has no settings folder (no home folder).";
        token_file: template "agents.tokenFile" = "Agents can't connect: the token file could not be written: {error}";
        listen: template "agents.listen" = "Couldn't listen on 127.0.0.1:{port}: {error}";
        port_out_of_range: template "agents.portOutOfRange" = "Agents can't connect: the control port {port} is outside 0 to 65535.";
        publish: template "agents.publish" = "Couldn't publish the agent connection: {error}";
    }
}

impl AgentsWords {
    /// The sentence for `problem`; `app` is the app's name. The app's own note is shown as it wrote it.
    pub fn problem(&self, problem: &LinkProblem, app: &str) -> String {
        match problem {
            LinkProblem::NotAllowed => self.not_allowed.as_str().to_owned(),
            LinkProblem::NoSettingsFolder => self.no_settings_folder.fill(&[("app", app)]),
            LinkProblem::TokenFile(error) => self.token_file.fill(&[("error", error)]),
            LinkProblem::Listen { port, error } => self.listen.fill(&[("port", &port.to_string()), ("error", error)]),
            LinkProblem::PortOutOfRange(port) => self.port_out_of_range.fill(&[("port", &port.to_string())]),
            LinkProblem::Publish(error) => self.publish.fill(&[("error", error)]),
            LinkProblem::App(note) => note.clone(),
        }
    }
}
