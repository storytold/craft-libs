//! Consent: what leaves the machine, and whether the user agreed to it.
//!
//! Before a job goes to a remote connection the app shows a [`Disclosure`]: which inputs are
//! sent (by kind and size, never contents) and which parameter names ride along. Agreement is
//! recorded in a [`ConsentLedger`] per app, connection, destination and capability, for the kinds
//! of data it named, and optionally until a time. A configured connection never authorises an app
//! by itself: pointing a connection at a different host, asking for a different capability or
//! letting a timed agreement lapse all ask again. Each kind of data lapses on its own: renewing
//! text does not renew an image permission that has run out. Records saved before capabilities were scoped
//! cover nothing, so they ask once more. The clock is the caller's (unix seconds). Local
//! connections never need consent. The disclosure names exactly what this app sends, which is why
//! consent stays in the app even when the key came from the launcher.

use serde::{Deserialize, Serialize};

use crate::capability::CapabilityId;
use crate::connection::{Connection, Locality};
use crate::endpoint::Endpoint;
use crate::ids::{AppId, ConnectionId};
use crate::model::InputRole;
use crate::payload::PayloadKind;
use crate::request::Request;

/// One thing a request sends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisclosureItem {
    /// The role the input plays in the request.
    pub role: InputRole,
    /// The kind of data sent.
    pub kind: PayloadKind,
    /// Size in bytes when known.
    pub bytes: Option<u64>,
}

/// What a request will send to a connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disclosure {
    /// The app that sends the request.
    pub app: AppId,
    /// The connection the request goes to.
    pub connection: ConnectionId,
    /// What the request asks the connection to do. Consent is given per capability.
    pub capability: CapabilityId,
    /// Whether the connection is local or remote.
    pub locality: Locality,
    /// `scheme://host[:port]`, or the route name when there is no endpoint.
    pub destination: String,
    /// The inputs the request sends.
    pub items: Vec<DisclosureItem>,
    /// Parameter names only, never values.
    pub param_names: Vec<String>,
}

impl Disclosure {
    /// Builds the disclosure for sending `request` from `app` through `connection`.
    pub fn for_request(app: &AppId, connection: &Connection, request: &Request) -> Self {
        let destination = connection
            .endpoint
            .as_deref()
            .and_then(|e| Endpoint::parse(e).ok())
            .map(|e| e.fingerprint())
            .unwrap_or_else(|| connection.provider.clone());
        Self {
            app: app.clone(),
            connection: connection.id.clone(),
            capability: request.capability.clone(),
            locality: connection.locality,
            destination,
            items: request
                .inputs
                .iter()
                .map(|i| DisclosureItem { role: i.role.clone(), kind: i.payload.kind(), bytes: i.payload.size_hint() })
                .collect(),
            param_names: request.params.keys().cloned().collect(),
        }
    }

    /// The distinct kinds of data this request sends. Parameters carry values the app chose,
    /// free text among them, so a request with any counts as sending text even with no inputs.
    pub fn kinds(&self) -> Vec<PayloadKind> {
        let mut kinds: Vec<PayloadKind> = self.items.iter().map(|i| i.kind).collect();
        if !self.param_names.is_empty() {
            kinds.push(PayloadKind::Text);
        }
        kinds.sort();
        kinds.dedup();
        kinds
    }
}

/// One saved agreement: the kinds of data it covers, all lapsing together. Agreeing to a kind
/// again moves it out of any record with a different expiry, so one kind's renewal never extends
/// another's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRecord {
    /// The app that agreed.
    pub app: AppId,
    /// The connection agreed to.
    pub connection: ConnectionId,
    /// The destination agreed to. A different destination needs fresh consent.
    pub destination: String,
    /// The capability agreed to. A record from before consent was scoped to a capability has
    /// none and covers nothing, so the person is asked once more.
    #[serde(default)]
    pub capability: Option<CapabilityId>,
    /// The kinds of data the agreement covers.
    pub kinds: Vec<PayloadKind>,
    /// Unix seconds. The agreement holds while the clock is before this; `None` holds until
    /// revoked.
    #[serde(default)]
    pub expires_at: Option<u64>,
}

impl ConsentRecord {
    /// Same app, connection, destination and capability. A record with no capability (from before
    /// consent was scoped to one) matches nothing.
    fn covers(&self, d: &Disclosure) -> bool {
        self.app == d.app && self.connection == d.connection && self.destination == d.destination && self.capability.as_ref() == Some(&d.capability)
    }
}

