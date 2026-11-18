//! Drive v3 REST: list a folder or what was shared, download or export, create, and update after
//! a version check.

use std::time::Instant;

use crate::encode::{drive_literal, form, pct};
use crate::http::{Method, Request, Response};
use crate::oauth::Auth;
use crate::{Error, Result};

pub const API: &str = "https://www.googleapis.com/drive/v3";
pub const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3";

pub const MIME_FOLDER: &str = "application/vnd.google-apps.folder";
/// Every native Google type (Docs, Sheets, Slides, Forms, …) starts with this.
pub const MIME_GOOGLE_PREFIX: &str = "application/vnd.google-apps.";

/// The most bytes downloaded or uploaded for one document.
pub const MAX_DOCUMENT: u64 = 256 << 20;
/// Files asked for per page (Drive's own maximum).
pub const PAGE_SIZE: u32 = 1000;
/// The most pages one folder listing reads: a folder of more than 5,000 entries shows its first
/// 5,000, so one listing can't grow without bound.
pub const MAX_PAGES: usize = 5;

const FIELDS: &str = "id,name,mimeType,modifiedTime,version,size,headRevisionId,parents,trashed,capabilities(canEdit),sharingUser(displayName)";

/// The native Google types that open by exporting them, with the Office Open XML type and the
/// extension they are exported as. A spreadsheet application opens a Google Sheet as `.xlsx`; a
/// word processor a Google Doc as `.docx`.
pub const EXPORTS: &[(&str, &str, &str)] = &[
    ("application/vnd.google-apps.document", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", "docx"),
    ("application/vnd.google-apps.spreadsheet", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "xlsx"),
    ("application/vnd.google-apps.presentation", "application/vnd.openxmlformats-officedocument.presentationml.presentation", "pptx"),
];

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)]
    pub can_edit: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    #[serde(default)]
    pub display_name: String,
}

/// A file's metadata, as much as the shared pages need.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    /// RFC 3339, shown as is.
    #[serde(default)]
    pub modified_time: String,
    /// Drive's change counter (an int64 sent as a string).
    #[serde(default)]
    pub version: String,
    /// Bytes; absent for native Google files and folders.
    #[serde(default, deserialize_with = "opt_u64_str")]
    pub size: Option<u64>,
    /// The folders it is in (one, in practice); empty when the folder isn't visible, such as a
    /// file shared with the person.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    /// The current content revision; empty for native Google files. Unlike `version`, renaming
    /// or sharing the file doesn't change it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub head_revision_id: String,
    /// Absent in older answers: then nothing is promised and a save is refused by Drive itself.
    #[serde(default)]
    pub capabilities: Option<Capabilities>,
    /// Who shared it, for a file shared with the person.
    #[serde(default)]
    pub sharing_user: Option<User>,
    /// In the trash: Drive still answers for it, and a save would land where nobody looks.
    #[serde(default)]
    pub trashed: bool,
}

fn opt_u64_str<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<u64>, D::Error> {
    let s: Option<String> = serde::Deserialize::deserialize(d)?;
    Ok(s.and_then(|s| s.parse().ok()))
}

impl DriveFile {
    /// What the conflict check compares: the content revision when Drive gives one, so a
    /// rename or a new share isn't taken for an edit; otherwise `version`.
    pub fn content_version(&self) -> &str {
        if self.head_revision_id.is_empty() { &self.version } else { &self.head_revision_id }
    }
    pub fn is_folder(&self) -> bool {
        self.mime_type == MIME_FOLDER
    }
    /// A native Google file (a Google Doc, Sheet, Slides deck, Form, …): never overwritten.
    pub fn is_native(&self) -> bool {
        self.mime_type.starts_with(MIME_GOOGLE_PREFIX)
    }
    /// For a native file that opens by export: the type it is exported as and its extension.
    pub fn export(&self) -> Option<(&'static str, &'static str)> {
        EXPORTS.iter().find(|(native, _, _)| *native == self.mime_type).map(|&(_, mime, ext)| (mime, ext))
    }
    /// The name it opens under: an exported file gets its export's extension (`Budget.xlsx`), so
    /// an application's format dispatch understands it.
    pub fn open_name(&self) -> String {
        match self.export() {
            Some((_, ext)) if !self.name.to_ascii_lowercase().ends_with(&format!(".{ext}")) => format!("{}.{ext}", self.name),
            _ => self.name.clone(),
        }
    }
    /// The type the bytes [`Drive::download`] returns are in.
    pub fn open_mime(&self) -> &str {
        self.export().map_or(self.mime_type.as_str(), |(mime, _)| mime)
    }
    /// Whether a save may replace its contents: the person may edit it and it isn't native.
    pub fn writable(&self) -> bool {
        !self.is_native() && !self.is_folder() && self.capabilities.as_ref().is_some_and(|c| c.can_edit)
    }
}

