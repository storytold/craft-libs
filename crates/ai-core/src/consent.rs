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
    pub role: InputRole,
    pub kind: PayloadKind,
    pub bytes: Option<u64>,
}

/// What a request will send to a connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disclosure {
    pub app: AppId,
    pub connection: ConnectionId,
    /// What the request asks the connection to do. Consent is given per capability.
    pub capability: CapabilityId,
    pub locality: Locality,
    /// `scheme://host[:port]`, or the route name when there is no endpoint.
    pub destination: String,
    pub items: Vec<DisclosureItem>,
    /// Parameter names only, never values.
    pub param_names: Vec<String>,
}

impl Disclosure {
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
    pub app: AppId,
    pub connection: ConnectionId,
    /// The destination agreed to. A different destination needs fresh consent.
    pub destination: String,
    /// The capability agreed to. A record from before consent was scoped to a capability has
    /// none and covers nothing, so the person is asked once more.
    #[serde(default)]
    pub capability: Option<CapabilityId>,
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

    pub fn revoke(&mut self, app: &AppId, connection: &ConnectionId) {
        self.records.retain(|r| !(&r.app == app && &r.connection == connection));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilityId;
    use crate::connection::fixtures::{local, remote};
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
    fn disclosure_lists_kinds_sizes_and_param_names_not_contents() {
        let d = Disclosure::for_request(&app(), &remote("a"), &req());
        assert_eq!(d.destination, "https://api.example.com");
        assert_eq!(d.kinds(), vec![PayloadKind::Text, PayloadKind::Image]);
        assert_eq!(d.items.first().and_then(|i| i.bytes), Some(10));
        assert_eq!(d.param_names, vec!["seed".to_string()]);
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("a cat"));
    }

    #[test]
    fn local_connections_never_ask() {
        let d = Disclosure::for_request(&app(), &local("a"), &req());
        assert!(!ConsentLedger::default().needs_consent(&d, 0));
    }

