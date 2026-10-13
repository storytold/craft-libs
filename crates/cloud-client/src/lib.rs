//! Talking to CollabCraft, from an app.
//!
//! This is the only way a Craft app reaches storage, sharing or contacts. It
//! holds no provider credential, knows no provider URL and implements no
//! provider protocol. Swapping Nextcloud for something else changes nothing
//! here and nothing in any app.
//!
//! Signing in uses the device flow, because a desktop app cannot host a
//! redirect worth trusting. The app asks CollabCraft to start one, opens the
//! user's browser, and polls until it can collect a bearer token. Whatever
//! happens in that browser is the identity provider's business, so OIDC works
//! without a line of OAuth code here.
//!
//! Every call is blocking. An app's UI thread must therefore not call one
//! directly, and [`Client`] is cheap to move to a worker.

use std::time::Duration;

use craft_cloud::{Capability, Contact, Entry, Permission, Share, ShareTarget};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("collabcraft could not be reached: {0}")]
    Unreachable(String),
    #[error("sign in again")]
    Unauthenticated,
    #[error("link a cloud account first")]
    NotLinked,
    #[error("{code}: {detail}")]
    Refused { code: String, detail: String },
    #[error("unexpected answer: {0}")]
    Malformed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// What the server says about the signed-in user.
#[derive(Clone, Debug, Deserialize)]
pub struct Me {
    pub id: String,
    pub display: String,
    pub email: Option<String>,
    pub avatar: Option<String>,
    /// Apps this user may open.
    pub apps: Vec<String>,
    /// What the provider behind CollabCraft can do, so a UI hides what it
    /// cannot deliver rather than offering a button that fails.
    pub capabilities: Vec<Capability>,
    /// Until this is true, Open and Save have nowhere to go.
    pub provider_linked: bool,
}

