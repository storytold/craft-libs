//! Signing in from a desktop app, through Login Flow v2.
//!
//! This is the flow the Nextcloud desktop client uses. The app asks the server
//! to begin a login and gets back a URL plus a poll token. The user opens that
//! URL in a browser and authenticates there. The app polls until the server
//! hands back an app password.
//!
//! The browser doing the authenticating is the whole point. Any login backend
//! works unchanged, OIDC through an external provider included, and the app
//! never sees the account password, never registers an OAuth client and never
//! implements PKCE. A server that later moves to passkeys needs no change here.
//!
//! What comes back is an app password, not the account password. It is scoped
//! to one device and revocable on its own from Nextcloud's security settings.
//!
//! Nothing here sleeps or opens a socket. [`LoginFlow::poll`] answers "not yet"
//! and leaves the waiting to the caller, which is what keeps this crate free of
//! a runtime and testable without a server.
//!
//! Written from Nextcloud's published Login Flow v2 documentation.

use crate::error::{Error, Result};
use crate::http::{Http, Method, Request};

/// A login the user has not finished yet.
#[derive(Clone, Debug)]
pub struct LoginFlow {
    /// Open this in a browser. The user authenticates there.
    pub login_url: String,
    /// Where to poll. Returned by the server rather than built here, because a
    /// server behind a different public name answers on its own address.
    pub endpoint: String,
    /// Identifies this attempt while polling. Not a credential for anything
    /// else, and it stops being useful once the flow completes.
    pub token: String,
}

/// What a finished login yields.
#[derive(Clone, Debug)]
pub struct AppPassword {
    /// The server the credential belongs to, as the server itself named it.
    pub server: String,
    /// The login name to authenticate as. Not necessarily the display name.
    pub user: String,
    /// Device-scoped and separately revocable. Never the account password.
    pub password: String,
}

/// Ask the server to begin a login.
///
/// `base` is the server root, such as `https://cloud.example.com`.
pub fn begin<H: Http>(http: &H, base: &str) -> Result<LoginFlow> {
    let url = format!("{}/index.php/login/v2", base.trim_end_matches('/'));
    let req = Request::new(Method::Post, url);
    let resp = crate::send(http, req)?;
    let v = json(&resp.body)?;

    let login_url = string(&v, "login")?;
    let poll = v.get("poll").ok_or_else(|| Error::malformed("login flow", "no poll object"))?;
    let endpoint = string(poll, "endpoint")?;
    let token = string(poll, "token")?;
    Ok(LoginFlow { login_url, endpoint, token })
}

impl LoginFlow {
    /// Check once whether the user has finished.
    ///
    /// `Ok(None)` means not yet, which the server reports as 404 for as long
    /// as the login is outstanding. The caller decides how long to keep asking
    /// and how long to wait between tries. Nextcloud expires a flow after
    /// about twenty minutes.
    ///
    /// The server answers exactly once with the credential. A second call
    /// after success will not return it again, so a caller that loses it has
    /// to start a new flow.
    pub fn poll<H: Http>(&self, http: &H) -> Result<Option<AppPassword>> {
        let req = Request::new(Method::Post, self.endpoint.clone())
            .header("content-type", "application/x-www-form-urlencoded")
            .body(format!("token={}", crate::encode_segment(&self.token)).into_bytes());

        // Deliberately not crate::send, which rejects a non-2xx. Here a 404 is
        // the ordinary answer rather than a failure.
        let resp = http.send(req).map_err(|e| Error::Transport(e.to_string()))?;
        if resp.status == 404 {
            return Ok(None);
        }
        if !resp.is_success() {
            return Err(Error::status(Method::Post, &self.endpoint, &resp));
        }

        let v = json(&resp.body)?;
        Ok(Some(AppPassword {
            server: string(&v, "server")?,
            user: string(&v, "loginName")?,
            password: string(&v, "appPassword")?,
        }))
    }
}

fn json(body: &[u8]) -> Result<serde_json::Value> {
    serde_json::from_slice(body).map_err(|e| Error::malformed("login flow", e.to_string()))
}

fn string(v: &serde_json::Value, key: &'static str) -> Result<String> {
    v.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::malformed("login flow", format!("no {key}")))
}

/// Mint a CraftCollab relay token for one file.
///
/// The `craftcollab` app checks the account may open that file, then signs a
/// token carrying the file id and the permission bits. The relay verifies the
/// signature locally and never calls back here, so this is needed once per
/// document rather than once per edit.
///
/// The signing key lives only on the server. A client cannot mint its own, and
/// that is deliberate: anything able to would be able to forge any file id and
/// any permissions.
pub fn collab_token<H: Http>(http: &H, account: &crate::Account, file_id: u64) -> Result<String> {
    let url = format!("{}/ocs/v2.php/apps/craftcollab/api/v1/token/{file_id}", account.base().trim_end_matches('/'));
    // JSON rather than the crate's usual XML, because the relay token is a
    // single opaque string and the OCS envelope adds nothing worth parsing.
    let req = account.request(Method::Get, url).header("accept", "application/json");
    let resp = crate::send(http, req)?;
    let v = json(&resp.body)?;
    v.pointer("/ocs/data/token")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::malformed("collab token", "no token in the OCS data"))
}
