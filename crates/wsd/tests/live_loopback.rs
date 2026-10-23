//! The shared core against a separately started loopback fake device: metadata (WS-Transfer `Get` and
//! WS-MetadataExchange `GetMetadata`) and the WS-Eventing client life cycle. Skipped unless
//! `CRAFT_WSD_LIVE_URL` names its WSD print service (for example
//! `http://127.0.0.1:3921/WebServices/PrinterService`). The device endpoint defaults to
//! `/WebServices/Device` on the same host and port; `CRAFT_WSD_LIVE_DEVICE_URL` overrides it.
//! Loopback only: the URLs go through the same policy as any device URL, and the test also insists
//! on a loopback address. No discovery is attempted (that would be multicast).
//!
//! Run: `CRAFT_WSD_LIVE_URL=http://127.0.0.1:PORT/PATH cargo test -p craft-wsd --test live_loopback`
#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::Duration;

use craft_wsd::eventing::{self, Expiry, Filter, SubscribeRequest};
use craft_wsd::http::HttpTransport;
use craft_wsd::metadata::{self, DIALECT_RELATIONSHIP};
use craft_wsd::{DeviceUrl, EndpointRef, ServiceKind, SoapClient};

/// The device and print-service URLs from `CRAFT_WSD_LIVE_URL` (and optionally
/// `CRAFT_WSD_LIVE_DEVICE_URL`), or `None` to skip the live tests. Panics unless both are loopback.
fn urls() -> Option<(DeviceUrl, DeviceUrl)> {
    let service = std::env::var("CRAFT_WSD_LIVE_URL").ok().filter(|u| !u.is_empty())?;
    let service = DeviceUrl::parse(&service).expect("CRAFT_WSD_LIVE_URL must be a numeric loopback/private http URL");
    assert!(service.host().is_loopback(), "the live test only talks to loopback");
    let device = match std::env::var("CRAFT_WSD_LIVE_DEVICE_URL").ok().filter(|u| !u.is_empty()) {
        Some(u) => DeviceUrl::parse(&u).expect("CRAFT_WSD_LIVE_DEVICE_URL must be a numeric loopback/private http URL"),
        None => DeviceUrl::parse(&format!("http://{}:{}/WebServices/Device", service.host(), service.port())).unwrap(),
    };
    assert!(device.host().is_loopback());
    Some((device, service))
}

/// A real HTTP client for `url` with a 30-second budget per test.
fn client(url: DeviceUrl) -> SoapClient<HttpTransport> {
    SoapClient::new(HttpTransport::new(), url).with_budget(Duration::from_secs(30))
}

#[test]
fn metadata_names_the_print_service_by_type() {
    let Some((device, service)) = urls() else { return };
    let meta = metadata::transfer_get(&client(device.clone())).expect("WS-Transfer Get");
    assert!(meta.model.is_some() && meta.device.is_some(), "{meta:?}");
    assert_eq!(meta.kinds(), [ServiceKind::Print]);
    let hosted = meta.endpoint_for(ServiceKind::Print).expect("a print service address the policy allows");
    assert_eq!((hosted.host(), hosted.port()), (service.host(), service.port()));
    assert!(meta.endpoint_for(ServiceKind::Scan).is_err());
    let by_dialect = metadata::get_metadata(&client(device), Some(DIALECT_RELATIONSHIP)).expect("GetMetadata");
    assert_eq!(by_dialect.hosted.len(), meta.hosted.len());
}

#[test]
fn a_subscription_can_be_made_renewed_queried_and_ended() {
    let Some((_, service)) = urls() else { return };
    let c = client(service);
    // Nothing listens on the sink; the device only has to accept the address and hand back a manager.
    let mut request = SubscribeRequest::new(EndpointRef::new("http://127.0.0.1:9/events"));
    request.expires = Some(Expiry::After(Duration::from_secs(300)));
    request.filter = Some(Filter::actions(&["http://schemas.microsoft.com/windows/2006/08/wdp/print/JobStatusEvent"]).unwrap());
    let sub = eventing::subscribe(&c, &request).expect("Subscribe");
    assert!(sub.identifier().is_some(), "{sub:?}");
    assert!(matches!(sub.expires, Some(Expiry::After(d)) if d > Duration::ZERO), "{sub:?}");
    let renewed = eventing::renew(&c, &sub, Some(Expiry::After(Duration::from_secs(600)))).expect("Renew");
    assert!(renewed.is_some());
    assert!(eventing::get_status(&c, &sub).expect("GetStatus").is_some());
    eventing::unsubscribe(&c, &sub).expect("Unsubscribe");
    assert!(eventing::get_status(&c, &sub).is_err(), "the subscription is gone");
}
