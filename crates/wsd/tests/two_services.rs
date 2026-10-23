//! The same core code, driven against a print-shaped and a scan-shaped in-memory device: discovery,
//! metadata, a service call, and the whole WS-Eventing life cycle. Nothing here is specific to
//! either service; only the `ServiceKind` passed in differs. No sockets are involved.
#![cfg(feature = "testing")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::net::SocketAddr;
use std::time::Duration;

use craft_wsd::discovery::fake::{FakeResponder, FakeSocket};
use craft_wsd::discovery::{DiscoverOptions, Target, discover};
use craft_wsd::eventing::{self, CollectingSink, EndStatus, EventingFault, Expiry, Filter, Incoming, SubscribeRequest};
use craft_wsd::testing::FakeDevice;
use craft_wsd::{EndpointRef, Error, QName, ReferenceParameter, ServiceKind, SoapClient, ns, xml};

/// A discovery target of `kind` that advertises `device`'s URL and one public address the policy drops.
fn target_for(kind: ServiceKind, device: &FakeDevice) -> Target {
    Target {
        address: "urn:uuid:00000000-0000-4000-8000-000000000001".into(),
        types: vec![QName::new(ns::WSDP, "Device"), kind.descriptor().device_type_qname()],
        scopes: vec![],
        xaddrs: vec![device.device_url().as_url(), "http://8.8.8.8/not-allowed".into()],
        metadata_version: Some(1),
    }
}

