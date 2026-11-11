//! The credits data: contributors, the credited AI models and the About tab's contents. The app bakes the tables at
//! build time and passes them in; nothing here carries interface text.

use crate::runtime::env::Label;
use std::borrow::Cow;

/// A link on the About tab.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct AboutLink {
    /// The link text.
    pub label: Label,
    /// The target URL, reported in the output for the app to open.
    pub url: String,
}

impl AboutLink {
    /// A link showing `label` to `url`.
    pub fn new(label: impl Into<Label>, url: impl Into<String>) -> Self {
        AboutLink { label: label.into(), url: url.into() }
    }
}

/// The About tab's contents.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct AboutInfo {
    /// The application's name, in the dialog title.
    pub app_name: Label,
    /// One sentence about the application.
    pub blurb: Label,
    /// The long version, such as `0.42.1 (a1b2c3d, 2026-10-30)`.
    pub version: Label,
    /// The prominent button, such as "Join us on Discord".
    pub community: Option<AboutLink>,
    /// The centred link row.
    pub links: Vec<AboutLink>,
    /// The faint last line naming what the application is built with.
    pub built_with: Label,
}

impl AboutInfo {
    /// An About tab for `app_name` with its `blurb` and `version`, without links.
    pub fn new(app_name: impl Into<Label>, blurb: impl Into<Label>, version: impl Into<Label>) -> Self {
        AboutInfo {
            app_name: app_name.into(),
            blurb: blurb.into(),
            version: version.into(),
            community: None,
            links: Vec::new(),
            built_with: Label::Empty,
        }
    }

    /// The prominent button under the blurb.
    #[must_use]
    pub fn community(mut self, link: AboutLink) -> Self {
        self.community = Some(link);
        self
    }

    /// The centred link row.
    #[must_use]
    pub fn links(mut self, links: impl IntoIterator<Item = AboutLink>) -> Self {
        self.links = links.into_iter().collect();
        self
    }

    /// The faint last line.
    #[must_use]
    pub fn built_with(mut self, text: impl Into<Label>) -> Self {
        self.built_with = text.into();
        self
    }
}

/// One contributor, as the build script bakes them. A display or real name is shown only with recorded consent;
/// without one the `@login` stands in.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Contributor {
    /// The login, shown as `@login` when no name is credited.
    pub login: Cow<'static, str>,
    /// The name the contributor consented to show.
    pub display_name: Option<Cow<'static, str>>,
    /// The real name the contributor consented to show.
    pub real_name: Option<Cow<'static, str>>,
    /// The URL of the contributor's profile, which activating the row reports as the link.
    pub profile: Option<Cow<'static, str>>,
    /// Merged pull requests.
    pub prs: u64,
    /// Commits.
    pub commits: u64,
    /// Lines added.
    pub lines_added: u64,
    /// Lines deleted.
    pub lines_deleted: u64,
    /// Binary assets added.
    pub binary_added: u64,
    /// Binary assets deleted.
    pub binary_deleted: u64,
    /// ISO 8601 UTC time of the first commit; empty for someone credited only through a merged pull request.
    pub first_commit: Cow<'static, str>,
    /// ISO 8601 UTC time of the latest commit; empty for someone credited only through a merged pull request.
    pub last_commit: Cow<'static, str>,
}

impl Contributor {
    /// A contributor with `login` and no counts.
    pub fn new(login: impl Into<Cow<'static, str>>) -> Self {
        Contributor {
            login: login.into(),
            display_name: None,
            real_name: None,
            profile: None,
            prs: 0,
            commits: 0,
            lines_added: 0,
            lines_deleted: 0,
            binary_added: 0,
            binary_deleted: 0,
            first_commit: Cow::Borrowed(""),
            last_commit: Cow::Borrowed(""),
        }
    }

    /// The name the contributor consented to show as a display name.
    #[must_use]
    pub fn display_name(mut self, name: impl Into<Cow<'static, str>>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// The name the contributor consented to show as a real name.
    #[must_use]
    pub fn real_name(mut self, name: impl Into<Cow<'static, str>>) -> Self {
        self.real_name = Some(name.into());
        self
    }

    /// The URL of the contributor's profile.
    #[must_use]
    pub fn profile(mut self, url: impl Into<Cow<'static, str>>) -> Self {
        self.profile = Some(url.into());
        self
    }

    /// Merged pull requests and commits.
    #[must_use]
    pub fn counts(mut self, prs: u64, commits: u64) -> Self {
        self.prs = prs;
        self.commits = commits;
        self
    }

    /// Lines added and deleted.
    #[must_use]
    pub fn lines(mut self, added: u64, deleted: u64) -> Self {
        self.lines_added = added;
        self.lines_deleted = deleted;
        self
    }

    /// Binary assets added and deleted.
    #[must_use]
    pub fn binary(mut self, added: u64, deleted: u64) -> Self {
        self.binary_added = added;
        self.binary_deleted = deleted;
        self
    }

    /// The ISO 8601 UTC times of the first and the latest commit.
    #[must_use]
    pub fn active(mut self, first: impl Into<Cow<'static, str>>, last: impl Into<Cow<'static, str>>) -> Self {
        self.first_commit = first.into();
        self.last_commit = last.into();
        self
    }

    /// Lines added minus lines deleted; negative when the contributor removed more than they added.
    pub fn lines_delta(&self) -> i128 {
        i128::from(self.lines_added) - i128::from(self.lines_deleted)
    }
}

/// An AI model credited in Co-Authored-By trailers.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModelCredit {
    /// The company that makes the model.
    pub company: Cow<'static, str>,
    /// The model's name.
    pub model: Cow<'static, str>,
    /// The model's version.
    pub version: Cow<'static, str>,
    /// Commits carrying the model's Co-Authored-By trailer.
    pub commits: u64,
    /// Lines added in those commits.
    pub lines_added: u64,
    /// Lines deleted in those commits.
    pub lines_deleted: u64,
}

impl ModelCredit {
    /// A credit for `model` of `company` at `version`, with no commits.
    pub fn new(company: impl Into<Cow<'static, str>>, model: impl Into<Cow<'static, str>>, version: impl Into<Cow<'static, str>>) -> Self {
        ModelCredit { company: company.into(), model: model.into(), version: version.into(), commits: 0, lines_added: 0, lines_deleted: 0 }
    }

    /// The commits with the trailer and the lines they added and deleted.
    #[must_use]
    pub fn work(mut self, commits: u64, lines_added: u64, lines_deleted: u64) -> Self {
        self.commits = commits;
        self.lines_added = lines_added;
        self.lines_deleted = lines_deleted;
        self
    }
}

/// The credits tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Credits {
    /// Rows of the Contributors tab.
    pub contributors: Cow<'static, [Contributor]>,
    /// Rows of the Models tab.
    pub models: Cow<'static, [ModelCredit]>,
    /// Every commit, for the counts line and the models' share.
    pub total_commits: u64,
}

impl Credits {
    /// Credits with `contributors`, `models` and the count of every commit.
    pub fn new(contributors: impl Into<Cow<'static, [Contributor]>>, models: impl Into<Cow<'static, [ModelCredit]>>, total_commits: u64) -> Self {
        Credits { contributors: contributors.into(), models: models.into(), total_commits }
    }

    /// Every commit, or the busiest model's count if the total trails it: the whole the models' shares are of.
    pub fn share_total(&self) -> u64 {
        self.total_commits.max(self.models.iter().map(|model| model.commits).max().unwrap_or(0))
    }
}
