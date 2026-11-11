//! `AboutDialog`: Help > About in one window for every app, with the app's own data. Tabs About, Contributors and
//! Models.

mod contributors;
mod credits;
#[cfg(feature = "testing")]
mod stories;

pub use contributors::{CommitShare, ContributorSort, NameMode};
pub use credits::{AboutInfo, AboutLink, Contributor, Credits, ModelCredit};

use crate::buttons::{Button, IconButton, Link};
use crate::choice::Segmented;
use crate::contract::{
    Bind, CollectionEvent, ColumnEvent, ColumnSort, ColumnSpec, Entry, NoValue, Outcome, Props as _, Remember as _, SelectMode, Show as _,
    ShowCollection as _, ShowContainer as _, clicked,
};
use crate::dialog::{Dialog, DialogOutput};
use crate::display::{Shade, Text};
use crate::layout::RowAlign;
use crate::layout_components::{Row, Stack};
use crate::list::ListRow;
use crate::pickers::Select;
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use crate::style::TypeRole;
use crate::table::Table;
use crate::tabs::{Tab, TabKind, TabStrip};
use contributors::{Writer, sorted_indices};
use craft_behavior::{ColumnKey, ColumnWidth, IconId, SelectionChange, keyed};
use craft_theme::Variant;
use egui::{Response, Ui};
use std::borrow::Cow;
use std::cell::RefCell;

craft_ui::words! {
    /// The words of the About dialog. `{app}` is the application's name. The counted words give "1 PR" and "3 PRs".
    pub struct AboutWords {
        title: template "aboutDialog.title" = "About {app}";
        about_tab: "aboutDialog.tab" = "About";
        contributors_tab: "aboutDialog.contributors" = "Contributors";
        models_tab: "aboutDialog.models" = "Models";
        version: template "aboutDialog.version" = "Version {version}";
        show: "aboutDialog.show" = "Show";
        username: "aboutDialog.username" = "Username";
        display_name: "aboutDialog.displayName" = "Display name";
        real_name: "aboutDialog.realName" = "Real name";
        sort: "aboutDialog.sort" = "Sort";
        reverse: "aboutDialog.reverse" = "Reverse the order";
        counts: template "aboutDialog.counts" = "Contributors: {contributors} · Commits: {commits}";
        no_contributors: "aboutDialog.noContributors" = "No contributor data was built into this copy.";
        no_models: "aboutDialog.noModels" = "No model credits were built into this copy.";
        sort_name: "aboutDialog.sortName" = "Name (A–Z)";
        sort_prs: "aboutDialog.sortPrs" = "Merged PRs";
        sort_commits: "aboutDialog.sortCommits" = "Commits";
        sort_lines_added: "aboutDialog.sortLinesAdded" = "Lines added";
        sort_lines_deleted: "aboutDialog.sortLinesDeleted" = "Lines deleted";
        sort_line_delta: "aboutDialog.sortLineDelta" = "Line delta";
        sort_binary_added: "aboutDialog.sortBinaryAdded" = "Binary assets added";
        sort_binary_deleted: "aboutDialog.sortBinaryDeleted" = "Binary assets removed";
        sort_first: "aboutDialog.sortFirst" = "First commit";
        sort_last: "aboutDialog.sortLast" = "Last commit";
        head_name: "aboutDialog.headName" = "Name";
        head_prs: "aboutDialog.headPrs" = "PRs";
        head_lines_added: "aboutDialog.headLinesAdded" = "+LOC";
        head_lines_deleted: "aboutDialog.headLinesDeleted" = "−LOC";
        head_line_delta: "aboutDialog.headLineDelta" = "ΔLOC";
        head_binary_added: "aboutDialog.headBinaryAdded" = "+Bin";
        head_binary_deleted: "aboutDialog.headBinaryDeleted" = "−Bin";
        head_first: "aboutDialog.headFirst" = "First";
        head_last: "aboutDialog.headLast" = "Last";
        company: "aboutDialog.company" = "Company";
        model: "aboutDialog.model" = "Model";
        model_version: "aboutDialog.modelVersion" = "Version";
        commits: "aboutDialog.commits" = "Commits";
        share: "aboutDialog.share" = "% of all commits";
        lines: "aboutDialog.lines" = "Lines +/−";
        plus: template "aboutDialog.plus" = "+{count}";
        minus: template "aboutDialog.minus" = "−{count}";
        lines_range: template "aboutDialog.linesRange" = "+{added} / −{deleted}";
        under_one: template "aboutDialog.underOne" = "<{percent}";
        summary: template "aboutDialog.summary" = "@{login}: {prs}, {commits}, +{added} / −{deleted} lines (Δ {delta}), +{binAdded} / −{binDeleted} binary assets, {first} – {last}";
        prs: count "aboutDialog.prCount" = "{n} PR" | "{n} PRs";
        commit_count: count "aboutDialog.commitCount" = "{n} commit" | "{n} commits";
    }
}