/// Everything a client of any WSD service does, written once.
fn exercise(kind: ServiceKind, other: ServiceKind) {
    let d = kind.descriptor();
    let device = FakeDevice::new(kind, 5357);

    // 1. Find the device by its service kind; only the policy-allowed address survives.
    let socket = FakeSocket::new();
    socket.add_device(FakeResponder::new(target_for(kind, &device), SocketAddr::from(([127, 0, 0, 1], 3702))));
    let report = discover(&socket, &DiscoverOptions { window: Duration::from_secs(1), ..DiscoverOptions::for_service(kind) }).unwrap();
    assert_eq!(report.devices.len(), 1);
    assert_eq!((report.devices[0].urls.len(), report.devices[0].dropped_xaddrs), (1, 1));
    // A probe for the other kind finds nothing on the same network.
    let none = discover(&socket, &DiscoverOptions { window: Duration::from_secs(1), ..DiscoverOptions::for_service(other) }).unwrap();
    assert!(none.devices.is_empty());

    // 2. Read its metadata from the discovered address and pick this kind's endpoint.
    let at_device = SoapClient::new(&device, report.devices[0].urls[0].clone());
    let meta = craft_wsd::metadata::transfer_get(&at_device).unwrap();
    assert_eq!(meta.model.as_ref().unwrap().manufacturer, "Example Works");
    assert_eq!(meta.kinds(), [kind]);
    assert_eq!(meta.endpoint_for(kind).unwrap(), device.service_url());
    assert!(matches!(meta.endpoint_for(other), Err(Error::NotFound(_))));
    assert_eq!(meta.host.as_ref().unwrap().types.len(), 2);
    let by_dialect = craft_wsd::metadata::get_metadata(&at_device, Some(craft_wsd::metadata::DIALECT_RELATIONSHIP)).unwrap();
    assert_eq!(by_dialect.hosted.len(), 1);

    // 3. Call a service operation with the descriptor's action table.
    let svc = at_device.at(meta.endpoint_for(kind).unwrap());
    let op = if kind == ServiceKind::Print { "GetPrinterElements" } else { "GetScannerElements" };
    let handled =
        svc.call(&d.call(op, format!("<{p}:{op}Request/>", p = d.prefix)), |n| Ok::<_, Error>(xml::child_text(n, d.namespace, "Handled"))).unwrap();
    assert_eq!(handled.as_deref(), Some(d.name));
    // An operation of the other service's namespace is refused by the device with a fault.
    let wrong = other.descriptor().call("Anything", format!("<{}:AnythingRequest/>", other.descriptor().prefix));
    assert!(matches!(svc.call(&wrong, |_| Ok::<_, Error>(())), Err(Error::Fault(f)) if f.code.is_action_not_supported()));

    // 4. Events: subscribe, renew, status, receive an event and an end, unsubscribe.
    let sink = CollectingSink::new();
    let mut req = SubscribeRequest::new(
        EndpointRef::new("http://127.0.0.1:9100/events").with_parameter(ReferenceParameter::new("urn:test", "Context", "ctx-1")),
    );
    req.end_to = Some(EndpointRef::new("http://127.0.0.1:9100/end"));
    req.expires = Some(Expiry::After(Duration::from_secs(600)));
    req.filter = Some(Filter::actions(&[&d.action("SomethingChanged")]).unwrap());
    let sub = eventing::subscribe(&svc, &req).unwrap();
    assert_eq!(sub.expires, Some(Expiry::After(Duration::from_secs(600))));
    let id = sub.identifier().unwrap().to_owned();
    assert_eq!(device.subscriptions().len(), 1);

    assert_eq!(eventing::renew(&svc, &sub, Some(Expiry::After(Duration::from_secs(7200)))).unwrap(), Some(Expiry::After(Duration::from_secs(3600))));
    assert_eq!(eventing::get_status(&svc, &sub).unwrap(), Some(Expiry::After(Duration::from_secs(1800))));
    // Follow-up messages carried the identifier as a header, to the manager address.
    let followups: Vec<_> = device.log().into_iter().filter(|l| l.identifier.is_some()).collect();
    assert_eq!(followups.len(), 2);
    assert!(followups.iter().all(|l| l.identifier.as_deref() == Some(id.as_str()) && l.path == FakeDevice::SERVICE_PATH));

    for (to, bytes) in device
        .notify(&d.action("SomethingChanged"), &format!("<{p}:SomethingChanged><{p}:What>paper</{p}:What></{p}:SomethingChanged>", p = d.prefix))
    {
        assert_eq!(to.address, "http://127.0.0.1:9100/events");
        eventing::dispatch(&bytes, &sink).unwrap();
    }
    let ended = device.end_all(&format!("{}/SourceShuttingDown", ns::WSE), "power off");
    assert_eq!(ended.len(), 1);
    eventing::dispatch(&ended[0].1, &sink).unwrap();
    let got = sink.items();
    let Incoming::Event(event) = &got[0] else { panic!("not an event: {got:?}") };
    assert_eq!(event.body, QName::new(d.namespace, "SomethingChanged"));
    assert_eq!(event.header("urn:test", "Context"), Some("ctx-1"), "the NotifyTo reference parameter came back as a header");
    let what = event.with_body(|b| Ok::<_, Error>(xml::child_text(b, d.namespace, "What"))).unwrap();
    assert_eq!(what.as_deref(), Some("paper"));
    let Incoming::SubscriptionEnd(end) = &got[1] else { panic!("not an end: {got:?}") };
    assert_eq!((end.status.clone(), end.reason.as_str(), end.identifier()), (EndStatus::SourceShuttingDown, "power off", Some(id.as_str())));
    assert!(device.subscriptions().is_empty());

    // Unsubscribing, and managing a subscription that is gone, behave the same on both.
    let sub2 = eventing::subscribe(&svc, &SubscribeRequest::new(EndpointRef::new("http://127.0.0.1:9100/events"))).unwrap();
    eventing::unsubscribe(&svc, &sub2).unwrap();
    assert!(device.subscriptions().is_empty());
    match eventing::renew(&svc, &sub2, None) {
        Err(Error::Fault(f)) => assert_eq!(EventingFault::from_subcode(&f.code), Some(EventingFault::UnableToRenew)),
        other => panic!("{other:?}"),
    }

    // A sink or manager outside the connection policy never reaches the wire.
    let before = device.log().len();
    let mut public = SubscribeRequest::new(EndpointRef::new("http://8.8.8.8/events"));
    assert!(matches!(eventing::subscribe(&svc, &public), Err(Error::Invalid(_))));
    public.notify_to = EndpointRef::new("http://127.0.0.1:9100/events");
    // An allowed sink is not enough: where the device sends the end-of-subscription message is checked too.
    public.end_to = Some(EndpointRef::new("http://8.8.8.8/end"));
    assert!(matches!(eventing::subscribe(&svc, &public), Err(Error::Invalid(_))));
    let hijacked = eventing::Subscription { manager: EndpointRef::new("http://8.8.8.8/mgr"), expires: None };
    assert!(matches!(eventing::renew(&svc, &hijacked, None), Err(Error::Policy(_))));
    assert_eq!(device.log().len(), before);
}

#[test]
fn the_same_core_drives_a_print_shaped_device() {
    exercise(ServiceKind::Print, ServiceKind::Scan);
}

#[test]
fn the_same_core_drives_a_scan_shaped_device() {
    exercise(ServiceKind::Scan, ServiceKind::Print);
}
