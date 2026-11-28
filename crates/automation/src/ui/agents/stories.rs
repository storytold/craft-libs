//! Stories of the Agents page.

use super::AgentsPage;
use crate::agents::{AgentLinkStatus, LinkProblem};
use craft_ui::patterns::preferences::SettingsDraft;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn draft() -> SettingsDraft {
    let mut draft = SettingsDraft::default();
    draft.set("integrations.controlPort", &0_i64);
    draft.set("integrations.readRoot", &"/photos/agents");
    draft
}

fn status(port: Option<u16>, problem: Option<LinkProblem>) -> AgentLinkStatus {
    AgentLinkStatus { port, launch_flags: false, problem }
}

#[story(AgentsPage<'static>, covers = [Default], drive = Drive::Never)]
fn agents_page_listening(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let status = status(Some(4321), None);
    AgentsPage::new("integrations", data.value("listening", draft()), &status).mcp_flag(true).show(ui).into()
}

#[story(AgentsPage<'static>, covers = [], drive = Drive::Never)]
fn agents_page_off(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let status = status(None, None);
    AgentsPage::new("integrations", data.value("off", draft()), &status).mcp_flag(true).show(ui).into()
}

#[story(AgentsPage<'static>, covers = [], drive = Drive::Never)]
fn agents_page_problem(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let status = status(None, Some(LinkProblem::Listen { port: 7878, error: "Address already in use".into() }));
    AgentsPage::new("integrations", data.value("problem", draft()), &status).mcp_flag(true).show(ui).into()
}

#[story(AgentsPage<'static>, covers = [], drive = Drive::Never)]
fn agents_page_without_the_flag(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let status = status(None, None);
    AgentsPage::new("integrations", data.value("legacy", draft()), &status).show(ui).into()
}
