//! The theme under inspection, what the user picked in the panels, and the caches of the two sweeps.

use craft_theme::report::{ContrastReport, contrast_report};
use craft_theme::{AppId, AppSizes, Appearance, Explained, ThemeDef, ThemeId, TokenRef, explain_sized};
use std::sync::Arc;

/// The theme and appearance the panels explain, and the app whose pins apply.
#[derive(Clone)]
pub struct ThemeSubject<'a> {
    pub(crate) theme: &'a ThemeDef,
    pub(crate) appearance: Appearance,
    pub(crate) app: Option<&'a AppId>,
}

impl<'a> ThemeSubject<'a> {
    /// The subject `theme` shows in `appearance`, with no app pins.
    pub fn new(theme: &'a ThemeDef, appearance: Appearance) -> Self {
        ThemeSubject { theme, appearance, app: None }
    }

    /// The pins of `app` apply.
    #[must_use]
    pub fn app(mut self, app: &'a AppId) -> Self {
        self.app = Some(app);
        self
    }
}

/// A row the user picked in the browser, the Problems view or the detail: a colour or component token, or a size
/// by its path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Pick {
    /// A colour or component token.
    Token(TokenRef),
    /// A size, by its path such as `size.control.md`.
    Size(String),
}

/// The inputs of the last explanation, as the theme's id, the appearance, the app and the revision of the app layer,
/// and the explanation.
type Explanation = (ThemeId, Appearance, Option<AppId>, u64, Arc<Explained>);

/// What the theme panels share: the app layer, the picked row, and the explanation and contrast report of the
/// subject, computed again only when the subject's theme, appearance or app changes or the app layer is edited.
#[derive(Default)]
#[non_exhaustive]
pub struct ThemeWorkspace {
    explanation: Option<Explanation>,
    report: Option<(ThemeId, Arc<ContrastReport>)>,
    /// The row picked in the browser, the Problems view or the detail.
    pub pick: Option<Pick>,
    sizes: AppSizes,
    /// Counts the edits of `sizes`: the explanation is current while it has not moved.
    sizes_revision: u64,
}

impl ThemeWorkspace {
    /// A workspace with an empty app layer and nothing picked.
    pub fn new() -> Self {
        Self::default()
    }

    /// The app's size overrides: the sizes the gallery resolves with.
    pub fn sizes(&self) -> &AppSizes {
        &self.sizes
    }

    /// Counts the edits of the app's size overrides.
    pub fn sizes_revision(&self) -> u64 {
        self.sizes_revision
    }

    /// Runs `edit` over the app's size overrides. It returns whether it changed them; a change makes the next
    /// `refresh` explain the subject again.
    pub fn edit_sizes(&mut self, edit: impl FnOnce(&mut AppSizes) -> bool) {
        if edit(&mut self.sizes) {
            self.sizes_revision += 1;
        }
    }

    /// Brings the explanation and the contrast report up to date with `subject` and the app layer.
    pub fn refresh(&mut self, subject: &ThemeSubject<'_>) {
        let current = self.explanation.as_ref().is_some_and(|(theme, appearance, app, revision, _)| {
            *theme == subject.theme.id && *appearance == subject.appearance && app.as_ref() == subject.app && *revision == self.sizes_revision
        });
        if !current {
            let explained = Arc::new(explain_sized(subject.theme, subject.appearance, subject.app, &self.sizes));
            self.explanation = Some((subject.theme.id.clone(), subject.appearance, subject.app.cloned(), self.sizes_revision, explained));
        }
        if self.report.as_ref().is_none_or(|(theme, _)| *theme != subject.theme.id) {
            self.report = Some((subject.theme.id.clone(), Arc::new(contrast_report(subject.theme))));
        }
    }

    /// The explanation of the subject of the last `refresh`.
    pub fn explained(&self) -> Option<&Arc<Explained>> {
        self.explanation.as_ref().map(|(_, _, _, _, explained)| explained)
    }

    /// The contrast report of the theme of the last `refresh`.
    pub fn report(&self) -> Option<&Arc<ContrastReport>> {
        self.report.as_ref().map(|(_, report)| report)
    }
}