/// The dialog's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AboutTab {
    /// The application's name, blurb and links.
    #[default]
    About,
    /// The contributors table.
    Contributors,
    /// The credited AI models.
    Models,
}

/// What the dialog keeps while it is open, in its container's slot.
#[derive(Clone, Debug, Default, PartialEq)]
struct View {
    tab: AboutTab,
    names: NameMode,
    sort: ContributorSort,
    descending: bool,
}

/// The Models table's columns, the first of which holds the row label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelColumn {
    Company,
    Model,
    Version,
    Commits,
    Share,
    Lines,
}

impl ModelColumn {
    const ALL: [ModelColumn; 6] =
        [ModelColumn::Company, ModelColumn::Model, ModelColumn::Version, ModelColumn::Commits, ModelColumn::Share, ModelColumn::Lines];

    fn key(self) -> ColumnKey {
        ColumnKey::new(match self {
            ModelColumn::Company => "company",
            ModelColumn::Model => "model",
            ModelColumn::Version => "version",
            ModelColumn::Commits => "commits",
            ModelColumn::Share => "share",
            ModelColumn::Lines => "lines",
        })
    }

    fn head(self, words: &AboutWords) -> Label {
        match self {
            ModelColumn::Company => words.company.clone(),
            ModelColumn::Model => words.model.clone(),
            ModelColumn::Version => words.model_version.clone(),
            ModelColumn::Commits => words.commits.clone(),
            ModelColumn::Share => words.share.clone(),
            ModelColumn::Lines => words.lines.clone(),
        }
    }

    fn numeric(self) -> bool {
        matches!(self, ModelColumn::Commits | ModelColumn::Share | ModelColumn::Lines)
    }

    /// The cell text of a column that is not the label column; `total` counts every commit.
    fn cell(self, model: &ModelCredit, total: u64, writer: &Writer<'_>) -> String {
        match self {
            ModelColumn::Company => model.company.to_string(),
            ModelColumn::Model => model.model.to_string(),
            ModelColumn::Version => model.version.to_string(),
            ModelColumn::Commits => writer.count(model.commits),
            ModelColumn::Share => writer.share(model.commits, total),
            ModelColumn::Lines => writer.lines(model),
        }
    }
}

/// What the dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct AboutOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The URL of the link, the community button or the contributor activated in this pass. The dialog never opens
    /// it: the app does.
    pub link: Option<String>,
}

impl Outcome for AboutOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Help > About: tabs About, Contributors and Models over the app's [`AboutInfo`] and [`Credits`], with an OK button.
///
/// - **About.** The blurb, the version, the community button and the link row centred, the built-with line faint.
/// - **Contributors.** Show (Username, Display name, Real name) and Sort (the list, with a button to reverse) over a
///   table of every figure; a header press sorts by its column. Activating a row (Enter or a double click) reports the
///   contributor's profile URL as the link (nothing for one without a profile); the row's tooltip is the summary.
/// - **Models.** A table of the credited models with their commits, share of all commits and lines.
///
/// The tab, the name mode and the sort order are kept in the dialog's container while it is open. Links are reported
/// in [`AboutOutput::link`] and never opened from here. Enter and Escape close the dialog.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct AboutDialog<'a> {
    info: &'a AboutInfo,
    credits: &'a Credits,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// The tab the dialog opens on.
    #[prop]
    tab: Option<AboutTab>,
}

