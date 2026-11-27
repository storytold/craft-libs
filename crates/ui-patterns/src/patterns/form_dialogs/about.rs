//! `AboutDialog`: Help > About in one window for every app, with the app's own data. Tabs About, Contributors and
//! Models, then Licenses when the app gives license texts, then the app's own tabs.

mod contributors;
mod credits;
#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use contributors::{CommitShare, ContributorSort, NameMode};
pub use credits::{AboutImage, AboutInfo, AboutLicense, AboutLink, Contributor, Credits, ModelCredit};

use crate::buttons::{Button, IconButton, Link};
use crate::choice::Segmented;
use crate::contract::Gap;
use crate::contract::{
    Bind, CollectionEvent, ColumnEvent, ColumnSort, ColumnSpec, Entry, LeastWidth, NoValue, Outcome, Props as _, Remember as _, Scroll, SelectMode,
    Show as _, ShowCollection as _, ShowContainer as _, clicked,
};
use crate::dialog::{Dialog, DialogId, DialogOutput, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use crate::display::{Divider, Picture, PictureSize, Shade, Text};
use crate::layout::RowAlign;
use crate::layout_components::{Row, Stack};
use crate::list::{ListRow, ThumbSize};
use crate::pickers::Select;
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use crate::style::TypeRole;
use crate::table::Table;
use crate::tabs::{Tab, TabKind, TabStrip};
use crate::text::SelectableText;
use contributors::{Writer, sorted_indices};
use craft_behavior::{ColumnKey, ColumnWidth, EditEvent, IconId, Mixed, SelectionChange, keyed};
use craft_theme::{Mode, Variant};
use egui::{Response, Ui};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::Arc;

craft_ui::words! {
    /// The words of the About dialog. `{app}` is the application's name. The counted words give "1 PR" and "3 PRs".
    pub struct AboutWords {
        title: template "aboutDialog.title" = "About {app}";
        about_tab: "aboutDialog.tab" = "About";
        contributors_tab: "aboutDialog.contributors" = "Contributors";
        models_tab: "aboutDialog.models" = "Models";
        licenses_tab: "aboutDialog.licenses" = "Licenses";
        /// The name of the Licenses tab's picker, which lists the licenses by name.
        license: "aboutDialog.license" = "License";
        /// The footer button of the listed layout ([`AboutLayout::Listed`]).
        close: "aboutDialog.close" = "Close";
        version: template "aboutDialog.version" = "Version {version}";
        show: "aboutDialog.show" = "Show";
        username: "aboutDialog.username" = "Username";
        display_name: "aboutDialog.displayName" = "Display name";
        real_name: "aboutDialog.realName" = "Real name";
        sort: "aboutDialog.sort" = "Sort";
        reverse: "aboutDialog.reverse" = "Reverse the order";
        view: "aboutDialog.view" = "View";
        part_of: "aboutDialog.partOf" = "Part of";
        grab_bag: "aboutDialog.grabBag" = "Grab bag";
        table: "aboutDialog.table" = "Table";
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
    /// The contributors, as a grab bag of links or a table.
    Contributors,
    /// The credited AI models.
    Models,
    /// The app's third-party license texts, shown after Models when the app gives any.
    Licenses,
    /// The app's own tab at this position in the app's tabs, shown after Models and Licenses.
    App(u8),
}

/// How the About tab lays out the app's [`AboutInfo`], and the dialog's footer button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AboutLayout {
    /// The mark and the app's name centred, the blurb and the version, then the community button and the link row
    /// centred, then the built-with line faint, the blocks set apart by the section gap; OK closes.
    #[default]
    Centred,
    /// From the start edge, one item to a line: the app's name as a heading, the version, the blurb, the community
    /// link and then the other links in their order, the built-with line as a faint caption, the links set apart from
    /// the lines above and below them by the item gap; Close closes.
    Listed,
}

/// A tab the app adds after Models and Licenses: a name for the hosted dialog's `tab` field, a title and a body.
///
/// The body is `Send` because a [`HostedAbout`] lives on the dialog stack, which needs its dialogs `Send`.
pub struct AboutAppTab {
    name: &'static str,
    title: Label,
    body: Box<dyn FnMut(&mut Ui) + Send>,
}

impl AboutAppTab {
    /// A tab named `name` in the hosted dialog's `tab` field, titled `title`, whose content `body` draws.
    pub fn new(name: &'static str, title: impl Into<Label>, body: impl FnMut(&mut Ui) + Send + 'static) -> Self {
        AboutAppTab { name, title: title.into(), body: Box::new(body) }
    }
}

/// What the dialog keeps while it is open, in its container's slot.
#[derive(Clone, Debug, Default, PartialEq)]
struct View {
    tab: AboutTab,
    names: NameMode,
    sort: ContributorSort,
    descending: bool,
    /// The contributors show as a table; the grab bag of name links otherwise.
    table: bool,
    /// The position of the license the Licenses tab shows.
    license: usize,
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
/// - **About.** The app's mark when it has one, the blurb, the version, the community button and the link row
///   centred, then "Part of" with the suite's logo when it has one, the built-with line faint; or, with
///   [`AboutLayout::Listed`], the name, version, blurb and links one to a line, and Close for OK.
/// - **Contributors.** Show (Username, Display name, Real name), Sort (the list, with a button to reverse) and View
///   (Grab bag or Table). The grab bag flows the names as links: a click reports the contributor's profile URL as the
///   link (nothing for one without a profile), and the link's tooltip is the summary. The table has every figure; a
///   header press sorts by its column, activating a row (Enter or a double click) reports the profile URL, and the
///   row's tooltip is the summary.
/// - **Models.** A table of the credited models with their commits, share of all commits and lines.
/// - **Licenses.** Shown only when the app gives [`AboutLicense`]s ([`licenses`](AboutDialog::licenses)): a License
///   picker that lists them by name when there are several, then the picked license's full text, monospace and
///   selectable, in a vertically scrolling stack.
/// - **App tabs.** The app's own [`AboutAppTab`]s ([`app_tabs`](AboutDialog::app_tabs)) follow Models and Licenses,
///   each body in a vertically scrolling stack.
///
/// The name mode, the sort order, the view and the picked license are kept in the dialog's container while it is open,
/// and so is the tab unless [`tab`](AboutDialog::tab) binds it. Links are reported in [`AboutOutput::link`] and never
/// opened from here. Enter and Escape close the dialog.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct AboutDialog<'a> {
    info: &'a AboutInfo,
    credits: &'a Credits,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// Binds the shown tab to `tab`; the dialog opens on it.
    #[prop]
    tab: Option<Bind<'a, AboutTab>>,
    /// The third-party license texts the Licenses tab shows; without any the dialog has no Licenses tab.
    #[prop]
    licenses: Option<&'a [AboutLicense]>,
    /// The app's own tabs, shown after Models and Licenses.
    #[prop]
    app_tabs: Option<&'a mut [AboutAppTab]>,
    /// How the About tab lays out the info, and the footer button that goes with it.
    #[prop]
    layout: AboutLayout,
}

impl<'a> AboutDialog<'a> {
    /// The dialog over `info` and `credits`, opening on the About tab.
    pub fn new(info: &'a AboutInfo, credits: &'a Credits) -> Self {
        AboutDialog { info, credits, open: None, tab: None, licenses: None, app_tabs: None, layout: AboutLayout::default() }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> AboutOutput {
        let AboutDialog { info, credits, open, tab, licenses, mut app_tabs, layout } = self;
        let licenses = licenses.unwrap_or_default();
        // The strip shows the titles while the body draws a tab, so it reads its own copy of them.
        let app_titles: Vec<Label> = app_tabs.as_deref().unwrap_or_default().iter().map(|app| app.title.clone()).collect();
        let env = Env::of(ui);
        let words = env.words::<AboutWords>();
        let common = env.words::<CommonWords>();
        let dates = env.dates();
        let writer = Writer { format: env.format(), dates: &dates, words: &words };
        let title = Label::from(words.title.fill(&[("app", info.app_name.as_str())]));
        let shown = Shown { words: &words, writer: &writer, info, credits, licenses, layout };

        let mut tab = tab;
        let view = RefCell::new(View::default());
        let link = RefCell::new(None);
        let button = match layout {
            AboutLayout::Centred => common.ok.clone(),
            AboutLayout::Listed => words.close.clone(),
        };
        let mut dialog = Dialog::new(title.clone()).collection_body().close_on_enter(true).buttons([FooterButton::new(button, DialogRole::Accept)]);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        // The listed layout is a wide dialog, as WordCraft's About, whose tabs hold the credit tables.
        if layout == AboutLayout::Listed {
            dialog = dialog.least_width(LeastWidth::WideDialog);
        }
        let shown_dialog = dialog.show(ui, |ui, cx| {
            let mut kept = if cx.appeared() { View::default() } else { cx.memory::<View>().clone() };
            // A bound tab is the caller's each pass, so a tab set from outside shows on the next pass.
            if let Some(bind) = tab.as_mut()
                && let Some(Mixed::Same(held)) = bind.reborrow().read(cx)
            {
                kept.tab = held;
            }
            let before = kept.tab;
            *view.borrow_mut() = kept;
            Stack::new().header(|ui, _| shown.tabs(ui, &title, &app_titles, &view)).fill_width().show(ui, |ui, _| {
                let tab = view.borrow().tab;
                let app = match tab {
                    AboutTab::App(at) => app_tabs.as_deref_mut().and_then(|tabs| tabs.get_mut(usize::from(at))),
                    _ => None,
                };
                match (tab, app) {
                    (AboutTab::Contributors, _) => shown.contributors(ui, &view, &link),
                    (AboutTab::Models, _) => shown.models(ui),
                    (AboutTab::Licenses, _) if !shown.licenses.is_empty() => shown.licenses(ui, &view),
                    (AboutTab::App(_), Some(app)) => {
                        Stack::new().scroll(Scroll::vertical()).fill().show(ui, |ui, _| (app.body)(ui));
                    }
                    // An app tab past the given ones, and Licenses without licenses, show About.
                    _ => shown.about(ui, &link),
                }
            });
            let picked = view.borrow().tab;
            if picked != before
                && let Some(bind) = tab.as_mut()
            {
                let name: Arc<str> = Arc::from(title.as_str());
                let _ = bind.reborrow().write(cx, &EditEvent::set(Mixed::Same(before), picked, name, None));
            }
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
    licenses: &'s [AboutLicense],
    layout: AboutLayout,
}

impl Shown<'_> {
    /// The tab strip: the three built in, Licenses when there are licenses, then one per app tab titled `app`. A pick
    /// of a tab is kept in `view`.
    fn tabs(&self, ui: &mut Ui, title: &Label, app: &[Label], view: &RefCell<View>) {
        let words = self.words;
        let mut tabs = vec![
            (AboutTab::About, words.about_tab.clone()),
            (AboutTab::Contributors, words.contributors_tab.clone()),
            (AboutTab::Models, words.models_tab.clone()),
        ];
        if !self.licenses.is_empty() {
            tabs.push((AboutTab::Licenses, words.licenses_tab.clone()));
        }
        tabs.extend(app.iter().enumerate().filter_map(|(at, title)| u8::try_from(at).ok().map(|at| (AboutTab::App(at), title.clone()))));
        let source = keyed(&tabs[..], |tab: &(AboutTab, Label)| tab.0).with_text(|tab| Cow::Borrowed(tab.1.as_str()));
        let shown = view.borrow().tab;
        // The strip sits in the stack's header, which is as tall as what it holds: it takes its tabs' height. The listed
        // layout's tabs are a strip of tab faces over a rule, as WordCraft's About had them; the centred layout's are
        // its text.
        let kind = match self.layout {
            AboutLayout::Listed => TabKind::Panel,
            AboutLayout::Centred => TabKind::Text,
        };
        let out =
            TabStrip::new(title.clone()).kind(kind).shown(shown).natural_height(true).show_items(ui, &source, &mut |key, _| {
                Tab::new(tabs.iter().find(|tab| tab.0 == *key).map_or_else(Label::default, |tab| tab.1.clone()))
            });
        for event in out.response.events {
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                && let Some(picked) = keys.last()
            {
                view.borrow_mut().tab = *picked;
            }
        }
    }

    /// The About tab, laid out as [`AboutLayout`] says.
    fn about(&self, ui: &mut Ui, link: &RefCell<Option<String>>) {
        let (words, info) = (self.words, self.info);
        if self.layout == AboutLayout::Listed {
            // Three blocks set apart by the item gap, as WordCraft's About spaced them: the name, version and blurb; the
            // links; the built-with line.
            Stack::new().gap(Gap::Item).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    Text::new(info.app_name.clone()).role(TypeRole::Heading).show(ui);
                    Text::new(words.version.fill(&[("version", info.version.as_str())])).show(ui);
                    Text::new(info.blurb.clone()).show(ui);
                });
                Stack::new().show(ui, |ui, _| {
                    for (at, item) in info.community.iter().chain(info.links.iter()).enumerate() {
                        if clicked(&Link::new(item.label.clone()).id_salt(("link", at)).show(ui)) {
                            *link.borrow_mut() = Some(item.url.clone());
                        }
                    }
                });
                Text::new(info.built_with.clone()).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
            });
            return;
        }
        let dark = Env::of(ui).view().mode() == Mode::Dark;
        // Four blocks set apart by the section gap: the blurb and version at the start edge, the community button and
        // the link row each centred across the body, the built-with line at the start edge. The app's mark, when it has
        // one, is centred above them, and "Part of" with the suite's logo, when it has one, follows the link row.
        Stack::new().gap(Gap::Section).fill_width().show(ui, |ui, _| {
            if let Some(mark) = &info.mark {
                Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| {
                    Picture::svg(mark.label.clone(), mark.source(dark)).extent(PictureSize::Thumb(ThumbSize::Small)).show(ui);
                });
            }
            Stack::new().show(ui, |ui, _| {
                Text::new(info.blurb.clone()).show(ui);
                Text::new(words.version.fill(&[("version", info.version.as_str())])).show(ui);
            });
            if let Some(community) = &info.community {
                Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| {
                    if clicked(&Button::new(community.label.clone()).variant(Variant::PRIMARY).id_salt("community").show(ui)) {
                        *link.borrow_mut() = Some(community.url.clone());
                    }
                });
            }
            if !info.links.is_empty() {
                Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| {
                    for (at, item) in info.links.iter().enumerate() {
                        // A dot sets apart two text links; a link with an icon is a button of its own.
                        if at > 0 && item.icon.is_none() && info.links.get(at - 1).is_some_and(|previous| previous.icon.is_none()) {
                            Text::new("·").shade(Shade::Faint).show(ui);
                        }
                        let activated = if let Some(icon) = &item.icon {
                            clicked(&Button::new(item.label.clone()).icon(icon.clone()).variant(Variant::GHOST).id_salt(("link", at)).show(ui))
                        } else {
                            clicked(&Link::new(item.label.clone()).id_salt(("link", at)).show(ui))
                        };
                        if activated {
                            *link.borrow_mut() = Some(item.url.clone());
                        }
                    }
                });
            }
            if let Some(logo) = &info.part_of {
                Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| {
                    Text::new(words.part_of.clone()).shade(Shade::Muted).show(ui);
                    Picture::svg(logo.label.clone(), logo.source(dark)).show(ui);
                });
            }
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
        let views = [Entry::new(false, words.grab_bag.clone()), Entry::new(true, words.table.clone())];
        let sorts: Vec<Entry<ContributorSort>> = ContributorSort::ALL.iter().map(|key| Entry::new(*key, key.labels(words).0.clone())).collect();
        Stack::new().show(ui, |ui, _| {
            Row::new().wrap(true).show(ui, |ui, _| {
                Text::new(words.show.clone()).show(ui);
                Segmented::new(words.show.clone(), Bind::bound(&mut work.names), &names).show(ui);
                Divider::vertical().show(ui);
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
                Divider::vertical().show(ui);
                Segmented::new(words.view.clone(), Bind::bound(&mut work.table), &views).show(ui);
            });
            let counts =
                words.counts.fill(&[("contributors", &writer.count(list.len() as u64)), ("commits", &writer.count(self.credits.total_commits))]);
            Text::new(counts).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
            if list.is_empty() {
                Text::new(words.no_contributors.clone()).show(ui);
                return;
            }
            let order = sorted_indices(list, work.names, work.sort, !work.descending);
            let mode = work.names;
            if !work.table {
                // Each name is one link; a link that does not fit moves whole to the next line.
                Stack::new().scroll(Scroll::vertical()).fill().show(ui, |ui, _| {
                    Row::new().wrap(true).gap(Gap::Section).show(ui, |ui, _| {
                        for &at in &order {
                            let contributor = &list[at];
                            let name =
                                Link::new(contributor.name(mode).into_owned()).tooltip(writer.summary(contributor)).id_salt(("name", at)).show(ui);
                            if clicked(&name) {
                                *link.borrow_mut() = contributor.profile.as_deref().map(str::to_owned);
                            }
                        }
                    });
                });
                return;
            }
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
            let source = keyed(&order[..], |at: &usize| *at).with_text(|at| Cow::Owned(list[*at].name(mode).into_owned()));
            let by = if work.descending { ColumnSort::descending(work.sort.column()) } else { ColumnSort::ascending(work.sort.column()) };
            let out = Table::<usize>::new(words.contributors_tab.clone(), columns.clone())
                .select(SelectMode::Single)
                .sort(Some(by))
                // The rows are the credits named by `mode`: their measure holds while it does, whatever the order.
                .rows_stamp(mode as u64)
                .show_items(ui, &source, &mut |key, icx| {
                    let contributor = &list[*key];
                    let mut row = ListRow::new(contributor.name(mode).into_owned())
                        .cells(icx.cells())
                        .striped(icx.row.striped)
                        .tooltip(writer.summary(contributor));
                    for sort in ContributorSort::ALL.iter().filter(|sort| **sort != ContributorSort::Name) {
                        row = row.cell(sort.column(), writer.cell(*sort, contributor));
                    }
                    row
                });
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
        let _ = Table::<usize>::new(words.models_tab.clone(), columns.clone()).select(SelectMode::None).rows_stamp(0).show_items(
            ui,
            &source,
            &mut |key, icx| {
                let model = &models[*key];
                let mut row = ListRow::new(model.company.to_string()).cells(icx.cells()).striped(icx.row.striped);
                for column in ModelColumn::ALL.iter().filter(|column| **column != ModelColumn::Company) {
                    row = row.cell(column.key(), column.cell(model, total, writer));
                }
                row
            },
        );
    }

    /// The Licenses tab: the License picker when there are several licenses, then the picked license's text.
    fn licenses(&self, ui: &mut Ui, view: &RefCell<View>) {
        let words = self.words;
        let licenses = self.licenses;
        let mut work = view.borrow().clone();
        // A kept pick past the licenses given in this pass shows the first.
        if work.license >= licenses.len() {
            work.license = 0;
        }
        Stack::new().show(ui, |ui, _| {
            if licenses.len() > 1 {
                let entries: Vec<Entry<usize>> = licenses.iter().enumerate().map(|(at, license)| Entry::new(at, license.name.clone())).collect();
                Row::new().show(ui, |ui, _| {
                    Text::new(words.license.clone()).show(ui);
                    Select::new(words.license.clone(), Bind::bound(&mut work.license), &entries).show(ui);
                });
            }
            if let Some(license) = licenses.get(work.license) {
                Stack::new().scroll(Scroll::vertical()).fill().show(ui, |ui, _| {
                    SelectableText::new(license.name.clone(), &license.text).multiline().mono().id_salt(("license", work.license)).show(ui);
                });
            }
        });
        *view.borrow_mut() = work;
    }
}

