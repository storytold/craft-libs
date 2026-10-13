//! Sharing through the OCS Share API.
//!
//! Nextcloud exposes shares at `/ocs/v2.php/apps/files_sharing/api/v1/shares`. Every call needs
//! the `OCS-APIRequest: true` header, which [`Account::request`] always sets.
//!
//! The envelope is the thing to know. OCS answers `HTTP 200` and reports the real outcome in its
//! own `<meta><statuscode>`, so a refused share arrives as a success at the HTTP layer. That is
//! why [`Error::Ocs`] exists separately from [`Error::Status`].
//!
//! Written from Nextcloud's published OCS Share API documentation.

use crate::error::{Error, Result};
use crate::http::{Http, Method};
use crate::{Account, encode_path, send, xml};

/// Who a share is for. The numbers are the API's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareKind {
    User,
    Group,
    PublicLink,
    Email,
}

impl ShareKind {
    pub fn as_u8(self) -> u8 {
        match self {
            ShareKind::User => 0,
            ShareKind::Group => 1,
            ShareKind::PublicLink => 3,
            ShareKind::Email => 4,
        }
    }

    fn from_str(raw: &str) -> Option<ShareKind> {
        match raw.trim() {
            "0" => Some(ShareKind::User),
            "1" => Some(ShareKind::Group),
            "3" => Some(ShareKind::PublicLink),
            "4" => Some(ShareKind::Email),
            _ => None,
        }
    }
}

/// What the recipient may do. The bits are the API's own, and they combine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Permissions(pub u32);

impl Permissions {
    pub const READ: Permissions = Permissions(1);
    pub const UPDATE: Permissions = Permissions(2);
    pub const CREATE: Permissions = Permissions(4);
    pub const DELETE: Permissions = Permissions(8);
    pub const SHARE: Permissions = Permissions(16);
    /// Read plus update, which is what co-editing a document needs.
    pub const EDIT: Permissions = Permissions(3);
    pub const ALL: Permissions = Permissions(31);

    pub fn contains(self, other: Permissions) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub fn with(self, other: Permissions) -> Permissions {
        Permissions(self.0 | other.0)
    }
}

/// A share to create.
#[derive(Clone, Debug)]
pub struct NewShare {
    /// Path relative to the user's files root.
    pub path: String,
    pub kind: ShareKind,
    /// The recipient. A user id or group id. Empty for a public link.
    pub share_with: String,
    pub permissions: Permissions,
    /// Optional password, which a public link usually wants.
    pub password: Option<String>,
    /// Optional expiry as `YYYY-MM-DD`, which is the only form the API takes.
    pub expires: Option<String>,
    /// Optional note shown to the recipient.
    pub note: Option<String>,
    /// Ask the server to mail the recipient. Meaningful for an email share,
    /// where the mail is the whole delivery, and harmless elsewhere.
    pub send_mail: bool,
}

impl NewShare {
    /// Share with a named user, read and write.
    pub fn with_user(path: impl Into<String>, user: impl Into<String>) -> NewShare {
        NewShare {
            path: path.into(),
            kind: ShareKind::User,
            share_with: user.into(),
            permissions: Permissions::EDIT,
            password: None,
            expires: None,
            note: None,
            send_mail: false,
        }
    }

    /// A public link, read only.
    pub fn public_link(path: impl Into<String>) -> NewShare {
        NewShare {
            path: path.into(),
            kind: ShareKind::PublicLink,
            share_with: String::new(),
            permissions: Permissions::READ,
            password: None,
            expires: None,
            note: None,
            send_mail: false,
        }
    }

    #[must_use]
    pub fn permissions(mut self, p: Permissions) -> NewShare {
        self.permissions = p;
        self
    }

    #[must_use]
    pub fn password(mut self, p: impl Into<String>) -> NewShare {
        self.password = Some(p.into());
        self
    }

    #[must_use]
    pub fn expires(mut self, date: impl Into<String>) -> NewShare {
        self.expires = Some(date.into());
        self
    }

    #[must_use]
    /// Whether the server should mail the recipient.
    pub fn send_mail(mut self, send: bool) -> NewShare {
        self.send_mail = send;
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> NewShare {
        self.note = Some(note.into());
        self
    }
}

/// A share as the server reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Share {
    pub id: String,
    pub kind: Option<ShareKind>,
    /// Path relative to the owner's files root.
    pub path: String,
    pub share_with: String,
    /// Display name for the recipient, when the server supplies one.
    pub share_with_name: String,
    pub permissions: Permissions,
    /// Set for a public link. Empty otherwise.
    pub url: String,
    pub owner: String,
    pub expires: String,
}

/// Share operations against one account.
pub struct Shares<'a> {
    account: &'a Account,
}