impl<'a> AboutDialog<'a> {
    /// The dialog over `info` and `credits`, opening on the About tab.
    pub fn new(info: &'a AboutInfo, credits: &'a Credits) -> Self {
        AboutDialog { info, credits, open: None, tab: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> AboutOutput {
        let AboutDialog { info, credits, open, tab } = self;
        let env = Env::of(ui);
        let words = env.words::<AboutWords>();
        let common = env.words::<CommonWords>();
        let dates = env.dates();
        let writer = Writer { format: env.format(), dates: &dates, words: &words };
        let title = Label::from(words.title.fill(&[("app", info.app_name.as_str())]));
        let shown = Shown { words: &words, writer: &writer, info, credits };

        let view = RefCell::new(View::default());
        let link = RefCell::new(None);
        let mut dialog =
            Dialog::new(title.clone()).collection_body().close_on_enter(true).buttons([FooterButton::new(common.ok.clone(), DialogRole::Accept)]);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let shown_dialog = dialog.show(ui, |ui, cx| {
            *view.borrow_mut() = if cx.appeared() { View { tab: tab.unwrap_or_default(), ..View::default() } } else { cx.memory::<View>().clone() };
            Stack::new().header(|ui, _| shown.tabs(ui, &title, &view)).show(ui, |ui, _| {
                let tab = view.borrow().tab;
                match tab {
                    AboutTab::About => shown.about(ui, &link),
                    AboutTab::Contributors => shown.contributors(ui, &view, &link),
                    AboutTab::Models => shown.models(ui),
                }
            });
            *cx.memory::<View>() = view.borrow().clone();
        });
        let DialogOutput { response, closed, .. } = shown_dialog.output;
        AboutOutput { response, closed, link: link.into_inner() }
    }
}

/// What the tabs read while the dialog shows them.
struct Shown<'s> {
    words: &'s AboutWords,
    writer: &'s Writer<'s>,
    info: &'s AboutInfo,
    credits: &'s Credits,
}

impl Shown<'_> {
    /// The tab strip; a pick of a tab is kept in `view`.
    fn tabs(&self, ui: &mut Ui, title: &Label, view: &RefCell<View>) {
        let words = self.words;
        let tabs = [
            (AboutTab::About, words.about_tab.clone()),
            (AboutTab::Contributors, words.contributors_tab.clone()),
            (AboutTab::Models, words.models_tab.clone()),
        ];
        let source = keyed(&tabs[..], |tab: &(AboutTab, Label)| tab.0).with_text(|tab| Cow::Borrowed(tab.1.as_str()));
        let shown = view.borrow().tab;
        let out = TabStrip::new(title.clone())
            .kind(TabKind::Text)
            .shown(shown)
            .show_items(ui, &source, &mut |key, _| Tab::new(tabs.iter().find(|tab| tab.0 == *key).map_or_else(Label::default, |tab| tab.1.clone())));
        for event in out.response.events {
            if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                && let Some(picked) = keys.last()
            {
                view.borrow_mut().tab = *picked;
            }
        }
    }

    /// The About tab.
    fn about(&self, ui: &mut Ui, link: &RefCell<Option<String>>) {
        let (words, info) = (self.words, self.info);
        Stack::new().show(ui, |ui, _| {
            Text::new(info.blurb.clone()).show(ui);
            Text::new(words.version.fill(&[("version", info.version.as_str())])).show(ui);
            Row::new().align(RowAlign::Center).show(ui, |ui, _| {
                if let Some(community) = &info.community
                    && clicked(&Button::new(community.label.clone()).variant(Variant::PRIMARY).id_salt("community").show(ui))
                {
                    *link.borrow_mut() = Some(community.url.clone());
                }
            });
            Row::new().align(RowAlign::Center).show(ui, |ui, _| {
                for (at, item) in info.links.iter().enumerate() {
                    if at > 0 {
                        Text::new("·").shade(Shade::Faint).show(ui);
                    }
                    if clicked(&Link::new(item.label.clone()).id_salt(("link", at)).show(ui)) {
                        *link.borrow_mut() = Some(item.url.clone());
                    }
                }
            });
            Text::new(info.built_with.clone()).shade(Shade::Faint).show(ui);
        });
    }

