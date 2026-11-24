//! The JMAP session object (RFC 8620 §2), read with bounds.
//!
//! Everything in it is treated as hostile: counts and string lengths are bounded, ids are checked (§1.2),
//! addresses are resolved against the session address and must pass the transport rule and the loopback
//! check ([`Address::check_link`]), templates are checked once here (see [`crate::template`]), and the core
//! limits get safe defaults and caps ([`CoreLimits`]). Properties the crate does not understand are ignored,
//! as §2 requires; capability objects are kept as JSON for the caller.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Map, Value};

use crate::address::Address;
use crate::classify::parse_json;
use crate::error::{Error, Unusable};
use crate::id::Id;
use crate::template::AddressTemplate;

/// The core capability (RFC 8620 §2).
pub const CAPABILITY_CORE: &str = "urn:ietf:params:jmap:core";
/// The mail capability (RFC 8621 §1.3.1).
pub const CAPABILITY_MAIL: &str = "urn:ietf:params:jmap:mail";

/// The most accounts a session may list.
pub const MAX_ACCOUNTS: usize = 1024;
/// The most capabilities a session, or one account, may list.
pub const MAX_CAPABILITIES: usize = 256;
/// The longest capability identifier kept.
pub const MAX_CAPABILITY_LEN: usize = 256;
/// The longest state string accepted (RFC 8620 §2 `state`, §3.4 `sessionState`).
pub const MAX_STATE_LEN: usize = 1024;
/// The longest user name or account name accepted.
pub const MAX_NAME_LEN: usize = 1024;

/// The largest UnsignedInt (RFC 8620 §1.3).
const MAX_UNSIGNED_INT: u64 = 9_007_199_254_740_991;

/// Variables of `downloadUrl` (RFC 8620 §2, §6.2).
const DOWNLOAD_VARIABLES: [&str; 4] = ["accountId", "blobId", "type", "name"];
/// Variables of `uploadUrl` (RFC 8620 §2, §6.1).
const UPLOAD_VARIABLES: [&str; 1] = ["accountId"];
/// Variables of `eventSourceUrl` (RFC 8620 §2, §7.3).
const EVENT_SOURCE_VARIABLES: [&str; 3] = ["types", "closeafter", "ping"];

/// The limits of the core capability (RFC 8620 §2), each defaulted when missing or not an UnsignedInt, and
/// clamped between 1 and a cap so that an absurd value cannot drive the client into absurd work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoreLimits {
    max_size_upload: u64,
    max_concurrent_upload: u64,
    max_size_request: u64,
    max_concurrent_requests: u64,
    max_calls_in_request: u64,
    max_objects_in_get: u64,
    max_objects_in_set: u64,
}

/// A limit's name, its default (the RFC's suggested minimum) and its cap.
struct LimitRule {
    name: &'static str,
    default: u64,
    cap: u64,
}

const SIZE_UPLOAD: LimitRule = LimitRule { name: "maxSizeUpload", default: 50_000_000, cap: 1_099_511_627_776 };
const CONCURRENT_UPLOAD: LimitRule = LimitRule { name: "maxConcurrentUpload", default: 4, cap: 64 };
const SIZE_REQUEST: LimitRule = LimitRule { name: "maxSizeRequest", default: 10_000_000, cap: 1_073_741_824 };
const CONCURRENT_REQUESTS: LimitRule = LimitRule { name: "maxConcurrentRequests", default: 4, cap: 64 };
const CALLS_IN_REQUEST: LimitRule = LimitRule { name: "maxCallsInRequest", default: 16, cap: 1024 };
const OBJECTS_IN_GET: LimitRule = LimitRule { name: "maxObjectsInGet", default: 500, cap: 100_000 };
const OBJECTS_IN_SET: LimitRule = LimitRule { name: "maxObjectsInSet", default: 500, cap: 100_000 };

/// An UnsignedInt (RFC 8620 §1.3), accepting a JSON number written with a fraction or exponent when its
/// value is a whole number.
///
/// A whole number above the UnsignedInt range is read as the largest value, so that an absurdly large limit
/// is capped like any other large one rather than replaced by the default.
fn unsigned_int(value: &Value) -> Option<u64> {
    let number = value.as_number()?;
    if let Some(whole) = number.as_u64() {
        return Some(whole.min(MAX_UNSIGNED_INT));
    }
    let float = number.as_f64()?;
    if !(float.is_finite() && float >= 0.0 && float.fract() == 0.0) {
        return None;
    }
    if float > 9_007_199_254_740_991.0 {
        return Some(MAX_UNSIGNED_INT);
    }
    // Whole, non-negative and below 2^53: the conversion is exact.
    Some(float as u64)
}