/// A relay token and where to take it.
#[derive(Clone, Debug, Deserialize)]
pub struct Join {
    pub token: String,
    pub relay: String,
    pub room: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeviceStart {
    pub device_code: String,
    pub verify_url: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LinkStart {
    pub login_url: String,
    pub handle: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ShareRequest {
    pub id: String,
    pub targets: Vec<ShareTarget>,
    pub permission: Permission,
    pub note: Option<String>,
    pub notify: bool,
    /// A key the recipient must present. Only meaningful for a link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// `YYYY-MM-DD`, after which the share stops working.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ShareOutcome {
    pub shared: Vec<Share>,
    /// Named rather than counted, so a dialog can say which address failed.
    pub failed: Vec<ShareFailure>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ShareFailure {
    pub target: ShareTarget,
    pub code: String,
    pub detail: String,
}

#[derive(Deserialize)]
struct ApiError {
    code: String,
    detail: String,
}

/// A connection to one CollabCraft deployment.
#[derive(Clone)]
pub struct Client {
    base: String,
    token: Option<String>,
    http: ureq::Agent,
}

impl Client {
    /// `base` is the service root, such as `https://craft.example.com`.
    pub fn new(base: impl Into<String>) -> Client {
        Client {
            base: base.into().trim_end_matches('/').to_owned(),
            token: None,
            http: ureq::Agent::config_builder()
                .timeout_connect(Some(Duration::from_secs(10)))
                .timeout_global(Some(Duration::from_secs(60)))
                // Trust what the machine trusts. The default is Mozilla's root
                // list, which rejects any CollabCraft behind an internally
                // signed certificate, and says only "UnknownIssuer".
                .tls_config(
                    ureq::tls::TlsConfig::builder()
                        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                        .build(),
                )
                // A non-2xx is an answer carrying a reason, not a transport
                // failure. Treating it as an error would throw the body away
                // and leave every refusal looking identical.
                .http_status_as_error(false)
                .build()
                .into(),
        }
    }

    /// Present a session token on later calls.
    pub fn with_token(mut self, token: impl Into<String>) -> Client {
        self.token = Some(token.into());
        self
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn is_signed_in(&self) -> bool {
        self.token.is_some()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn get(&self, path: &str) -> Result<Vec<u8>> {
        let mut req = self.http.get(self.url(path));
        if let Some(t) = &self.token {
            req = req.header("authorization", &format!("Bearer {t}"));
        }
        read(req.call())
    }

    fn post_json<T: Serialize>(&self, path: &str, body: &T) -> Result<Vec<u8>> {
        let mut req = self.http.post(self.url(path));
        if let Some(t) = &self.token {
            req = req.header("authorization", &format!("Bearer {t}"));
        }
        read(req.send_json(body))
    }

    // ---- signing in -------------------------------------------------------

    /// Begin a sign-in. Open [`DeviceStart::verify_url`] in a browser, then
    /// poll.
    pub fn sign_in_start(&self) -> Result<DeviceStart> {
        parse(&self.post_json("/auth/device/start", &())?)
    }

    /// Collect the session once the browser is done.
    ///
    /// `Ok(None)` means the user has not finished yet, which is a state rather
    /// than a failure, so a caller loops on it.
    pub fn sign_in_poll(&self, device_code: &str) -> Result<Option<String>> {
        #[derive(Serialize)]
        struct Body<'a> {
            device_code: &'a str,
        }
        #[derive(Deserialize)]
        struct Token {
            token: String,
        }
        match self.post_json("/auth/device/poll", &Body { device_code }) {
            Ok(body) => Ok(Some(parse::<Token>(&body)?.token)),
            Err(Error::Refused { code, .. }) if code == "authorization_pending" => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Run a sign-in to completion, calling `show` with the URL to open.
    ///
    /// Blocks, so it belongs on a worker. The deadline is the server's, which
    /// is why it is not a parameter.
    pub fn sign_in(&mut self, show: impl FnOnce(&str)) -> Result<()> {
        let start = self.sign_in_start()?;
        show(&start.verify_url);
        let deadline = std::time::Instant::now() + Duration::from_secs(start.expires_in);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_secs(start.interval.clamp(1, 10)));
            if let Some(token) = self.sign_in_poll(&start.device_code)? {
                self.token = Some(token);
                return Ok(());
            }
        }
        Err(Error::Refused { code: "expired".into(), detail: "the sign-in timed out".into() })
    }

    // ---- linking a cloud account ------------------------------------------

    pub fn link_start(&self) -> Result<LinkStart> {
        parse(&self.post_json("/auth/link/start", &())?)
    }

    /// `Ok(false)` means the user has not finished in the browser yet.
    pub fn link_poll(&self, handle: &str) -> Result<bool> {
        #[derive(Serialize)]
        struct Body<'a> {
            handle: &'a str,
        }
        match self.post_json("/auth/link/poll", &Body { handle }) {
            Ok(_) => Ok(true),
            Err(Error::Refused { code, .. }) if code == "authorization_pending" => Ok(false),
            Err(e) => Err(e),
        }
    }

    // ---- the things an app actually does ----------------------------------

    pub fn me(&self) -> Result<Me> {
        parse(&self.get("/api/v1/me")?)
    }

    /// One folder. An empty path is the user's root.
    pub fn list(&self, path: &str) -> Result<Vec<Entry>> {
        parse(&self.get(&format!("/api/v1/files?path={}", enc(path)))?)
    }

    /// Everything other people have shared with this user.
    pub fn shared_with_me(&self) -> Result<Vec<Entry>> {
        parse(&self.get("/api/v1/files/shared")?)
    }

    pub fn read(&self, id: &str) -> Result<Vec<u8>> {
        self.get(&format!("/api/v1/files/content?id={}", enc(id)))
    }

    /// Save, refusing to overwrite a newer version when `if_match` is given.
    pub fn write(&self, id: &str, bytes: &[u8], if_match: Option<&str>) -> Result<Entry> {
        let mut url = format!("/api/v1/files/content?id={}", enc(id));
        if let Some(tag) = if_match {
            url.push_str(&format!("&if_match={}", enc(tag)));
        }
        let mut req = self.http.put(self.url(&url));
        if let Some(t) = &self.token {
            req = req.header("authorization", &format!("Bearer {t}"));
        }
        parse(&read(req.send(bytes))?)
    }

    pub fn create(&self, folder: &str, name: &str, bytes: &[u8]) -> Result<Entry> {
        let url = format!("/api/v1/files/new?folder={}&name={}", enc(folder), enc(name));
        let mut req = self.http.post(self.url(&url));
        if let Some(t) = &self.token {
            req = req.header("authorization", &format!("Bearer {t}"));
        }
        parse(&read(req.send(bytes))?)
    }

    /// People matching what has been typed, for a share dialog.
    pub fn contacts(&self, query: &str, limit: usize) -> Result<Vec<Contact>> {
        parse(&self.get(&format!("/api/v1/contacts?q={}&limit={limit}", enc(query)))?)
    }

    /// Share with several people at once.
    ///
    /// Partial success is reported rather than hidden, so a dialog can say
    /// which address failed instead of failing the lot.
    pub fn share(&self, request: &ShareRequest) -> Result<ShareOutcome> {
        parse(&self.post_json("/api/v1/shares", request)?)
    }

    pub fn shares(&self, id: &str) -> Result<Vec<Share>> {
        parse(&self.get(&format!("/api/v1/shares?id={}", enc(id)))?)
    }

    /// Withdraw one share.
    ///
    /// `share_id` is the one [`Share::id`] carried, and means nothing to
    /// anybody but the provider that issued it.
    pub fn unshare(&self, share_id: &str) -> Result<()> {
        let mut req = self.http.delete(self.url(&format!("/api/v1/shares/{}", enc(share_id))));
        if let Some(t) = &self.token {
            req = req.header("authorization", &format!("Bearer {t}"));
        }
        read(req.call()).map(|_| ())
    }

    /// A relay token for one document, plus where to take it.
    pub fn join(&self, id: &str) -> Result<Join> {
        parse(&self.get(&format!("/api/v1/token?id={}", enc(id)))?)
    }
}

fn read(result: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Vec<u8>> {
    let mut response = result.map_err(|e| Error::Unreachable(e.to_string()))?;
    let code = response.status().as_u16();
    let body = response.body_mut().read_to_vec().map_err(|e| Error::Unreachable(e.to_string()))?;
    if (200..300).contains(&code) {
        return Ok(body);
    }
    Err(status(code, body))
}

/// Turn a status and body into the error a caller can act on.
fn status(code: u16, body: Vec<u8>) -> Error {
    if let Ok(parsed) = serde_json::from_slice::<ApiError>(&body) {
        return match parsed.code.as_str() {
            "unauthenticated" => Error::Unauthenticated,
            "provider_not_linked" => Error::NotLinked,
            _ => Error::Refused { code: parsed.code, detail: parsed.detail },
        };
    }
    match code {
        401 => Error::Unauthenticated,
        428 => Error::NotLinked,
        _ => Error::Refused { code: code.to_string(), detail: String::new() },
    }
}

fn parse<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T> {
    serde_json::from_slice(body).map_err(|e| Error::Malformed(e.to_string()))
}

/// Percent-encode a query value. Stricter than needed and never wrong.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(*b as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
