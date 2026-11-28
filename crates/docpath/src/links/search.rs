//! `LinkSearch` and `SearchHit`: a cancellable folder search for missing links, run as a job
//! (craft-browse's `SearchRun` runs it on a `JobQueue`).

use std::path::PathBuf;

use super::candidate::{last_component, split_ext};
use super::index::index_folder_reporting;
use super::{Candidate, Link, LinkFiles, MatchCriteria, check_candidate, rank};

/// Files indexed between two `Progress` updates.
const PROGRESS_STEP: usize = 500;

/// What to search: the folder, the matching rule, and how many files to look at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSearch {
    /// The folder the user chose (the dialog's "Search in").
    pub root: PathBuf,
    /// "Exact name matches only": only files whose whole name equals the link's, ignoring case.
    /// Off: files with the same name and any extension count too.
    pub exact_names: bool,
    /// The checks each found file goes through ([`check_candidate`](super::check_candidate)).
    pub criteria: MatchCriteria,
    /// Most files indexed: [`LinkSearch::LIMIT`] by default.
    pub limit: usize,
}

impl LinkSearch {
    /// The default file limit: 200 000.
    pub const LIMIT: usize = 200_000;

    /// A search of `root` with the default criteria, exact names off, and [`LinkSearch::LIMIT`].
    pub fn new(root: impl Into<PathBuf>) -> Self {
        // new: the dialog's defaults.
        Self { root: root.into(), exact_names: false, criteria: MatchCriteria::default(), limit: Self::LIMIT }
    }

    /// Runs the search for `links` (the missing ones): indexes `root` ([`index_folder`](super::index_folder)),
    /// reporting `Progress` every 500 files indexed, then for each link checks the files whose
    /// name matches (by `exact_names`) with [`check_candidate`](super::check_candidate), then
    /// `checker` (the app's own properties: it returns the candidate with its problems added and
    /// `ok` and `score` adjusted), and reports each as a `Hit`, in link order, each link's hits
    /// ranked ([`rank`](super::rank)) after the checker has run. `cancelled` is asked between folders and between
    /// checks; once it says yes nothing more is reported and the result is `Cancelled`. Blocks:
    /// run it in a job.
    pub fn run<K: Clone>(
        &self,
        links: &[Link<K>],
        files: &dyn LinkFiles,
        checker: &dyn Fn(&Link<K>, Candidate) -> Candidate,
        cancelled: &dyn Fn() -> bool,
        report: &mut dyn FnMut(SearchUpdate<K>),
    ) -> SearchEnd {
        // filmcraft: crates/engine/src/relink.rs:512 (`search`; it ran on the UI thread).
        let index = index_folder_reporting(&self.root, self.limit, files, cancelled, &mut |indexed| {
            if indexed % PROGRESS_STEP == 0 && !cancelled() {
                report(SearchUpdate::Progress { files: indexed });
            }
        });
        if cancelled() {
            return SearchEnd::Cancelled;
        }
        for link in links {
            let wanted = last_component(&link.stored).to_lowercase();
            let stem = split_ext(&wanted).0;
            let mut found: Vec<&PathBuf> = if self.exact_names {
                index.by_name.get(&wanted).into_iter().flatten().collect()
            } else {
                index.by_name.iter().filter(|(name, _)| split_ext(name).0 == stem).flat_map(|(_, paths)| paths).collect()
            };
            found.sort();
            let mut candidates = Vec::with_capacity(found.len());
            for path in found {
                if cancelled() {
                    return SearchEnd::Cancelled;
                }
                candidates.push(checker(link, check_candidate(link, path, &self.criteria, files)));
            }
            rank(&mut candidates);
            for candidate in candidates {
                if cancelled() {
                    return SearchEnd::Cancelled;
                }
                report(SearchUpdate::Hit(SearchHit { link: link.key.clone(), candidate }));
            }
        }
        if index.truncated { SearchEnd::Truncated { files: index.files } } else { SearchEnd::Done { files: index.files } }
    }
}

/// One candidate found for one link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit<K> {
    /// The link's key.
    pub link: K,
    /// The checked file.
    pub candidate: Candidate,
}

/// What a running search reports, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchUpdate<K> {
    /// Files indexed so far (for "Searching… 120 files").
    Progress {
        /// The number of files indexed so far.
        files: usize,
    },
    /// A candidate.
    Hit(SearchHit<K>),
}

/// How a search ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchEnd {
    /// Every folder was seen.
    Done {
        /// The number of files indexed.
        files: usize,
    },
    /// The limit stopped the walk.
    Truncated {
        /// The number of files indexed before the limit.
        files: usize,
    },
    /// `cancelled` said yes.
    Cancelled,
}