impl CoreLimits {
    fn read(object: &Map<String, Value>, rule: &LimitRule) -> u64 {
        object.get(rule.name).and_then(unsigned_int).unwrap_or(rule.default).clamp(1, rule.cap)
    }

    fn from_capability(object: &Map<String, Value>) -> Self {
        // RFC 8620 §2.1's example spells the concurrency limit without its final "s"; accept both spellings.
        let concurrent_requests = match object.get(CONCURRENT_REQUESTS.name) {
            Some(_) => Self::read(object, &CONCURRENT_REQUESTS),
            None => Self::read(object, &LimitRule { name: "maxConcurrentRequest", ..CONCURRENT_REQUESTS }),
        };
        Self {
            max_size_upload: Self::read(object, &SIZE_UPLOAD),
            max_concurrent_upload: Self::read(object, &CONCURRENT_UPLOAD),
            max_size_request: Self::read(object, &SIZE_REQUEST),
            max_concurrent_requests: concurrent_requests,
            max_calls_in_request: Self::read(object, &CALLS_IN_REQUEST),
            max_objects_in_get: Self::read(object, &OBJECTS_IN_GET),
            max_objects_in_set: Self::read(object, &OBJECTS_IN_SET),
        }
    }

    /// `maxSizeUpload`, octets (default 50 000 000, cap 2^40).
    pub fn max_size_upload(&self) -> u64 {
        self.max_size_upload
    }

    /// `maxConcurrentUpload` (default 4, cap 64).
    pub fn max_concurrent_upload(&self) -> u64 {
        self.max_concurrent_upload
    }

    /// `maxSizeRequest`, octets (default 10 000 000, cap 2^30).
    pub fn max_size_request(&self) -> u64 {
        self.max_size_request
    }

    /// `maxConcurrentRequests` (default 4, cap 64).
    pub fn max_concurrent_requests(&self) -> u64 {
        self.max_concurrent_requests
    }

    /// `maxCallsInRequest` (default 16, cap 1024).
    pub fn max_calls_in_request(&self) -> u64 {
        self.max_calls_in_request
    }

    /// `maxObjectsInGet` (default 500, cap 100 000).
    pub fn max_objects_in_get(&self) -> u64 {
        self.max_objects_in_get
    }

    /// `maxObjectsInSet` (default 500, cap 100 000).
    pub fn max_objects_in_set(&self) -> u64 {
        self.max_objects_in_set
    }
}

impl Default for CoreLimits {
    /// The defaults used for every limit a server omits.
    fn default() -> Self {
        Self::from_capability(&Map::new())
    }
}

/// An account the credentials give access to (RFC 8620 §1.6.2, §2).
#[derive(Clone, PartialEq)]
pub struct Account {
    name: String,
    personal: bool,
    read_only: bool,
    capabilities: BTreeMap<String, Value>,
}

impl Account {
    /// The name to show for the account (often an e-mail address: personal data).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `isPersonal`: whether the account belongs to the authenticated user.
    pub fn is_personal(&self) -> bool {
        self.personal
    }

    /// `isReadOnly`: whether the whole account is read-only.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Whether `accountCapabilities` lists `capability`.
    pub fn has_capability(&self, capability: &str) -> bool {
        self.capabilities.contains_key(capability)
    }

    /// The account-level object of a capability.
    pub fn capability(&self, capability: &str) -> Option<&Value> {
        self.capabilities.get(capability)
    }
}

impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account")
            .field("personal", &self.personal)
            .field("read_only", &self.read_only)
            .field("capabilities", &self.capabilities.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// A parsed session object.
#[derive(Clone, PartialEq)]
pub struct Session {
    address: Address,
    capabilities: BTreeMap<String, Value>,
    core: CoreLimits,
    accounts: BTreeMap<Id, Account>,
    primary_accounts: BTreeMap<String, Id>,
    username: String,
    api: Address,
    download: AddressTemplate,
    upload: AddressTemplate,
    event_source: Option<AddressTemplate>,
    state: String,
}

fn shape(what: &'static str) -> Error {
    Error::Unusable(Unusable::Shape(what))
}

fn is_capability_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_CAPABILITY_LEN && name.bytes().all(|b| (0x21..=0x7E).contains(&b))
}

/// Reads a capabilities object; badly named entries are skipped, too many is an error.
fn read_capabilities(value: &Value, what: &'static str) -> Result<BTreeMap<String, Value>, Error> {
    let object = value.as_object().ok_or_else(|| shape(what))?;
    if object.len() > MAX_CAPABILITIES {
        return Err(shape("the session lists too many capabilities"));
    }
    Ok(object.iter().filter(|(name, _)| is_capability_name(name)).map(|(name, value)| (name.clone(), value.clone())).collect())
}

