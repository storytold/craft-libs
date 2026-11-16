//! The built-in sources this build trusts: a commit of the index repository and the sha256 of its
//! `sources.json`.

use crate::source::encode_path;

/// `sources.json` pinned by commit and sha256. The indexes it lists are checked against the
/// sha256s in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexPin {
    /// The GitHub repository holding the indexes (`owner/name`).
    pub repo: &'static str,
    /// The repository's commit.
    pub commit: &'static str,
    /// The sha256 of `sources.json` at that commit, lowercase hex.
    pub sha256: &'static str,
}

impl IndexPin {
    /// Where `sources.json` is downloaded from: jsDelivr, then raw GitHub.
    pub fn urls(&self) -> Vec<String> {
        self.file_urls("sources.json")
    }

    /// Where a file of the repository at the pinned commit is downloaded from (an index's
    /// `path` or `gz_path`): jsDelivr, then raw GitHub.
    pub fn file_urls(&self, path: &str) -> Vec<String> {
        let (repo, commit, path) = (self.repo, self.commit, encode_path(path));
        vec![format!("https://cdn.jsdelivr.net/gh/{repo}@{commit}/{path}"), format!("https://raw.githubusercontent.com/{repo}/{commit}/{path}")]
    }
}

/// The built-in sources of this build: aiongg/craft-font-index at `5a9a7f8` (Google Fonts at
/// google/fonts `45e3d52`, and Noto), the copy `Index::bundled` compiles in.
pub const PINNED_INDEX: IndexPin = IndexPin {
    repo: "aiongg/craft-font-index",
    commit: "5a9a7f8038c8f3e2e1583a79c92a816e8e71b34f",
    sha256: "4b0c2b9cdec1885027dc3316cdc013ccfbc0d63b6ed9950138d762afaff72e23",
};