/// An [`AboutDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns, with the app's info and credits. A link the
/// user activates waits in [`take_link`](HostedAbout::take_link) until the app opens it; the dialog answers `Ok(())`
/// when it closes.
///
/// ```ignore
/// host.open(HostedAbout::new(info, credits));
/// // each pass: if let Some(url) = host.get_mut::<HostedAbout>().and_then(HostedAbout::take_link) { open_url(url) }
/// ```
pub struct HostedAbout {
    id: DialogId,
    info: AboutInfo,
    credits: Credits,
    tab: AboutTab,
    link: Option<String>,
    open: bool,
    licenses: Vec<AboutLicense>,
    app_tabs: Vec<AboutAppTab>,
    layout: AboutLayout,
    #[cfg(feature = "testing")]
    facts: crate::testing::StoryOutput,
}

impl HostedAbout {
    /// The dialog over `info` and `credits`, opening on the About tab.
    pub fn new(info: AboutInfo, credits: Credits) -> Self {
        HostedAbout {
            id: dialog_id(ABOUT_KEY),
            info,
            credits,
            tab: AboutTab::default(),
            link: None,
            open: true,
            licenses: Vec::new(),
            app_tabs: Vec::new(),
            layout: AboutLayout::default(),
            #[cfg(feature = "testing")]
            facts: crate::testing::StoryOutput::default(),
        }
    }

