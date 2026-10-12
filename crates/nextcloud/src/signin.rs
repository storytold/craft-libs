//! Login Flow v2: the user approves the app in a browser and the server hands back an app password.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rustls::crypto::CryptoProvider;
use serde::Deserialize;

use crate::account::normalize_server;
use crate::http::{Http, Kind, read_capped};
use crate::{Account, Error, Fingerprint};

/// How long the user has to approve the sign-in once the login page is shown.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const STALL: Duration = Duration::from_secs(30);
/// Form-encoding leaves only the RFC 3986 unreserved characters alone.
const FORM: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');
const MAX_RESPONSE: u64 = 1 << 20;

/// Where a sign-in stands.
pub enum SignInState {
    /// The server's certificate is untrusted; show the fingerprint and call [`SignIn::confirm_trust`].
    NeedsTrust {
        fingerprint: Fingerprint,
    },
    /// Open `login_url` in a browser and call [`SignIn::poll`] every couple of seconds.
    Waiting {
        login_url: String,
    },
    /// Signed in. Store `app_password` in the keyring.
    Done {
        account: Account,
        app_password: String,
    },
    Failed(Error),
}

impl fmt::Debug for SignInState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignInState::NeedsTrust { fingerprint } => f.debug_struct("NeedsTrust").field("fingerprint", fingerprint).finish(),
            SignInState::Waiting { login_url } => f.debug_struct("Waiting").field("login_url", login_url).finish(),
            SignInState::Done { account, .. } => f.debug_struct("Done").field("account", account).field("app_password", &"<redacted>").finish(),
            SignInState::Failed(e) => f.debug_tuple("Failed").field(e).finish(),
        }
    }
}

/// One Login Flow v2 sign-in. Methods block on the network, so call them off the UI thread.
///
/// `ureq` and `ureq_proto` log raw HTTP bytes at TRACE, including the app password in the poll's reply: cap those log
/// targets below TRACE.
pub struct SignIn {
    state: SignInState,
    account: Account,
    /// The User-Agent of every request.
    client_name: String,
    provider: Arc<CryptoProvider>,
    http: Option<Http>,
    /// Poll endpoint and token, once the flow has started.
    poll: Option<(String, String)>,
    deadline: Instant,
}

impl fmt::Debug for SignIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignIn").field("state", &self.state).finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct Start {
    poll: StartPoll,
    login: String,
}

#[derive(Deserialize)]
struct StartPoll {
    token: String,
    endpoint: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Done {
    login_name: String,
    app_password: String,
}

fn malformed(what: &str) -> Error {
    Error::Malformed(what.to_string())
}

/// `scheme://host[:port]` of a URL the server sent, normalized.
fn origin(url: &str) -> Result<String, Error> {
    let url = normalize_server(url).map_err(|_| malformed("bad poll endpoint"))?;
    let (scheme, rest) = url.split_once("://").ok_or_else(|| malformed("bad URL"))?;
    Ok(format!("{scheme}://{}", rest.split('/').next().unwrap_or(rest)))
}

impl SignIn {
    /// Starts signing in to `server` (as typed by the user). `client_name` (e.g. "LightCraft (Linux)") is sent as the
    /// User-Agent, which Nextcloud shows as the app password's name; an empty one or one with control characters
    /// becomes "Craft". Never panics; a bad address is `Failed(InvalidAddress)`. Blocks on the first request.
    pub fn start(server: &str, client_name: &str, provider: Arc<CryptoProvider>) -> SignIn {
        SignIn::new(Account { server: server.to_string(), user: String::new(), trust: None }, client_name, provider)
    }

    /// Signs in again to `account`'s server (e.g. after `Unauthorized`), keeping its trusted certificate: a different
    /// certificate ends as `Failed(CertificateChanged)`, with no trust prompt. Otherwise like [`SignIn::start`].
    pub fn start_for(account: &Account, client_name: &str, provider: Arc<CryptoProvider>) -> SignIn {
        SignIn::new(account.clone(), client_name, provider)
    }

    fn new(account: Account, client_name: &str, provider: Arc<CryptoProvider>) -> SignIn {
        let mut sign_in = SignIn {
            state: SignInState::Failed(Error::Cancelled),
            account,
            // A name that can't be a header value must not block the sign-in.
            client_name: if client_name.is_empty() || client_name.contains(char::is_control) { "Craft" } else { client_name }.to_string(),
            provider,
            http: None,
            poll: None,
            deadline: Instant::now() + SIGN_IN_TIMEOUT,
        };
        sign_in.state = match normalize_server(&sign_in.account.server) {
            Ok(server) => {
                sign_in.account.server = server;
                sign_in.begin(true)
            }
            Err(e) => SignInState::Failed(e),
        };
        sign_in
    }