/// Where a save goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveTarget {
    /// Replace the contents of a file opened from Drive, if it is still at `version` (its
    /// [`DriveFile::content_version`] when it was opened).
    Existing { id: String, name: String, version: String },
    /// A new file, in `parent` (a folder id) or My Drive.
    New { name: String, parent: Option<String> },
}

/// The MIME type for a file name: the common document, spreadsheet, presentation and export
/// types, and `application/octet-stream` for anything else.
pub fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xlsm" => "application/vnd.ms-excel.sheet.macroEnabled.12",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ott" => "application/vnd.oasis.opendocument.text-template",
        "fodt" => "application/vnd.oasis.opendocument.text-flat-xml",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "fods" => "application/vnd.oasis.opendocument.spreadsheet-flat-xml",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "xls" => "application/vnd.ms-excel",
        "doc" => "application/msword",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "rtf" => "application/rtf",
        "txt" => "text/plain",
        "pdf" => "application/pdf",
        "html" | "htm" => "text/html",
        "md" => "text/markdown",
        "tex" => "application/x-tex",
        "json" => "application/json",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => "application/octet-stream",
    }
}

/// Drive ids are URL-safe base64-ish; anything else is refused before it reaches a URL. `root`
/// (My Drive) passes too.
pub fn check_id(id: &str) -> Result<&str> {
    if !id.is_empty() && id.len() <= 256 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        Ok(id)
    } else {
        Err(Error::Invalid(format!("not a Google Drive file id: {:?}", id.chars().take(40).collect::<String>())))
    }
}

fn check_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
        return Err(Error::Invalid("a Google Drive file name can't be empty, very long or contain control characters".into()));
    }
    Ok(name)
}

/// The search expression for one folder's contents, not in the trash.
pub fn folder_query(folder: &str) -> Result<String> {
    Ok(format!("{} in parents and trashed = false", drive_literal(check_id(folder)?)))
}

/// The search expression for what other people shared with this person, not in the trash.
pub const SHARED_QUERY: &str = "sharedWithMe = true and trashed = false";

/// The search expression for anything named `name` directly in `parent`, not in the trash.
pub fn child_query(parent: &str, name: &str) -> Result<String> {
    Ok(format!("{} in parents and name = {} and trashed = false", drive_literal(check_id(parent)?), drive_literal(name)))
}