impl<'a> Shares<'a> {
    pub fn new(account: &'a Account) -> Shares<'a> {
        Shares { account }
    }

    /// Create a share and return it as the server recorded it.
    pub fn create<H: Http>(&self, http: &H, share: &NewShare) -> Result<Share> {
        // The API wants the path relative to the user's files root, so it is validated through the
        // same encoder as a WebDAV path and then sent decoded.
        let path = encode_path(&share.path)?;
        let path = crate::dav::decode(&path);

        let mut form = Form::new();
        form.add("path", &path);
        form.add("shareType", &share.kind.as_u8().to_string());
        if !share.share_with.is_empty() {
            form.add("shareWith", &share.share_with);
        }
        form.add("permissions", &share.permissions.0.to_string());
        if let Some(p) = &share.password {
            form.add("password", p);
        }
        if let Some(d) = &share.expires {
            form.add("expireDate", d);
        }
        if let Some(n) = &share.note {
            form.add("note", n);
        }
        // Stated rather than left to the server's default, which differs by
        // share type. Without this the dialog's checkbox would be advisory.
        if share.send_mail {
            form.add("sendMail", "true");
        }

        let req = self
            .account
            .request(Method::Post, format!("{}/shares", self.account.ocs_share_root()))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form.finish().into_bytes());
        let resp = send(http, req)?;
        check_ocs(&resp.body)?;

        xml::groups(&resp.body, "element")?
            .first()
            .map(share_from)
            // A single created share can arrive as <data> directly rather than wrapped in
            // <element>, which is why this falls back instead of failing.
            .or_else(|| xml::one_group(&resp.body, "data").ok().map(|g| share_from(&g)))
            .ok_or_else(|| Error::malformed("share response", "no share in the OCS data"))
    }

    /// List shares. With `path`, only shares of that file. Without, every share the user made.
    pub fn list<H: Http>(&self, http: &H, path: Option<&str>) -> Result<Vec<Share>> {
        let mut url = format!("{}/shares", self.account.ocs_share_root());
        if let Some(p) = path {
            let encoded = encode_path(p)?;
            let mut query = Form::new();
            query.add("path", &crate::dav::decode(&encoded));
            query.add("reshares", "true");
            url.push('?');
            url.push_str(&query.finish());
        }
        let resp = send(http, self.account.request(Method::Get, url))?;
        check_ocs(&resp.body)?;
        Ok(xml::groups(&resp.body, "element")?.iter().map(share_from).collect())
    }

    /// Remove a share by its id.
    pub fn delete<H: Http>(&self, http: &H, id: &str) -> Result<()> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(Error::invalid("share id", format!("{id:?}")));
        }
        let url = format!("{}/shares/{}", self.account.ocs_share_root(), id);
        let resp = send(http, self.account.request(Method::Delete, url))?;
        check_ocs(&resp.body)?;
        Ok(())
    }
}

/// Read the OCS envelope and turn a refusal into an error.
///
/// 100 is v1's success code and 200 is v2's. Both appear in the wild depending on the endpoint, so
/// both are accepted.
pub(crate) fn check_ocs(body: &[u8]) -> Result<()> {
    let meta = xml::one_group(body, "meta")?;
    let code = meta.get("statuscode").and_then(|s| s.parse::<u16>().ok());
    match code {
        Some(100) | Some(200) => Ok(()),
        Some(code) => Err(Error::Ocs { code, message: meta.get("message").unwrap_or("no message").to_owned() }),
        None => Err(Error::malformed("OCS envelope", "no statuscode in meta")),
    }
}

fn share_from(group: &xml::Group) -> Share {
    Share {
        id: group.get("id").unwrap_or_default().to_owned(),
        kind: group.get("share_type").and_then(ShareKind::from_str),
        path: group.get("path").unwrap_or_default().to_owned(),
        share_with: group.get("share_with").unwrap_or_default().to_owned(),
        share_with_name: group.get("share_with_displayname").unwrap_or_default().to_owned(),
        permissions: group.get("permissions").and_then(|s| s.parse().ok()).map(Permissions).unwrap_or_default(),
        url: group.get("url").unwrap_or_default().to_owned(),
        owner: group.get("uid_owner").unwrap_or_default().to_owned(),
        expires: group.get("expiration").unwrap_or_default().to_owned(),
    }
}

/// A `application/x-www-form-urlencoded` body, built in insertion order so tests can assert on it.
struct Form {
    out: String,
}

impl Form {
    fn new() -> Form {
        Form { out: String::new() }
    }

    fn add(&mut self, key: &str, value: &str) {
        if !self.out.is_empty() {
            self.out.push('&');
        }
        self.out.push_str(&crate::encode_segment(key));
        self.out.push('=');
        self.out.push_str(&crate::encode_segment(value));
    }

    fn finish(self) -> String {
        self.out
    }
}
