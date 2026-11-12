//! Fetching the session and talking to the API through the program's HTTP exchange.
//!
//! Each operation exists in two forms: a synchronous pair (build the [`HttpRequest`], then read the
//! [`HttpResponse`]) for programs that drive the exchange themselves, and an `async` helper that does both
//! through an [`HttpExchange`]. Neither follows redirects: a redirect comes back as
//! [`Error::Redirected`] with a checked target, and the caller decides.

use std::fmt;

use crate::address::Address;
use crate::classify::judge_response;
use crate::credentials::Credentials;
use crate::error::{BuildError, Error};
use crate::http::{HttpExchange, HttpRequest, HttpResponse, Method, RequestKind};
use crate::id::Id;
use crate::redact::{Excerpt, Redactor};
use crate::request::ApiRequest;
use crate::response::ApiResponse;
use crate::session::{Account, CAPABILITY_MAIL, CoreLimits, Session};

/// A redactor knowing the credentials and the session address.
fn base_redactor(address: &Address, credentials: &Credentials) -> Redactor {
    let mut redactor = Redactor::new();
    credentials.register_secrets(&mut redactor);
    redactor.add_personal(address.as_str());
    if let Some(query) = address.query() {
        redactor.add_personal(query);
        // A query parameter may carry a token: each value, as written and decoded.
        for pair in query.split('&') {
            let value = pair.split_once('=').map_or(pair, |(_, value)| value);
            redactor.add_personal(value);
            if let Ok(decoded) = crate::percent::decode(value) {
                let decoded = crate::credentials::Secret::new(decoded);
                redactor.add_personal(decoded.expose());
            }
        }
    }
    redactor
}

/// Builds the request that fetches the session object (RFC 8620 §2). The address must satisfy the transport
/// rule, since the credentials travel with the request.
pub fn session_request(address: &Address, credentials: &Credentials) -> Result<HttpRequest, Error> {
    address.require_secure_transport()?;
    Ok(HttpRequest::new(Method::Get, address.clone(), RequestKind::Session, credentials.authorization()))
}

/// Reads the response to a [`session_request`].
pub fn read_session(request: &HttpRequest, response: &HttpResponse, credentials: &Credentials) -> Result<Session, Error> {
    let redactor = base_redactor(request.address(), credentials);
    judge_response(request, response, &redactor)?;
    Session::parse(response.body(), request.address())
}

/// Fetches and reads the session object through `exchange`.
pub async fn fetch_session<E: HttpExchange>(exchange: &E, address: &Address, credentials: &Credentials) -> Result<Session, Error> {
    let request = session_request(address, credentials)?;
    let response = exchange.exchange(&request).await?;
    read_session(&request, &response, credentials)
}

/// A JMAP client for one session and one mail account.
#[derive(Clone)]
pub struct Client {
    session: Session,
    credentials: Credentials,
    account: Id,
    redactor: Redactor,
}

impl Client {
    /// A client for the session's mail account ([`Session::mail_account`]).
    pub fn new(session: Session, credentials: Credentials) -> Result<Self, Error> {
        let account = session.mail_account().cloned().ok_or(Error::NoMailAccount)?;
        Self::for_account(session, credentials, account)
    }

    /// A client for a chosen account of the session.
    pub fn for_account(session: Session, credentials: Credentials, account: Id) -> Result<Self, Error> {
        let account_name = session.account(&account).ok_or(Error::NoMailAccount)?.name().to_owned();
        let mut redactor = base_redactor(session.address(), &credentials);
        redactor.add_personal(session.username());
        redactor.add_personal(&account_name);
        Ok(Self { session, credentials, account, redactor })
    }

    /// The session.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The account id in use.
    pub fn account_id(&self) -> &Id {
        &self.account
    }

    /// The account in use.
    pub fn account(&self) -> Option<&Account> {
        self.session.account(&self.account)
    }

    /// The core limits, defaulted and capped.
    pub fn limits(&self) -> &CoreLimits {
        self.session.core_limits()
    }

    /// Whether the server advertises `capability`.
    pub fn has_capability(&self, capability: &str) -> bool {
        self.session.has_capability(capability)
    }

    /// Whether the account in use offers `capability`.
    pub fn account_has_capability(&self, capability: &str) -> bool {
        self.account().is_some_and(|account| account.has_capability(capability))
    }

    /// Whether the account offers mail.
    pub fn has_mail(&self) -> bool {
        self.account_has_capability(CAPABILITY_MAIL)
    }

    /// Cleans a server's text for an error message, with every secret this client knows replaced.
    pub fn excerpt(&self, text: &str) -> Excerpt {
        self.redactor.excerpt(text)
    }

    /// The redactor holding this client's secrets.
    pub fn redactor(&self) -> &Redactor {
        &self.redactor
    }

    /// Whether `response` reports a session state other than the one this client holds (RFC 8620 §3.4): the
    /// session should then be fetched again.
    pub fn is_session_stale(&self, response: &ApiResponse) -> bool {
        response.session_state() != self.session.state()
    }