    /// The license texts of the Licenses tab ([`AboutDialog::licenses`]); without any the dialog has no Licenses tab.
    #[must_use]
    pub fn licenses(mut self, licenses: impl IntoIterator<Item = AboutLicense>) -> Self {
        self.licenses = licenses.into_iter().collect();
        self
    }

    /// Adds an app tab after Models and Licenses.
    #[must_use]
    pub fn app_tab(mut self, tab: AboutAppTab) -> Self {
        self.app_tabs.push(tab);
        self
    }

    /// The tab the dialog opens on.
    #[must_use]
    pub fn tab(mut self, tab: AboutTab) -> Self {
        self.tab = tab;
        self
    }

    /// How the About tab lays out the info ([`AboutDialog::layout`]).
    #[must_use]
    pub fn layout(mut self, layout: AboutLayout) -> Self {
        self.layout = layout;
        self
    }

    /// The URL of the link the user activated since the last call, once. The app opens it.
    pub fn take_link(&mut self) -> Option<String> {
        self.link.take()
    }
}

/// The key of the hosted About dialog.
const ABOUT_KEY: &str = "help.about";

/// The tabs' names in the hosted dialog's `tab` field.
const TAB_NAMES: [(AboutTab, &str); 4] =
    [(AboutTab::About, "about"), (AboutTab::Contributors, "contributors"), (AboutTab::Models, "models"), (AboutTab::Licenses, "licenses")];