    #[test]
    fn remote_asks_until_granted_then_stops() {
        let d = Disclosure::for_request(&app(), &remote("a"), &req());
        let mut ledger = ConsentLedger::default();
        assert!(ledger.needs_consent(&d, 0));
        ledger.grant(&d);
        assert!(!ledger.needs_consent(&d, 0));
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

    #[test]
    fn changing_the_host_invalidates_consent() {
        let mut ledger = ConsentLedger::default();
        ledger.grant(&Disclosure::for_request(&app(), &remote("a"), &req()));
        let mut moved = remote("a");
        moved.endpoint = Some("https://other.example.org".into());
        assert!(ledger.needs_consent(&Disclosure::for_request(&app(), &moved, &req()), 0));
    }

    #[test]
    fn consent_is_per_app_and_revocable() {
        let mut ledger = ConsentLedger::default();
        let d = Disclosure::for_request(&app(), &remote("a"), &req());
        ledger.grant(&d);
        let other = Disclosure::for_request(&AppId::new("wordcraft").unwrap(), &remote("a"), &req());
        assert!(ledger.needs_consent(&other, 0));
        ledger.revoke(&app(), &remote("a").id);
        assert!(ledger.needs_consent(&d, 0));
    }

    #[test]
    fn parameters_alone_still_need_consent() {
        let only_params = Request::new(CapabilityId::parse("image.inpaint").unwrap()).with_param("negative", "a private note".into());
        let d = disclosure_for(&only_params);
        assert_eq!(d.kinds(), vec![PayloadKind::Text]);
        let mut ledger = ConsentLedger::default();
        assert!(ledger.needs_consent(&d, 0));
        ledger.grant(&d);
        assert!(!ledger.needs_consent(&d, 0));
    }

    fn upscale_req() -> Request {
        Request::new(CapabilityId::parse("image.upscale").unwrap()).with_input(InputRole::Source, Payload::inline("image/png", vec![0; 10]).unwrap())
    }

    fn disclosure_for(r: &Request) -> Disclosure {
        Disclosure::for_request(&app(), &remote("a"), r)
    }

    #[test]
    fn disclosure_names_the_capability() {
        assert_eq!(disclosure_for(&req()).capability, CapabilityId::parse("image.inpaint").unwrap());
    }

    #[test]
    fn consent_for_one_capability_does_not_cover_another() {
        let mut ledger = ConsentLedger::default();
        ledger.grant(&disclosure_for(&req()));
        let other = disclosure_for(&upscale_req());
        assert!(ledger.needs_consent(&other, 0));
        ledger.grant(&other);
        assert!(!ledger.needs_consent(&other, 0));
        assert!(!ledger.needs_consent(&disclosure_for(&req()), 0));
        assert_eq!(ledger.records.len(), 2);
    }

    #[test]
    fn a_timed_agreement_lapses_at_its_expiry() {
        let d = disclosure_for(&req());
        let mut ledger = ConsentLedger::default();
        ledger.grant_until(&d, 1_000);
        assert!(!ledger.needs_consent(&d, 0));
        assert!(!ledger.needs_consent(&d, 999));
        assert!(ledger.needs_consent(&d, 1_000));
        assert!(ledger.needs_consent(&d, 5_000));
    }

    #[test]
    fn an_open_agreement_never_lapses() {
        let d = disclosure_for(&req());
        let mut ledger = ConsentLedger::default();
        ledger.grant(&d);
        assert!(!ledger.needs_consent(&d, u64::MAX));
    }

    #[test]
    fn agreeing_again_after_it_lapsed_renews_it() {
        let d = disclosure_for(&req());
        let mut ledger = ConsentLedger::default();
        ledger.grant_until(&d, 1_000);
        assert!(ledger.needs_consent(&d, 2_000));
        ledger.grant_until(&d, 3_000);
        assert!(!ledger.needs_consent(&d, 2_000));
        assert!(ledger.needs_consent(&d, 3_000));
        assert_eq!(ledger.records.len(), 1);
    }

    #[test]
    fn the_latest_agreement_sets_the_expiry_for_that_capability() {
        let d = disclosure_for(&req());
        let mut ledger = ConsentLedger::default();
        ledger.grant(&d);
        ledger.grant_until(&d, 1_000);
        assert!(ledger.needs_consent(&d, 1_000));
        ledger.grant(&d);
        assert!(!ledger.needs_consent(&d, u64::MAX));
    }

    #[test]
    fn a_timed_agreement_for_one_capability_leaves_another_open() {
        let mut ledger = ConsentLedger::default();
        ledger.grant(&disclosure_for(&req()));
        ledger.grant_until(&disclosure_for(&upscale_req()), 1_000);
        assert!(!ledger.needs_consent(&disclosure_for(&req()), 5_000));
        assert!(ledger.needs_consent(&disclosure_for(&upscale_req()), 5_000));
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

    #[test]
    fn renewing_one_kind_leaves_the_others_on_their_own_expiry() {
        let mut ledger = ConsentLedger::default();
        ledger.grant_until(&disclosure_for(&req()), 1_000);
        ledger.grant(&disclosure_for(&text_only_req()));
        let both = disclosure_for(&req());
        assert!(!ledger.needs_consent(&both, 999));
        assert!(ledger.needs_consent(&both, 1_000));
        assert!(!ledger.needs_consent(&disclosure_for(&text_only_req()), u64::MAX));
    }

    #[test]
    fn a_record_from_before_scoping_loads_but_covers_nothing() {
        let old = r#"{"records":[{"app":"photocraft","connection":"a","destination":"https://api.example.com","kinds":["text","image"]}]}"#;
        let ledger: ConsentLedger = serde_json::from_str(old).unwrap();
        assert_eq!(ledger.records.len(), 1);
        assert_eq!(ledger.records[0].capability, None);
        assert_eq!(ledger.records[0].expires_at, None);
        assert!(ledger.needs_consent(&disclosure_for(&req()), 0));
    }

    #[test]
    fn the_capability_and_expiry_survive_a_save() {
        let mut ledger = ConsentLedger::default();
        ledger.grant_until(&disclosure_for(&req()), 1_000);
        let back: ConsentLedger = serde_json::from_str(&serde_json::to_string(&ledger).unwrap()).unwrap();
        assert_eq!(back, ledger);
        assert_eq!(back.records[0].expires_at, Some(1_000));
    }
}