    pub fn state(&self) -> &SignInState {
        &self.state
    }

    /// Answers [`SignInState::NeedsTrust`]: `false` cancels, `true` trusts the certificate and starts the flow.
    pub fn confirm_trust(&mut self, accept: bool) -> &SignInState {
        if let SignInState::NeedsTrust { fingerprint } = self.state {
            self.state = if accept {
                self.account.trust = Some(fingerprint);
                self.begin(false)
            } else {
                SignInState::Failed(Error::Cancelled)
            };
        }
        &self.state
    }

    /// Asks the server once whether the user has approved. Only does anything in [`SignInState::Waiting`].
    pub fn poll(&mut self) -> &SignInState {
        if matches!(self.state, SignInState::Waiting { .. }) {
            self.state = self.poll_once();
        }
        &self.state
    }

    /// Gives up, unless already done.
    pub fn cancel(&mut self) {
        if !matches!(self.state, SignInState::Done { .. }) {
            self.state = SignInState::Failed(Error::Cancelled);
        }
    }

    #[doc(hidden)]
    pub fn set_deadline_for_test(&mut self, deadline: Instant) {
        self.deadline = deadline;
    }

    /// The start request. `ask_trust`: an untrusted certificate becomes `NeedsTrust` rather than a failure.
    fn begin(&mut self, ask_trust: bool) -> SignInState {
        match self.try_begin() {
            Ok(state) => state,
            Err(Error::UnknownCertificate { fingerprint }) if ask_trust => SignInState::NeedsTrust { fingerprint },
            Err(e) => SignInState::Failed(e),
        }
    }

    fn try_begin(&mut self) -> Result<SignInState, Error> {
        let http = Http::new(&self.account, None, self.provider.clone(), STALL)?;
        let url = format!("{}/index.php/login/v2", http.base());
        let headers = [("Accept", "application/json"), ("User-Agent", self.client_name.as_str())];
        let start: Start = json(&http, &url, &headers, b"")?.ok_or_else(|| malformed("login flow not found"))?;
        if origin(&start.poll.endpoint)? != origin(http.base())? {
            return Err(malformed("poll endpoint is on another server"));
        }
        if !(start.login.starts_with("https://") || start.login.starts_with("http://")) {
            return Err(malformed("login URL is not http(s)"));
        }
        self.poll = Some((start.poll.endpoint, start.poll.token));
        self.http = Some(http);
        self.deadline = Instant::now() + SIGN_IN_TIMEOUT;
        Ok(SignInState::Waiting { login_url: start.login })
    }

    fn poll_once(&mut self) -> SignInState {
        let waiting = |this: &SignIn| SignInState::Waiting {
            login_url: match &this.state {
                SignInState::Waiting { login_url } => login_url.clone(),
                _ => String::new(),
            },
        };
        if Instant::now() >= self.deadline {
            return SignInState::Failed(Error::Cancelled);
        }
        let (Some(http), Some((endpoint, token))) = (&self.http, &self.poll) else { return SignInState::Failed(Error::Cancelled) };
        let body = format!("token={}", utf8_percent_encode(token, FORM));
        let headers =
            [("Accept", "application/json"), ("Content-Type", "application/x-www-form-urlencoded"), ("User-Agent", self.client_name.as_str())];
        match json::<Done>(http, endpoint, &headers, body.as_bytes()) {
            Ok(None) | Err(Error::Unreachable(_) | Error::RateLimited) => waiting(self),
            Ok(Some(done)) => self.finish(done),
            Err(e) => SignInState::Failed(e),
        }
    }

    fn finish(&self, done: Done) -> SignInState {
        let name = &done.login_name;
        if name.is_empty() || name.contains(|c: char| c == '/' || c.is_control()) {
            return SignInState::Failed(malformed("bad login name"));
        }
        let account = Account { user: done.login_name, ..self.account.clone() };
        SignInState::Done { account, app_password: done.app_password }
    }
}

/// POSTs `body` and parses a JSON reply; a 404 is `None`. Parse errors never quote the response.
fn json<T: for<'de> Deserialize<'de>>(http: &Http, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Option<T>, Error> {
    let res = match http.request("POST", url, headers, Some(body), Kind::Quick) {
        Err(Error::NotFound) => return Ok(None),
        res => res?,
    };
    if res.status().as_u16() != 200 {
        return Err(malformed("unexpected status"));
    }
    let bytes = read_capped(res.into_body(), MAX_RESPONSE)?;
    serde_json::from_slice(&bytes).map(Some).map_err(|_| malformed("unreadable response"))
}