impl HostedAbout {
    /// The name of `tab` in the dialog's `tab` field: the app tabs' names as well as the ones built in. An app tab
    /// past the given ones, and Licenses without licenses, show About, so they are named so.
    fn tab_name(&self, tab: AboutTab) -> &str {
        if let AboutTab::App(at) = tab
            && let Some(app) = self.app_tabs.get(usize::from(at))
        {
            return app.name;
        }
        TAB_NAMES.iter().find(|(known, _)| *known == tab && self.has(*known)).map_or("about", |(_, name)| *name)
    }

    /// The tab named `name` in the dialog's `tab` field; a name built in wins over an app tab's of the same name while
    /// the dialog shows that tab.
    fn tab_named(&self, name: &str) -> Option<AboutTab> {
        if let Some((tab, _)) = TAB_NAMES.iter().find(|(tab, known)| *known == name && self.has(*tab)) {
            return Some(*tab);
        }
        let at = self.app_tabs.iter().position(|app| app.name == name)?;
        u8::try_from(at).ok().map(AboutTab::App)
    }

    /// Whether the dialog shows the built-in `tab`: Licenses only with licenses, the others always.
    fn has(&self, tab: AboutTab) -> bool {
        tab != AboutTab::Licenses || !self.licenses.is_empty()
    }
}

impl StackDialog for HostedAbout {
    type Result = ();

    fn confirm(&mut self, _: &egui::Context) -> Option<Self::Result> {
        Some(())
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({ "tab": self.tab_name(self.tab) }))
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if field != "tab" {
            return Err(FieldRefusal::Unknown.message(field));
        }
        let picked = value.as_str().and_then(|name| self.tab_named(name));
        let Some(picked) = picked else { return Err(FieldRefusal::Rejects(&value).message(field)) };
        self.tab = picked;
        Ok(())
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let shown = AboutDialog {
            info: &self.info,
            credits: &self.credits,
            open: Some(Bind::bound(&mut self.open)),
            tab: Some(Bind::bound(&mut self.tab)),
            licenses: Some(&self.licenses[..]),
            app_tabs: Some(&mut self.app_tabs[..]),
            layout: self.layout,
        }
        .show(ui);
        #[cfg(feature = "testing")]
        {
            self.facts = crate::contract::Outcome::story_facts(&shown);
        }
        if let Some(link) = shown.link {
            self.link = Some(link);
        }
        shown.closed.map(|_| DialogResult::Ok(()))
    }

    fn key(&self) -> Option<&str> {
        Some(ABOUT_KEY)
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.facts.clone()
    }
}