    /// The Contributors tab: the controls, the counts line and the table.
    fn contributors(&self, ui: &mut Ui, view: &RefCell<View>, link: &RefCell<Option<String>>) {
        let (words, writer) = (self.words, self.writer);
        let list: &[Contributor] = &self.credits.contributors;
        let mut work = view.borrow().clone();
        let names = [
            Entry::new(NameMode::Username, words.username.clone()),
            Entry::new(NameMode::DisplayName, words.display_name.clone()),
            Entry::new(NameMode::RealName, words.real_name.clone()),
        ];
        let sorts: Vec<Entry<ContributorSort>> = ContributorSort::ALL.iter().map(|key| Entry::new(*key, key.labels(words).0.clone())).collect();
        Stack::new().show(ui, |ui, _| {
            Row::new().wrap(true).show(ui, |ui, _| {
                Text::new(words.show.clone()).show(ui);
                Segmented::new(words.show.clone(), Bind::bound(&mut work.names), &names).show(ui);
                Text::new(words.sort.clone()).show(ui);
                let before = work.sort;
                Select::new(words.sort.clone(), Bind::bound(&mut work.sort), &sorts).show(ui);
                if work.sort != before {
                    work.descending = !work.sort.default_ascending();
                }
                let icon = IconId::new(if work.descending { "chevron_down" } else { "chevron_up" });
                if clicked(&IconButton::new(icon).label(words.reverse.clone()).show(ui)) {
                    work.descending = !work.descending;
                }
            });
            let counts =
                words.counts.fill(&[("contributors", &writer.count(list.len() as u64)), ("commits", &writer.count(self.credits.total_commits))]);
            Text::new(counts).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
            if list.is_empty() {
                Text::new(words.no_contributors.clone()).show(ui);
                return;
            }
            let order = sorted_indices(list, work.names, work.sort, !work.descending);
            let columns: Vec<ColumnSpec> = ContributorSort::ALL
                .iter()
                .map(|key| {
                    let width = if *key == ContributorSort::Name { ColumnWidth::fill(1.0) } else { ColumnWidth::Content };
                    let mut column = ColumnSpec::new(key.column(), key.labels(words).1.clone(), width).sortable();
                    if key.numeric() {
                        column = column.numeric();
                    }
                    if !key.default_ascending() {
                        column = column.first_descending();
                    }
                    column
                })
                .collect();
            let mode = work.names;
            let source = keyed(&order[..], |at: &usize| *at).with_text(|at| Cow::Owned(list[*at].name(mode).into_owned()));
            let by = if work.descending { ColumnSort::descending(work.sort.column()) } else { ColumnSort::ascending(work.sort.column()) };
            let out = Table::<usize>::new(words.contributors_tab.clone(), columns.clone()).select(SelectMode::Single).sort(Some(by)).show_items(
                ui,
                &source,
                &mut |key, icx| {
                    let contributor = &list[*key];
                    let mut row = ListRow::new(contributor.name(mode).into_owned())
                        .cells(icx.cells())
                        .columns(&columns)
                        .striped(icx.row.striped)
                        .tooltip(writer.summary(contributor));
                    for sort in ContributorSort::ALL.iter().filter(|sort| **sort != ContributorSort::Name) {
                        row = row.cell(sort.column(), writer.cell(*sort, contributor));
                    }
                    row
                },
            );
            for event in out.events {
                match event {
                    CollectionEvent::Column(ColumnEvent::Sort { key, descending, .. }) => {
                        if let Some(sort) = ContributorSort::of_column(&key) {
                            work.sort = sort;
                            work.descending = descending;
                        }
                    }
                    CollectionEvent::Activate(at) => {
                        *link.borrow_mut() = list.get(at).and_then(|contributor| contributor.profile.as_deref()).map(str::to_owned);
                    }
                    _ => {}
                }
            }
        });
        *view.borrow_mut() = work;
    }

    /// The Models tab.
    fn models(&self, ui: &mut Ui) {
        let (words, writer) = (self.words, self.writer);
        let models: &[ModelCredit] = &self.credits.models;
        if models.is_empty() {
            Text::new(words.no_models.clone()).show(ui);
            return;
        }
        let total = self.credits.share_total();
        let columns: Vec<ColumnSpec> = ModelColumn::ALL
            .iter()
            .map(|column| {
                let width = if *column == ModelColumn::Company { ColumnWidth::fill(1.0) } else { ColumnWidth::Content };
                let spec = ColumnSpec::new(column.key(), column.head(words), width);
                if column.numeric() { spec.numeric() } else { spec }
            })
            .collect();
        let order: Vec<usize> = (0..models.len()).collect();
        let source = keyed(&order[..], |at: &usize| *at).with_text(|at| Cow::Owned(models[*at].company.to_string()));
        let _ = Table::<usize>::new(words.models_tab.clone(), columns.clone()).select(SelectMode::None).show_items(ui, &source, &mut |key, icx| {
            let model = &models[*key];
            let mut row = ListRow::new(model.company.to_string()).cells(icx.cells()).columns(&columns).striped(icx.row.striped);
            for column in ModelColumn::ALL.iter().filter(|column| **column != ModelColumn::Company) {
                row = row.cell(column.key(), column.cell(model, total, writer));
            }
            row
        });
    }
}