/// An app's saved agreements.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentLedger {
    /// The saved agreements.
    #[serde(default)]
    pub records: Vec<ConsentRecord>,
}

impl ConsentLedger {
    /// True when the user must be asked before this disclosure is sent. `now` is unix seconds;
    /// the model crate has no clock, so the caller supplies it. An agreement covers one app, one
    /// connection, one destination and one capability, for the kinds of data it named, until it
    /// lapses.
    pub fn needs_consent(&self, d: &Disclosure, now: u64) -> bool {
        if d.locality == Locality::Local {
            return false;
        }
        d.kinds().iter().any(|k| !self.records.iter().any(|r| r.covers(d) && r.expires_at.is_none_or(|t| now < t) && r.kinds.contains(k)))
    }

    /// Records agreement that holds until revoked.
    pub fn grant(&mut self, d: &Disclosure) {
        self.record(d, None);
    }

    /// Like [`grant`](Self::grant), but the agreement lapses at `expires_at` (unix seconds).
    pub fn grant_until(&mut self, d: &Disclosure, expires_at: u64) {
        self.record(d, Some(expires_at));
    }

    /// Records the kinds in `d` with the given expiry for the same app, connection, destination
    /// and capability. The latest agreement sets the expiry of the kinds it names; other kinds
    /// and other capabilities keep theirs.
    fn record(&mut self, d: &Disclosure, expires_at: Option<u64>) {
        let kinds = d.kinds();
        for r in self.records.iter_mut().filter(|r| r.covers(d) && r.expires_at != expires_at) {
            r.kinds.retain(|k| !kinds.contains(k));
        }
        self.records.retain(|r| !r.kinds.is_empty());
        if let Some(r) = self.records.iter_mut().find(|r| r.covers(d) && r.expires_at == expires_at) {
            for k in kinds {
                if !r.kinds.contains(&k) {
                    r.kinds.push(k);
                }
            }
            r.kinds.sort();
        } else {
            self.records.push(ConsentRecord {
                app: d.app.clone(),
                connection: d.connection.clone(),
                destination: d.destination.clone(),
                capability: Some(d.capability.clone()),
                kinds,
                expires_at,
            });
        }
    }

    /// Removes every agreement the app has for the connection, whatever the destination or capability.
    pub fn revoke(&mut self, app: &AppId, connection: &ConnectionId) {
        self.records.retain(|r| !(&r.app == app && &r.connection == connection));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilityId;
    use crate::connection::fixtures::remote;
    use crate::payload::Payload;

    fn app() -> AppId {
        AppId::new("photocraft").unwrap()
    }

    fn req() -> Request {
        Request::new(CapabilityId::parse("image.inpaint").unwrap())
            .with_input(InputRole::Source, Payload::inline("image/png", vec![0; 10]).unwrap())
            .with_input(InputRole::Prompt, Payload::text("a cat"))
            .with_param("seed", 7.into())
    }

    #[test]
    fn a_new_kind_of_data_asks_again() {
        let mut ledger = ConsentLedger::default();
        ledger.grant(&Disclosure::for_request(&app(), &remote("a"), &req()));
        let with_video = req().with_input(InputRole::Context, Payload::handle("file:///v.mp4", "video/mp4", Some(5)).unwrap());
        let d = Disclosure::for_request(&app(), &remote("a"), &with_video);
        assert!(ledger.needs_consent(&d, 0));
        ledger.grant(&d);
        assert!(!ledger.needs_consent(&d, 0));
        assert_eq!(ledger.records.len(), 1);
    }

    fn disclosure_for(r: &Request) -> Disclosure {
        Disclosure::for_request(&app(), &remote("a"), r)
    }

    fn text_only_req() -> Request {
        Request::new(CapabilityId::parse("image.inpaint").unwrap()).with_input(InputRole::Prompt, Payload::text("a cat"))
    }

    #[test]
    fn renewing_text_does_not_renew_an_expired_image_permission() {
        let mut ledger = ConsentLedger::default();
        ledger.grant_until(&disclosure_for(&req()), 1_000);
        let text = disclosure_for(&text_only_req());
        assert!(ledger.needs_consent(&text, 2_000));
        ledger.grant_until(&text, 5_000);
        assert!(!ledger.needs_consent(&text, 2_000));
        assert!(ledger.needs_consent(&disclosure_for(&req()), 2_000), "image consent lapsed at 1000 and was not renewed");
    }
}