/// The `multipart/related` body for a create: JSON metadata, then the content.
pub fn multipart(boundary: &str, metadata: &serde_json::Value, mime: &str, content: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(content.len() + 512);
    body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes());
    body.extend_from_slice(metadata.to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Type: {mime}\r\n\r\n").as_bytes());
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// A signed-in Drive client.
pub struct Drive {
    pub auth: Auth,
}

impl Drive {
    pub fn new(auth: Auth) -> Self {
        Self { auth }
    }

    /// Sends `req` with a fresh access token. A 401 (the token was revoked or expired early)
    /// drops the cached token and retries once after a refresh; if Google refuses the refresh,
    /// `access_token` signs out and this answers `NotSignedIn`. A 401 even with a freshly refreshed
    /// token is Drive's answer, not a sign-out: it comes back as that error.
    fn send(&mut self, req: Request) -> Result<Response> {
        let mut last = None;
        for _ in 0..2 {
            let token = self.auth.access_token(Instant::now())?;
            let resp = self.auth.http().send(req.clone().bearer(&token))?;
            if resp.status != 401 {
                return Ok(resp);
            }
            self.auth.invalidate_access();
            last = Some(resp);
        }
        Err(last.map_or(Error::NotSignedIn, |r| r.api_error()))
    }

    fn file(resp: Response) -> Result<DriveFile> {
        let v = resp.json()?;
        serde_json::from_value(v).map_err(|e| Error::Protocol(format!("file metadata: {e}")))
    }

    /// Who is signed in: the name Google shows, or the address when there is no name.
    pub fn who(&mut self) -> Result<String> {
        let v = self.send(Request::new(Method::Get, format!("{API}/about?fields={}", pct("user(displayName,emailAddress)"))))?.json()?;
        let user = v.get("user");
        let field = |k: &str| user.and_then(|u| u.get(k)).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty());
        Ok(field("displayName").or_else(|| field("emailAddress")).unwrap_or("Google Drive").chars().take(200).collect())
    }

    /// Every file matching `q`, folders first and then by name, at most [`MAX_PAGES`] pages.
    /// Entries with an id that isn't one are dropped.
    pub fn search(&mut self, q: &str) -> Result<Vec<DriveFile>> {
        let size = PAGE_SIZE.to_string();
        let fields = format!("nextPageToken,files({FIELDS})");
        let mut files = Vec::new();
        let mut next: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut pairs =
                vec![("q", q), ("fields", fields.as_str()), ("orderBy", "folder,name_natural"), ("pageSize", size.as_str()), ("spaces", "drive")];
            if let Some(t) = next.as_deref() {
                pairs.push(("pageToken", t));
            }
            let v = self.send(Request::new(Method::Get, format!("{API}/files?{}", form(&pairs))))?.json()?;
            if let Some(page) = v.get("files").and_then(|f| f.as_array()) {
                files.extend(
                    page.iter()
                        .take(PAGE_SIZE as usize)
                        .filter_map(|f| serde_json::from_value::<DriveFile>(f.clone()).ok())
                        .filter(|f| check_id(&f.id).is_ok()),
                );
            }
            next = v.get("nextPageToken").and_then(|t| t.as_str()).filter(|t| !t.is_empty() && t.len() <= 4096).map(str::to_owned);
            if next.is_none() {
                break;
            }
        }
        Ok(files)
    }

    /// The id of something named `name` directly in `parent`, when there is one.
    pub fn child_named(&mut self, parent: &str, name: &str) -> Result<Option<String>> {
        let q = child_query(parent, name)?;
        let pairs = [("q", q.as_str()), ("fields", "files(id)"), ("pageSize", "1"), ("spaces", "drive")];
        let v = self.send(Request::new(Method::Get, format!("{API}/files?{}", form(&pairs))))?.json()?;
        Ok(v.get("files")
            .and_then(|f| f.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("id"))
            .and_then(|i| i.as_str())
            .filter(|i| check_id(i).is_ok())
            .map(str::to_owned))
    }

    pub fn metadata(&mut self, id: &str) -> Result<DriveFile> {
        let id = check_id(id)?;
        let url = format!("{API}/files/{id}?fields={}", pct(FIELDS));
        Self::file(self.send(Request::new(Method::Get, url))?)
    }

    /// The file's bytes: the content for uploaded files, an Office Open XML export for Google
    /// Docs, Sheets and Slides. Other native files (Forms, Drawings, …) and folders are refused.
    pub fn download(&mut self, file: &DriveFile) -> Result<Vec<u8>> {
        let id = check_id(&file.id)?;
        if file.size.is_some_and(|s| s > MAX_DOCUMENT) {
            return Err(Error::Invalid(format!("{} is larger than {} MB", file.name, MAX_DOCUMENT >> 20)));
        }
        let url = match file.export() {
            Some((mime, _)) => format!("{API}/files/{id}/export?mimeType={}", pct(mime)),
            None if file.is_native() || file.is_folder() => {
                return Err(Error::Invalid(format!("{} can't be opened outside Google Drive", file.name)));
            }
            None => format!("{API}/files/{id}?alt=media"),
        };
        let resp = self.send(Request::new(Method::Get, url).limit(MAX_DOCUMENT))?;
        if !resp.ok() {
            return Err(resp.api_error());
        }
        Ok(resp.body)
    }

    /// Saves `content` and returns the file's new metadata.
    ///
    /// For an existing file, Drive's current version is read first; if it moved since the file
    /// was opened the save is refused with [`Error::Conflict`] and nothing is written. Drive v3
    /// has no conditional update, so a change landing between that read and the upload (well
    /// under a second) is not caught. Native Google files are never overwritten.
    pub fn save(&mut self, target: &SaveTarget, content: &[u8]) -> Result<DriveFile> {
        if content.len() as u64 > MAX_DOCUMENT {
            return Err(Error::Invalid(format!("documents larger than {} MB can't be saved to Drive", MAX_DOCUMENT >> 20)));
        }
        match target {
            SaveTarget::Existing { id, name, version } => {
                let id = check_id(id)?;
                let now = self.metadata(id)?;
                if now.is_native() || now.is_folder() {
                    return Err(Error::Invalid(format!("{} is a Google Drive file of its own type; save a copy instead", now.name)));
                }
                if now.trashed {
                    return Err(Error::Invalid(format!("{} is in the Google Drive trash; save a copy instead", now.name)));
                }
                if now.content_version() != version {
                    return Err(Error::Conflict { name: name.clone(), opened: version.clone(), remote: now.content_version().to_string() });
                }
                let url = format!("{UPLOAD_API}/files/{id}?uploadType=media&fields={}", pct(FIELDS));
                let req = Request::new(Method::Patch, url).body(mime_for(&now.name), content);
                Self::file(self.send(req)?)
            }
            SaveTarget::New { name, parent } => {
                let name = check_name(name)?;
                let mime = mime_for(name);
                let mut meta = serde_json::json!({ "name": name, "mimeType": mime });
                if let Some(p) = parent {
                    meta["parents"] = serde_json::json!([check_id(p)?]);
                }
                let boundary = format!("craft-{}", crate::pkce::random_token(18)?);
                let body = multipart(&boundary, &meta, mime, content);
                let url = format!("{UPLOAD_API}/files?uploadType=multipart&fields={}", pct(FIELDS));
                let req = Request::new(Method::Post, url).body(&format!("multipart/related; boundary={boundary}"), body);
                Self::file(self.send(req)?)
            }
        }
    }
}
