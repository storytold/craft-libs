//! The folder agents open and save files in when the person set none, and the tool that tells an
//! agent which folders it may use.
//!
//! Each app has its own default folder, `Documents/ArtCraft/Agents/<App>`, in the system's
//! Documents folder on every desktop system, so people find what an agent made. The person's own
//! folder settings ([`AgentSettings::read_root`], [`AgentSettings::write_root`]) win; an empty one
//! falls back to the default folder ([`AgentSettings::folders`]).
//!
//! An agent learns the folders from the app's own MCP tool [`FOLDERS_TOOL`], answered with
//! [`folders_answer`]. The ArtCraft gateway passes app tools through, so the agent finds it by
//! searching for "folder" or "files", and the app reports the folders it really uses.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::AgentSettings;

/// The folders under Documents that hold every app's default agent folder.
pub const DEFAULT_FOLDER_PARENTS: [&str; 2] = ["ArtCraft", "Agents"];

/// Longest app name a default folder is made for; app names are short catalog names.
const MAX_APP_NAME: usize = 64;

/// The MCP tool an app offers to tell agents its folders.
pub const FOLDERS_TOOL: &str = "agent_folders";

/// [`FOLDERS_TOOL`]'s description, for the app's tool list.
pub const FOLDERS_TOOL_DESCRIPTION: &str = "Where this app opens and saves files for agents. Put files the app should open in \
     read_folder (or a folder below it) and find what it saves in write_folder. Both are usually the app's agent folder in \
     Documents/ArtCraft/Agents; the person may have set others. null means the app has no such folder now.";

/// The folders an agent session may open from and save to, after the default is filled in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AgentFolders {
    /// The folder files may be opened from.
    pub read: Option<PathBuf>,
    /// The folder files may be saved to.
    pub write: Option<PathBuf>,
}

/// The app's default agent folder, `Documents/ArtCraft/Agents/<app_name>`. `None` when the
/// system names no Documents folder and no home folder, or when `app_name` isn't a plain name
/// (empty, too long, or with anything other than ASCII letters, digits, `-` and `_`), so a name
/// can't lead outside the Agents folder. Doesn't touch the disk.
pub fn default_folder(app_name: &str) -> Option<PathBuf> {
    default_folder_in(dirs::document_dir(), dirs::home_dir(), app_name)
}

/// [`default_folder`] for these system folders: Documents, otherwise `<home>/Documents` (Linux
/// without an XDG Documents folder). A relative folder is ignored.
fn default_folder_in(documents: Option<PathBuf>, home: Option<PathBuf>, app_name: &str) -> Option<PathBuf> {
    if !plain_app_name(app_name) {
        return None;
    }
    let documents = documents.filter(|d| d.is_absolute()).or_else(|| home.filter(|h| h.is_absolute()).map(|h| h.join("Documents")))?;
    Some(DEFAULT_FOLDER_PARENTS.iter().fold(documents, |path, part| path.join(part)).join(app_name))
}

/// A name that is one ordinary path component on every system.
fn plain_app_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_APP_NAME && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl AgentSettings {
    /// The folders for agents of the app `app_name`: the person's own folder settings, and
    /// [`default_folder`] for an empty one ([`Self::folders_with`]).
    pub fn folders(&self, app_name: &str) -> AgentFolders {
        self.folders_with(default_folder(app_name).as_deref())
    }

    /// The person's own folder settings, and `default` for an empty one. The default folder is
    /// created when it is used; if it can't be, that direction has no folder (and the reason is
    /// logged), so an agent is refused rather than pointed at a folder that isn't there.
    pub fn folders_with(&self, default: Option<&Path>) -> AgentFolders {
        let (read, write) = (self.read_folder(), self.write_folder());
        let default = if read.is_none() || write.is_none() { default.and_then(created) } else { None };
        AgentFolders { read: read.or_else(|| default.clone()), write: write.or(default) }
    }
}

/// `dir`, created with its parents if missing; `None` when it can't be.
fn created(dir: &Path) -> Option<PathBuf> {
    match std::fs::create_dir_all(dir) {
        Ok(()) => Some(dir.to_path_buf()),
        Err(e) => {
            log::warn!("couldn't create the agent folder {}: {e}", dir.display());
            None
        }
    }
}

/// [`FOLDERS_TOOL`]'s answer: `read_folder` and `write_folder`, each a path or `null`. A path
/// that isn't Unicode is shown with replacement characters.
pub fn folders_answer(folders: &AgentFolders) -> Value {
    let path = |p: &Option<PathBuf>| p.as_deref().map(|p| p.to_string_lossy().into_owned());
    json!({ "read_folder": path(&folders.read), "write_folder": path(&folders.write) })
}