fn bounded_string(object: &Map<String, Value>, key: &str, max: usize, what: &'static str) -> Result<Option<String>, Error> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.len() <= max => Ok(Some(text.clone())),
        Some(_) => Err(shape(what)),
    }
}

fn read_account(value: &Value) -> Result<Account, Error> {
    let object = value.as_object().ok_or_else(|| shape("an account is not an object"))?;
    let name = bounded_string(object, "name", MAX_NAME_LEN, "an account name is not a short string")?.unwrap_or_default();
    let capabilities = match object.get("accountCapabilities") {
        Some(value) => read_capabilities(value, "accountCapabilities is not an object")?,
        None => BTreeMap::new(),
    };
    Ok(Account {
        name,
        personal: object.get("isPersonal").and_then(Value::as_bool).unwrap_or(false),
        read_only: object.get("isReadOnly").and_then(Value::as_bool).unwrap_or(false),
        capabilities,
    })
}

/// Probe values used to check a template once, when the session is read.
const PROBE: [(&str, &str); 7] = [
    ("accountId", "probe"),
    ("blobId", "probe"),
    ("type", "application/octet-stream"),
    ("name", "probe"),
    ("types", "*"),
    ("closeafter", "no"),
    ("ping", "0"),
];

fn read_template(
    object: &Map<String, Value>,
    key: &'static str,
    base: &Address,
    required: &[&'static str],
    allowed: &[&str],
) -> Result<AddressTemplate, Error> {
    let text = object.get(key).and_then(Value::as_str).ok_or_else(|| shape("an address template is missing"))?;
    let template = AddressTemplate::parse(text, base).map_err(|error| Error::Unusable(Unusable::Template(key, error)))?;
    template.check_variables(required, allowed).map_err(|error| Error::Unusable(Unusable::Template(key, error)))?;
    template.expand(&PROBE).map_err(|error| Error::Unusable(Unusable::Template(key, error)))?;
    Ok(template)
}

impl Session {
    /// Reads a session object fetched from `address` (RFC 8620 §2).
    ///
    /// Required: `capabilities` with `urn:ietf:params:jmap:core`, `accounts`, `apiUrl`, `downloadUrl` (with
    /// at least `accountId` and `blobId`), `uploadUrl` (with `accountId`) and `state`. `primaryAccounts` and
    /// `username` default to empty. An `eventSourceUrl` that is missing or unusable is dropped, push being
    /// optional for a mail client. An account whose id is not a valid id, or whose object is malformed (not an
    /// object, an overlong name, a capabilities member that is not an object), is skipped; so are
    /// primary-account entries naming an invalid id.
    pub fn parse(body: &[u8], address: &Address) -> Result<Self, Error> {
        address.require_secure_transport()?;
        let value = parse_json(body)?;
        let object = value.as_object().ok_or_else(|| shape("the session is not a JSON object"))?;

        let capabilities =
            read_capabilities(object.get("capabilities").ok_or_else(|| shape("the session has no capabilities"))?, "capabilities is not an object")?;
        let core = capabilities
            .get(CAPABILITY_CORE)
            .and_then(Value::as_object)
            .map(CoreLimits::from_capability)
            .ok_or_else(|| shape("the session lacks the core capability"))?;

        let accounts_object = object.get("accounts").and_then(Value::as_object).ok_or_else(|| shape("the session has no accounts object"))?;
        if accounts_object.len() > MAX_ACCOUNTS {
            return Err(shape("the session lists too many accounts"));
        }
        let mut accounts = BTreeMap::new();
        for (id, account) in accounts_object {
            // An account that cannot be read cannot be used; the others still can.
            if let (Ok(id), Ok(account)) = (Id::new(id), read_account(account)) {
                accounts.insert(id, account);
            }
        }

        let mut primary_accounts = BTreeMap::new();
        if let Some(primary) = object.get("primaryAccounts") {
            let primary = primary.as_object().ok_or_else(|| shape("primaryAccounts is not an object"))?;
            if primary.len() > MAX_CAPABILITIES {
                return Err(shape("the session lists too many primary accounts"));
            }
            for (capability, id) in primary {
                if let (true, Some(Ok(id))) = (is_capability_name(capability), id.as_str().map(Id::new)) {
                    primary_accounts.insert(capability.clone(), id);
                }
            }
        }

        let username = bounded_string(object, "username", MAX_NAME_LEN, "username is not a short string")?.unwrap_or_default();
        let state =
            bounded_string(object, "state", MAX_STATE_LEN, "state is not a short string")?.ok_or_else(|| shape("the session has no state"))?;

        let api_text = object.get("apiUrl").and_then(Value::as_str).ok_or_else(|| shape("the session has no apiUrl"))?;
        let api = address.resolve(api_text).map_err(|error| Error::Unusable(Unusable::Address("apiUrl", error)))?;
        address.check_link(&api).map_err(|error| Error::Unusable(Unusable::Address("apiUrl", error)))?;

        let download = read_template(object, "downloadUrl", address, &["accountId", "blobId"], &DOWNLOAD_VARIABLES)?;
        let upload = read_template(object, "uploadUrl", address, &UPLOAD_VARIABLES, &UPLOAD_VARIABLES)?;
        let event_source = read_template(object, "eventSourceUrl", address, &EVENT_SOURCE_VARIABLES, &EVENT_SOURCE_VARIABLES).ok();

        Ok(Self { address: address.clone(), capabilities, core, accounts, primary_accounts, username, api, download, upload, event_source, state })
    }

    /// The address the session was fetched from.
    pub fn address(&self) -> &Address {
        &self.address
    }

    /// Whether the server advertises `capability`.
    pub fn has_capability(&self, capability: &str) -> bool {
        self.capabilities.contains_key(capability)
    }

    /// The server-level object of a capability.
    pub fn capability(&self, capability: &str) -> Option<&Value> {
        self.capabilities.get(capability)
    }

    /// The capability identifiers the server advertises.
    pub fn capabilities(&self) -> impl Iterator<Item = &str> {
        self.capabilities.keys().map(String::as_str)
    }

    /// The core limits, defaulted and capped.
    pub fn core_limits(&self) -> &CoreLimits {
        &self.core
    }

    /// The accounts, by id.
    pub fn accounts(&self) -> impl Iterator<Item = (&Id, &Account)> {
        self.accounts.iter()
    }

    /// One account.
    pub fn account(&self, id: &Id) -> Option<&Account> {
        self.accounts.get(id)
    }

    /// The primary account for a capability, when the server names one that is listed in `accounts`.
    pub fn primary_account(&self, capability: &str) -> Option<&Id> {
        self.primary_accounts.get(capability).filter(|id| self.accounts.contains_key(*id))
    }

    /// The account for mail.
    ///
    /// When the server names a primary account for `urn:ietf:params:jmap:mail`, that account, provided it is
    /// listed and offers mail; otherwise none (RFC 8620 §2: the server named a default, and it is not usable).
    /// When the server names none, the only personal account offering mail, if exactly one does.
    pub fn mail_account(&self) -> Option<&Id> {
        if let Some(id) = self.primary_accounts.get(CAPABILITY_MAIL) {
            let usable = self.accounts.get(id).is_some_and(|account| account.has_capability(CAPABILITY_MAIL));
            return usable.then_some(id);
        }
        let mut with_mail = self.accounts.iter().filter(|(_, account)| account.is_personal() && account.has_capability(CAPABILITY_MAIL));
        match (with_mail.next(), with_mail.next()) {
            (Some((id, _)), None) => Some(id),
            _ => None,
        }
    }

    /// The user name the server associates with the credentials (personal data; may be empty).
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The API endpoint (`apiUrl`), resolved and checked.
    pub fn api_address(&self) -> &Address {
        &self.api
    }

    /// The download template (`downloadUrl`).
    pub fn download_template(&self) -> &AddressTemplate {
        &self.download
    }

    /// The upload template (`uploadUrl`).
    pub fn upload_template(&self) -> &AddressTemplate {
        &self.upload
    }

    /// The push template (`eventSourceUrl`), when the server gave a usable one.
    pub fn event_source_template(&self) -> Option<&AddressTemplate> {
        self.event_source.as_ref()
    }

    /// The session state (`state`).
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Whether the API, download and upload endpoints all share the session address's origin. The
    /// credentials go to every endpoint the session names; a mail client may want to confirm with the person
    /// before sending them to another origin.
    pub fn endpoints_share_origin(&self) -> bool {
        let probe = |template: &AddressTemplate| template.expand(&PROBE).is_ok_and(|address| address.same_origin(&self.address));
        self.api.same_origin(&self.address) && probe(&self.download) && probe(&self.upload)
    }
}

impl fmt::Debug for Session {
    /// Origins, counts, capability names and state: no user name, no account names, no paths.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("origin", &self.address.origin())
            .field("api", &self.api.origin())
            .field("capabilities", &self.capabilities.keys().collect::<Vec<_>>())
            .field("accounts", &self.accounts.len())
            .field("core", &self.core)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
pub(crate) mod tests;