    /// The address to download a blob (RFC 8620 §6.2), with `media_type` as `type` and `name` as `name`.
    pub fn download_address(&self, blob: &Id, media_type: &str, name: &str) -> Result<Address, Error> {
        let values = [("accountId", self.account.as_str()), ("blobId", blob.as_str()), ("type", media_type), ("name", name)];
        Ok(self.session.download_template().expand(&values)?)
    }

    /// The address to upload a blob to the account (RFC 8620 §6.1).
    pub fn upload_address(&self) -> Result<Address, Error> {
        Ok(self.session.upload_template().expand(&[("accountId", self.account.as_str())])?)
    }

    /// The request that fetches the session again.
    pub fn session_request(&self) -> Result<HttpRequest, Error> {
        session_request(self.session.address(), &self.credentials)
    }

    /// The HTTP request for an API request (RFC 8620 §3.1). Checked first: the capabilities used must be
    /// advertised, the number of calls must be within `maxCallsInRequest`, and the body within
    /// `maxSizeRequest`.
    pub fn api_request(&self, request: &ApiRequest) -> Result<HttpRequest, Error> {
        if request.capabilities().any(|capability| !self.session.has_capability(capability)) {
            return Err(BuildError::UnsupportedCapability.into());
        }
        let calls = u64::try_from(request.len()).unwrap_or(u64::MAX);
        let max_calls = self.limits().max_calls_in_request();
        if calls > max_calls {
            return Err(BuildError::TooManyCalls { limit: max_calls }.into());
        }
        let body = request.to_json()?;
        let max_size = self.limits().max_size_request();
        if u64::try_from(body.len()).unwrap_or(u64::MAX) > max_size {
            return Err(BuildError::TooLarge { limit: max_size }.into());
        }
        let address = self.session.api_address().clone();
        let http = HttpRequest::new(Method::Post, address, RequestKind::Api, self.credentials.authorization()).with_json_body(body);
        Ok(match request.response_limit() {
            Some(limit) => http.with_response_limit(limit),
            None => http,
        })
    }

    /// Reads the response to an [`Client::api_request`].
    pub fn read_api_response(&self, request: &HttpRequest, response: &HttpResponse) -> Result<ApiResponse, Error> {
        judge_response(request, response, &self.redactor)?;
        ApiResponse::parse(response.body(), &self.redactor)
    }

    /// The HTTP request that downloads a blob (RFC 8620 §6.2).
    pub fn download_request(&self, blob: &Id, media_type: &str, name: &str) -> Result<HttpRequest, Error> {
        let address = self.download_address(blob, media_type, name)?;
        Ok(HttpRequest::new(Method::Get, address, RequestKind::Download, self.credentials.authorization()))
    }

    /// Reads the response to a [`Client::download_request`] and hands back the blob's bytes.
    pub fn read_download(&self, request: &HttpRequest, response: HttpResponse) -> Result<Vec<u8>, Error> {
        judge_response(request, &response, &self.redactor)?;
        Ok(response.into_body())
    }

    /// Sends an API request through `exchange` and reads the response.
    pub async fn call<E: HttpExchange>(&self, exchange: &E, request: &ApiRequest) -> Result<ApiResponse, Error> {
        let http = self.api_request(request)?;
        let response = exchange.exchange(&http).await?;
        self.read_api_response(&http, &response)
    }

    /// Downloads a blob through `exchange`.
    ///
    /// `max_bytes`, when given, lowers the largest body accepted for this download below
    /// [`DOWNLOAD_RESPONSE_LIMIT`](crate::http::DOWNLOAD_RESPONSE_LIMIT); the exchange sees it before
    /// reading the body.
    pub async fn download<E: HttpExchange>(
        &self,
        exchange: &E,
        blob: &Id,
        media_type: &str,
        name: &str,
        max_bytes: Option<usize>,
    ) -> Result<Vec<u8>, Error> {
        let mut http = self.download_request(blob, media_type, name)?;
        if let Some(limit) = max_bytes {
            http = http.with_response_limit(limit);
        }
        let response = exchange.exchange(&http).await?;
        self.read_download(&http, response)
    }

    /// Fetches the session again through `exchange`, for when [`Client::is_session_stale`] says so.
    pub async fn refetch_session<E: HttpExchange>(&self, exchange: &E) -> Result<Session, Error> {
        fetch_session(exchange, self.session.address(), &self.credentials).await
    }
}

impl fmt::Debug for Client {
    /// Only what says nothing about the person: the authentication scheme, how many capabilities the
    /// server offers, and the core limits. No account id, no address or origin (a personal domain names its
    /// owner), no user name, no account name, no secret.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("scheme", &self.credentials.scheme_name())
            .field("capabilities", &self.session.capabilities().count())
            .field("limits", self.session.core_limits())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