/// [`FOLDERS_TOOL`] as an MCP tool entry (`name`, `description`, an `inputSchema` with no
/// parameters), for an app that lists its tools as JSON.
pub fn folders_tool() -> Value {
    json!({
        "name": FOLDERS_TOOL,
        "description": FOLDERS_TOOL_DESCRIPTION,
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn abs(parts: &[&str]) -> PathBuf {
        let root = if cfg!(windows) { PathBuf::from(r"C:\") } else { PathBuf::from("/") };
        parts.iter().fold(root, |p, part| p.join(part))
    }

    // The default folder is in Documents; without one, in <home>/Documents.
    #[test]
    fn default_folder_is_in_documents_then_home() {
        let docs = abs(&["Users", "a", "Docs"]);
        let home = abs(&["Users", "a"]);
        assert_eq!(
            default_folder_in(Some(docs.clone()), Some(home.clone()), "PhotoCraft"),
            Some(docs.join("ArtCraft").join("Agents").join("PhotoCraft"))
        );
        assert_eq!(
            default_folder_in(None, Some(home.clone()), "PhotoCraft"),
            Some(home.join("Documents").join("ArtCraft").join("Agents").join("PhotoCraft"))
        );
        // A relative Documents folder is no place to put files.
        assert_eq!(
            default_folder_in(Some(PathBuf::from("Docs")), Some(home.clone()), "PhotoCraft"),
            Some(home.join("Documents").join("ArtCraft").join("Agents").join("PhotoCraft"))
        );
        assert_eq!(default_folder_in(None, Some(PathBuf::from("rel")), "PhotoCraft"), None);
        assert_eq!(default_folder_in(None, None, "PhotoCraft"), None);
    }

    // A name that could lead outside the Agents folder gets no default folder.
    #[test]
    fn odd_app_names_get_no_default_folder() {
        let docs = Some(abs(&["Docs"]));
        for name in ["", ".", "..", "../x", "a/b", r"a\b", "C:", "a b", "x".repeat(MAX_APP_NAME + 1).as_str(), "Fotó"] {
            assert_eq!(default_folder_in(docs.clone(), None, name), None, "{name:?}");
        }
        assert!(default_folder_in(docs, None, "grid-craft_2").is_some());
    }

    // The person's own folders win; the default only fills an empty setting, and is created.
    #[test]
    fn own_folders_win_and_the_default_is_created() {
        let mine = AgentSettings { read_root: " /in ".into(), write_root: "/out".into(), ..AgentSettings::default() };
        assert_eq!(mine.folders("PhotoCraft"), AgentFolders { read: Some(PathBuf::from("/in")), write: Some(PathBuf::from("/out")) });

        let dir = tempfile::tempdir().unwrap();
        let default = default_folder_in(Some(dir.path().to_path_buf()), None, "PhotoCraft").unwrap();
        assert_eq!(created(&default), Some(default.clone()));
        assert!(default.is_dir());
        // A file where the folder should be: no folder, not a crash.
        let blocked = dir.path().join("file");
        std::fs::write(&blocked, b"x").unwrap();
        assert_eq!(created(&blocked.join("Agents")), None);
    }

    // With no own folders, both directions get the one default folder, created; a default that
    // can't be created leaves them without one.
    #[test]
    fn empty_settings_use_one_default_folder() {
        let dir = tempfile::tempdir().unwrap();
        let default = dir.path().join("ArtCraft").join("Agents").join("PhotoCraft");
        let folders = AgentSettings::default().folders_with(Some(&default));
        assert_eq!(folders, AgentFolders { read: Some(default.clone()), write: Some(default.clone()) });
        assert!(default.is_dir());
        let only_read = AgentSettings { read_root: "/in".into(), ..AgentSettings::default() };
        assert_eq!(only_read.folders_with(Some(&default)), AgentFolders { read: Some(PathBuf::from("/in")), write: Some(default) });

        let file = dir.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(AgentSettings::default().folders_with(Some(&file.join("PhotoCraft"))), AgentFolders::default());
        assert_eq!(AgentSettings::default().folders("..").read, None);
    }

    // The tool's answer names both folders, null where there is none.
    #[test]
    fn the_answer_names_both_folders() {
        let folders = AgentFolders { read: Some(PathBuf::from("/in")), write: None };
        assert_eq!(folders_answer(&folders), json!({"read_folder": "/in", "write_folder": null}));
        let tool = folders_tool();
        assert_eq!(tool["name"], FOLDERS_TOOL);
        assert_eq!(tool["inputSchema"]["properties"], json!({}));
    }
}
