//! Device metadata: what a device says about itself and the services it hosts.
//!
//! Two requests return the same `wsx:Metadata` document: WS-Transfer `Get` (no body) and
//! WS-MetadataExchange `GetMetadata` (optionally for one dialect). [`DeviceMetadata`] is the typed
//! reading of it: the model and device descriptions, the host and the hosted services with their
//! types, service ids and endpoint references. Nothing is specific to print or scan: callers pick
//! a service by type with [`DeviceMetadata::service_endpoint`] (or by [`ServiceKind`]).
//!
//! Reading is bounded (sections, services, endpoints, types) and strict where a value decides
//! where we connect: a hosted service with two `Types` elements, or an endpoint reference with two
//! addresses, is refused rather than guessed at. Descriptive strings take the first occurrence.
//! Addresses are only ever turned into connectable URLs through the [`DeviceUrl`] policy, so a
//! device cannot steer a client to a public address or a host name.

use roxmltree::Node;

use crate::addressing::EndpointRef;
use crate::error::{Error, Result};
use crate::ns;
use crate::policy::DeviceUrl;
use crate::service::ServiceKind;
use crate::soap::{Call, SoapClient};
use crate::transport::Transport;
use crate::xml::{self, QName};

pub const DIALECT_THIS_MODEL: &str = "http://schemas.xmlsoap.org/ws/2006/02/devprof/ThisModel";
pub const DIALECT_THIS_DEVICE: &str = "http://schemas.xmlsoap.org/ws/2006/02/devprof/ThisDevice";
pub const DIALECT_RELATIONSHIP: &str = "http://schemas.xmlsoap.org/ws/2006/02/devprof/Relationship";
/// `Relationship/@Type` of the host relationship.
pub const RELATIONSHIP_HOST: &str = "http://schemas.xmlsoap.org/ws/2006/02/devprof/host";

pub const ACTION_TRANSFER_GET: &str = "http://schemas.xmlsoap.org/ws/2004/09/transfer/Get";
pub const ACTION_TRANSFER_GET_RESPONSE: &str = "http://schemas.xmlsoap.org/ws/2004/09/transfer/GetResponse";
pub const ACTION_GET_METADATA: &str = "http://schemas.xmlsoap.org/ws/2004/09/mex/GetMetadata/Request";
pub const ACTION_GET_METADATA_RESPONSE: &str = "http://schemas.xmlsoap.org/ws/2004/09/mex/GetMetadata/Response";

const MAX_SECTIONS: usize = 32;
const MAX_SERVICES: usize = 16;
const MAX_ENDPOINTS: usize = 8;
const MAX_TYPES: usize = 32;
const MAX_FIELD: usize = 256;

/// `wsdp:ThisModel`: the product.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThisModel {
    pub manufacturer: String,
    pub manufacturer_url: Option<String>,
    pub model_name: String,
    pub model_number: Option<String>,
    pub model_url: Option<String>,
    pub presentation_url: Option<String>,
    /// `pnpx:DeviceCategory` words (`Printers`, `Scanners`, ...), if the device states any.
    pub device_categories: Vec<String>,
}

/// `wsdp:ThisDevice`: this unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThisDevice {
    pub friendly_name: String,
    pub firmware_version: Option<String>,
    pub serial_number: Option<String>,
}

/// A `Host` or `Hosted` entry: endpoint references, types and a service id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceEntry {
    pub endpoints: Vec<EndpointRef>,
    pub types: Vec<QName>,
    pub service_id: Option<String>,
    /// `pnpx:HardwareId` / `pnpx:CompatibleId`, as the device lists them.
    pub hardware_ids: Vec<String>,
    pub compatible_ids: Vec<String>,
}

impl ServiceEntry {
    /// Whether the entry lists the type `q` (exact namespace and local name).
    pub fn has_type(&self, q: &QName) -> bool {
        self.types.contains(q)
    }

    /// Every endpoint address the connection policy allows, in the order given, without duplicates.
    pub fn urls(&self) -> Vec<DeviceUrl> {
        let mut out: Vec<DeviceUrl> = Vec::new();
        for e in &self.endpoints {
            if let Ok(u) = e.url()
                && !out.contains(&u)
            {
                out.push(u);
            }
        }
        out
    }
}

/// The typed form of a `wsx:Metadata` document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceMetadata {
    pub model: Option<ThisModel>,
    pub device: Option<ThisDevice>,
    /// The host of the relationship: the device itself, with its device-level types.
    pub host: Option<ServiceEntry>,
    pub hosted: Vec<ServiceEntry>,
    /// Sections that point elsewhere (`MetadataReference` / `Location`) instead of holding the
    /// metadata inline. They are listed, never followed.
    pub references: Vec<String>,
}

impl DeviceMetadata {
    /// Read a `wsx:Metadata` element.
    pub fn from_element(metadata: Node<'_, '_>) -> Result<Self> {
        if !xml::is(metadata, ns::WSX, "Metadata") {
            return Err(Error::Protocol(format!("expected wsx:Metadata, found {}", metadata.tag_name().name())));
        }
        let mut out = Self::default();
        for section in xml::children(metadata, ns::WSX, "MetadataSection").take(MAX_SECTIONS) {
            let dialect = section.attribute("Dialect").unwrap_or("");
            if let Some(r) = xml::child(section, ns::WSX, "Location") {
                out.references.push(xml::text(r));
                continue;
            }
            if let Some(r) = xml::child(section, ns::WSX, "MetadataReference") {
                out.references.push(EndpointRef::parse(r).map(|e| e.address).unwrap_or_default());
                continue;
            }
            match dialect {
                DIALECT_THIS_MODEL if out.model.is_none() => out.model = xml::child(section, ns::WSDP, "ThisModel").map(read_model),
                DIALECT_THIS_DEVICE if out.device.is_none() => out.device = xml::child(section, ns::WSDP, "ThisDevice").map(read_device),
                DIALECT_RELATIONSHIP => {
                    for rel in xml::children(section, ns::WSDP, "Relationship").filter(|r| r.attribute("Type") == Some(RELATIONSHIP_HOST)) {
                        out.read_relationship(rel);
                    }
                }
                _ => {}
            }
        }
        out.references.retain(|r| !r.is_empty());
        Ok(out)
    }

    /// Take a `Relationship`'s `Host` (the first one seen wins) and up to `MAX_SERVICES` `Hosted` entries.
    ///
    /// An entry that is ambiguous (two addresses in one reference, two `Types`, an oversized address) is
    /// dropped, so the client never guesses where to connect; it does not make the whole document
    /// unreadable, and the well-formed entries beside it stay usable.
    fn read_relationship(&mut self, rel: Node<'_, '_>) {
        if let Some(h) = xml::child(rel, ns::WSDP, "Host")
            && self.host.is_none()
        {
            self.host = read_entry(h).ok();
        }
        for h in xml::children(rel, ns::WSDP, "Hosted").take(MAX_SERVICES) {
            if let Ok(entry) = read_entry(h)
                && self.hosted.len() < MAX_SERVICES
            {
                self.hosted.push(entry);
            }
        }
    }

    /// The first hosted service that lists `service_type`.
    pub fn hosted_service(&self, service_type: &QName) -> Option<&ServiceEntry> {
        self.hosted.iter().find(|h| h.has_type(service_type))
    }

    /// The first hosted service of this kind (by its service type).
    pub fn hosted_kind(&self, kind: ServiceKind) -> Option<&ServiceEntry> {
        self.hosted_service(&kind.descriptor().service_type_qname())
    }

    /// Every connectable endpoint of every hosted service of this type, policy-checked.
    pub fn service_endpoints(&self, service_type: &QName) -> Vec<DeviceUrl> {
        let mut out: Vec<DeviceUrl> = Vec::new();
        for h in self.hosted.iter().filter(|h| h.has_type(service_type)) {
            for u in h.urls() {
                if !out.contains(&u) {
                    out.push(u);
                }
            }
        }
        out
    }

    /// The endpoint to talk to for `service_type`: the first policy-allowed address of the first
    /// hosted service of that type. A device that hosts the type only at a public address or a
    /// host name is [`Error::NotFound`], not silently used.
    pub fn service_endpoint(&self, service_type: &QName) -> Result<DeviceUrl> {
        match self.service_endpoints(service_type).into_iter().next() {
            Some(u) => Ok(u),
            None if self.hosted_service(service_type).is_some() => {
                Err(Error::NotFound(format!("service {service_type} is hosted, but at no address this client may connect to")))
            }
            None => Err(Error::NotFound(format!("the device hosts no service of type {service_type}"))),
        }
    }

    /// [`Self::service_endpoint`] for a service kind (print or scan).
    pub fn endpoint_for(&self, kind: ServiceKind) -> Result<DeviceUrl> {
        self.service_endpoint(&kind.descriptor().service_type_qname())
    }

    /// Which service kinds the device hosts (by hosted type), in [`ServiceKind::ALL`] order.
    pub fn kinds(&self) -> Vec<ServiceKind> {
        ServiceKind::ALL.into_iter().filter(|k| self.hosted_kind(*k).is_some()).collect()
    }
}

/// The text of the child `{ns_uri}local`, cut to `MAX_FIELD` characters.
fn field(n: Node<'_, '_>, ns_uri: &str, local: &str) -> Option<String> {
    xml::child_text(n, ns_uri, local).map(|t| t.chars().take(MAX_FIELD).collect())
}

/// Read `wsdp:ThisModel`; missing names are empty and at most eight device categories are kept.
fn read_model(n: Node<'_, '_>) -> ThisModel {
    ThisModel {
        manufacturer: field(n, ns::WSDP, "Manufacturer").unwrap_or_default(),
        manufacturer_url: field(n, ns::WSDP, "ManufacturerUrl"),
        model_name: field(n, ns::WSDP, "ModelName").unwrap_or_default(),
        model_number: field(n, ns::WSDP, "ModelNumber"),
        model_url: field(n, ns::WSDP, "ModelUrl"),
        presentation_url: field(n, ns::WSDP, "PresentationUrl"),
        device_categories: field(n, ns::PNPX, "DeviceCategory")
            .map(|c| c.split_whitespace().take(8).map(str::to_owned).collect())
            .unwrap_or_default(),
    }
}

/// Read `wsdp:ThisDevice`; a missing friendly name is empty.
fn read_device(n: Node<'_, '_>) -> ThisDevice {
    ThisDevice {
        friendly_name: field(n, ns::WSDP, "FriendlyName").unwrap_or_default(),
        firmware_version: field(n, ns::WSDP, "FirmwareVersion"),
        serial_number: field(n, ns::WSDP, "SerialNumber"),
    }
}

/// One `Host` / `Hosted` element. `Types` and `ServiceId` may each appear once; a second `Types`
/// would let two lists disagree about what the endpoint is.
fn read_entry(n: Node<'_, '_>) -> Result<ServiceEntry> {
    let mut entry = ServiceEntry::default();
    for epr in xml::children(n, ns::WSA, "EndpointReference").chain(xml::children(n, ns::WSA_FINAL, "EndpointReference")).take(MAX_ENDPOINTS) {
        entry.endpoints.push(EndpointRef::parse(epr)?);
    }
    let mut types = xml::children(n, ns::WSDP, "Types");
    if let Some(t) = types.next() {
        if types.next().is_some() {
            return Err(Error::Protocol("a Host or Hosted entry has more than one Types element".into()));
        }
        entry.types = xml::qname_list(t, MAX_TYPES);
    }
    entry.service_id = field(n, ns::WSDP, "ServiceId");
    entry.hardware_ids = xml::children(n, ns::PNPX, "HardwareId").take(8).map(xml::text).filter(|t| !t.is_empty()).collect();
    entry.compatible_ids = xml::children(n, ns::PNPX, "CompatibleId").take(8).map(xml::text).filter(|t| !t.is_empty()).collect();
    Ok(entry)
}

/// The `Get` request: an empty Body.
pub fn transfer_get_call() -> Call<'static> {
    Call::new(ACTION_TRANSFER_GET, ACTION_TRANSFER_GET_RESPONSE).reply_element(ns::WSX, "Metadata")
}

/// The `GetMetadata` request body, for one dialect or (with `None`) everything.
pub fn get_metadata_body(dialect: Option<&str>) -> String {
    match dialect {
        Some(d) => format!("<wsx:GetMetadata><wsx:Dialect>{}</wsx:Dialect></wsx:GetMetadata>", xml::esc(d)),
        None => "<wsx:GetMetadata/>".to_owned(),
    }
}

/// The `GetMetadata` request.
pub fn get_metadata_call(dialect: Option<&str>) -> Call<'static> {
    Call::new(ACTION_GET_METADATA, ACTION_GET_METADATA_RESPONSE)
        .reply_element(ns::WSX, "Metadata")
        .namespace("wsx", ns::WSX)
        .body(get_metadata_body(dialect))
}

/// WS-Transfer `Get` on the client's endpoint (a device or hosted-service address).
pub fn transfer_get<T: Transport>(client: &SoapClient<T>) -> Result<DeviceMetadata> {
    client.call(&transfer_get_call(), DeviceMetadata::from_element)
}

/// WS-MetadataExchange `GetMetadata` on the client's endpoint.
pub fn get_metadata<T: Transport>(client: &SoapClient<T>, dialect: Option<&str>) -> Result<DeviceMetadata> {
    client.call(&get_metadata_call(dialect), DeviceMetadata::from_element)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A metadata document written in our own words: a model, a device, and a relationship with
    /// the host and one hosted service of `service_type` at `address`.
    pub(crate) fn document(service_ns: &str, service_prefix: &str, service_type: &str, address: &str) -> String {
        format!(
            "<wsx:Metadata xmlns:wsx=\"{wsx}\" xmlns:wsdp=\"{wsdp}\" xmlns:wsa=\"{wsa}\" xmlns:pnpx=\"{pnpx}\" xmlns:{service_prefix}=\"{service_ns}\">\
<wsx:MetadataSection Dialect=\"{DIALECT_THIS_MODEL}\"><wsdp:ThisModel><wsdp:Manufacturer xml:lang=\"en\">Example Works</wsdp:Manufacturer><wsdp:ModelName>Model 1</wsdp:ModelName><wsdp:ModelNumber>M-1</wsdp:ModelNumber><pnpx:DeviceCategory>Printers Scanners</pnpx:DeviceCategory></wsdp:ThisModel></wsx:MetadataSection>\
<wsx:MetadataSection Dialect=\"{DIALECT_THIS_DEVICE}\"><wsdp:ThisDevice><wsdp:FriendlyName>Office device</wsdp:FriendlyName><wsdp:SerialNumber>SN-0001</wsdp:SerialNumber></wsdp:ThisDevice></wsx:MetadataSection>\
<wsx:MetadataSection Dialect=\"{DIALECT_RELATIONSHIP}\"><wsdp:Relationship Type=\"{RELATIONSHIP_HOST}\">\
<wsdp:Host><wsa:EndpointReference><wsa:Address>urn:uuid:11111111-2222-4333-8444-555555555555</wsa:Address></wsa:EndpointReference><wsdp:Types>wsdp:Device {service_prefix}:{service_type}Device</wsdp:Types><wsdp:ServiceId>uri:host</wsdp:ServiceId></wsdp:Host>\
<wsdp:Hosted><wsa:EndpointReference><wsa:Address>{address}</wsa:Address></wsa:EndpointReference><wsdp:Types>{service_prefix}:{service_type}</wsdp:Types><wsdp:ServiceId>uri:svc</wsdp:ServiceId><pnpx:HardwareId>VEN_X&amp;DEV_Y</pnpx:HardwareId><pnpx:CompatibleId>UMB\\x</pnpx:CompatibleId></wsdp:Hosted>\
</wsdp:Relationship></wsx:MetadataSection></wsx:Metadata>",
            wsx = ns::WSX,
            wsdp = ns::WSDP,
            wsa = ns::WSA,
            pnpx = ns::PNPX,
        )
    }

    /// Parse `text` and read its root element as device metadata.
    fn read(text: &str) -> Result<DeviceMetadata> {
        let doc = xml::parse(text)?;
        DeviceMetadata::from_element(doc.root_element())
    }

    /// The scan service's namespace.
    fn scan_ns() -> &'static str {
        ServiceKind::Scan.descriptor().namespace
    }

    #[test]
    fn a_full_document_is_read_into_the_typed_model() {
        let m = read(&document(scan_ns(), "wscn", "ScannerServiceType", "http://127.0.0.1:3920/ScannerService")).unwrap();
        assert_eq!(m.model.as_ref().unwrap().manufacturer, "Example Works");
        assert_eq!(m.model.as_ref().unwrap().device_categories, ["Printers", "Scanners"]);
        assert_eq!(m.device.as_ref().unwrap().friendly_name, "Office device");
        let host = m.host.as_ref().unwrap();
        assert_eq!(host.service_id.as_deref(), Some("uri:host"));
        assert_eq!(host.types.len(), 2);
        let hosted = &m.hosted[0];
        assert_eq!(hosted.hardware_ids, ["VEN_X&DEV_Y"]);
        assert_eq!(hosted.compatible_ids, ["UMB\\x"]);
        assert!(m.references.is_empty());
    }

    #[test]
    fn the_endpoint_for_a_type_is_picked_by_namespace_and_name_not_by_prefix() {
        let print_ns = ServiceKind::Print.descriptor().namespace;
        // The same prefix `svc` bound to different namespaces: only the namespace decides.
        let print = read(&document(print_ns, "svc", "PrinterServiceType", "http://127.0.0.1:5357/print")).unwrap();
        let scan = read(&document(scan_ns(), "svc", "ScannerServiceType", "http://127.0.0.1:3920/scan")).unwrap();
        assert_eq!(print.endpoint_for(ServiceKind::Print).unwrap().as_url(), "http://127.0.0.1:5357/print");
        assert!(matches!(print.endpoint_for(ServiceKind::Scan), Err(Error::NotFound(_))));
        assert_eq!(scan.endpoint_for(ServiceKind::Scan).unwrap().port(), 3920);
        assert_eq!(print.kinds(), [ServiceKind::Print]);
        assert_eq!(scan.kinds(), [ServiceKind::Scan]);
        // A look-alike type in another namespace is not the service.
        let fake = read(&document("urn:other", "wscn", "ScannerService", "http://127.0.0.1:1/x")).unwrap();
        assert!(fake.kinds().is_empty());
    }

    #[test]
    fn public_and_hostname_service_addresses_are_not_used() {
        for bad in ["http://8.8.8.8/svc", "http://scanner.local/svc", "https://127.0.0.1/svc", "urn:uuid:1"] {
            let m = read(&document(scan_ns(), "wscn", "ScannerServiceType", bad)).unwrap();
            let t = ServiceKind::Scan.descriptor().service_type_qname();
            assert!(m.service_endpoints(&t).is_empty(), "{bad}");
        }
        // Hosted, but only at an address the policy refuses: an error that says so.
        let m = read(&document(scan_ns(), "wscn", "ScannerServiceType", "http://8.8.8.8/svc")).unwrap();
        assert!(matches!(m.endpoint_for(ServiceKind::Scan), Err(Error::NotFound(msg)) if msg.contains("no address")));
    }

    #[test]
    fn several_endpoints_prefer_the_first_allowed_one_and_drop_duplicates() {
        let text = format!(
            "<wsx:Metadata xmlns:wsx=\"{}\" xmlns:wsdp=\"{}\" xmlns:wsa=\"{}\" xmlns:t=\"urn:t\"><wsx:MetadataSection Dialect=\"{DIALECT_RELATIONSHIP}\"><wsdp:Relationship Type=\"{RELATIONSHIP_HOST}\"><wsdp:Hosted>\
<wsa:EndpointReference><wsa:Address>http://example.com/a</wsa:Address></wsa:EndpointReference>\
<wsa:EndpointReference><wsa:Address>http://10.0.0.7:80/b</wsa:Address></wsa:EndpointReference>\
<wsa:EndpointReference><wsa:Address>http://10.0.0.7/b</wsa:Address></wsa:EndpointReference>\
<wsa:EndpointReference><wsa:Address>http://[fe80::1]:8080/c</wsa:Address></wsa:EndpointReference>\
<wsdp:Types>t:Svc</wsdp:Types></wsdp:Hosted></wsdp:Relationship></wsx:MetadataSection></wsx:Metadata>",
            ns::WSX,
            ns::WSDP,
            ns::WSA
        );
        let m = read(&text).unwrap();
        let urls = m.service_endpoints(&QName::new("urn:t", "Svc"));
        assert_eq!(urls.iter().map(DeviceUrl::as_url).collect::<Vec<_>>(), ["http://10.0.0.7:80/b", "http://[fe80::1]:8080/c"]);
        assert_eq!(m.service_endpoint(&QName::new("urn:t", "Svc")).unwrap(), urls[0]);
    }

    #[test]
    fn hostile_documents_are_errors_or_bounded() {
        // Wrong root, wrong namespace, empty.
        assert!(matches!(read("<wsx:Metadata xmlns:wsx=\"urn:other\"/>"), Err(Error::Protocol(_))));
        assert!(matches!(read("<x/>"), Err(Error::Protocol(_))));
        assert!(matches!(read("not xml"), Err(Error::Xml(_))));
        let empty = read(&format!("<wsx:Metadata xmlns:wsx=\"{}\"/>", ns::WSX)).unwrap();
        assert_eq!(empty, DeviceMetadata::default());
        // Two Types on one hosted entry, and two addresses in one reference, drop that entry only.
        let base = document(scan_ns(), "wscn", "ScannerService", "http://127.0.0.1:1/x");
        let two_types = base.replace("<wsdp:ServiceId>uri:svc</wsdp:ServiceId>", "<wsdp:Types>wsdp:Device</wsdp:Types>");
        assert!(read(&two_types).unwrap().hosted.is_empty());
        let two_addr = base.replace(
            "<wsa:Address>http://127.0.0.1:1/x</wsa:Address>",
            "<wsa:Address>http://127.0.0.1:1/x</wsa:Address><wsa:Address>http://127.0.0.1:2/y</wsa:Address>",
        );
        assert!(read(&two_addr).unwrap().hosted.is_empty());
        // Duplicate descriptive sections: the first wins.
        let dup = base.replacen("<wsx:MetadataSection Dialect=\"http://schemas.xmlsoap.org/ws/2006/02/devprof/ThisDevice\">", "<wsx:MetadataSection Dialect=\"http://schemas.xmlsoap.org/ws/2006/02/devprof/ThisDevice\"><wsdp:ThisDevice><wsdp:FriendlyName>first</wsdp:FriendlyName></wsdp:ThisDevice></wsx:MetadataSection><wsx:MetadataSection Dialect=\"http://schemas.xmlsoap.org/ws/2006/02/devprof/ThisDevice\">", 1);
        assert_eq!(read(&dup).unwrap().device.unwrap().friendly_name, "first");
        // A relationship of another type contributes nothing.
        let other = base.replace(RELATIONSHIP_HOST, "urn:other-relationship");
        assert!(read(&other).unwrap().hosted.is_empty());
        // Huge fan-out is capped.
        let hosted: String = (0..100)
            .map(|i| {
                format!(
                    "<wsdp:Hosted><wsa:EndpointReference><wsa:Address>http://10.0.0.1:{}/s</wsa:Address></wsa:EndpointReference></wsdp:Hosted>",
                    i + 1
                )
            })
            .collect();
        let big = format!(
            "<wsx:Metadata xmlns:wsx=\"{}\" xmlns:wsdp=\"{}\" xmlns:wsa=\"{}\"><wsx:MetadataSection Dialect=\"{DIALECT_RELATIONSHIP}\"><wsdp:Relationship Type=\"{RELATIONSHIP_HOST}\">{hosted}</wsdp:Relationship></wsx:MetadataSection></wsx:Metadata>",
            ns::WSX,
            ns::WSDP,
            ns::WSA
        );
        assert_eq!(read(&big).unwrap().hosted.len(), MAX_SERVICES);
        let sections: String = (0..200).map(|_| "<wsx:MetadataSection Dialect=\"urn:x\"/>").collect();
        assert!(read(&format!("<wsx:Metadata xmlns:wsx=\"{}\">{sections}</wsx:Metadata>", ns::WSX)).is_ok());
    }

    #[test]
    fn one_ambiguous_hosted_entry_does_not_hide_the_well_formed_ones() {
        let print_ns = ServiceKind::Print.descriptor().namespace;
        let good = document(print_ns, "svc", "PrinterServiceType", "http://127.0.0.1:5357/print");
        let two_types = "<wsdp:Hosted><wsdp:Types>wsdp:Device</wsdp:Types><wsdp:Types>wsdp:Device</wsdp:Types></wsdp:Hosted>";
        let two_addr = "<wsdp:Hosted><wsa:EndpointReference><wsa:Address>http://127.0.0.1:1/a</wsa:Address><wsa:Address>http://127.0.0.1:2/b</wsa:Address></wsa:EndpointReference></wsdp:Hosted>";
        for bad in [two_types, two_addr] {
            let text = good.replacen("<wsdp:Hosted>", &format!("{bad}<wsdp:Hosted>"), 1);
            let m = read(&text).unwrap();
            assert_eq!(m.hosted.len(), 1, "only the well-formed entry is kept");
            assert_eq!(m.endpoint_for(ServiceKind::Print).unwrap().as_url(), "http://127.0.0.1:5357/print");
        }
        // An ambiguous Host entry is dropped the same way and leaves the services readable.
        let text = good.replacen("<wsdp:Host>", "<wsdp:Host><wsdp:Types>wsdp:Device</wsdp:Types><wsdp:Types>wsdp:Device</wsdp:Types>", 1);
        let m = read(&text).unwrap();
        assert!(m.host.is_none());
        assert_eq!(m.kinds(), [ServiceKind::Print]);
    }

    #[test]
    fn sections_that_point_elsewhere_are_listed_not_followed() {
        let text = format!(
            "<wsx:Metadata xmlns:wsx=\"{}\" xmlns:wsa=\"{}\"><wsx:MetadataSection Dialect=\"urn:a\"><wsx:Location>http://example.com/meta</wsx:Location></wsx:MetadataSection><wsx:MetadataSection Dialect=\"urn:b\"><wsx:MetadataReference><wsa:Address>http://10.0.0.2/m</wsa:Address></wsx:MetadataReference></wsx:MetadataSection><wsx:MetadataSection Dialect=\"urn:c\"><wsx:MetadataReference/></wsx:MetadataSection></wsx:Metadata>",
            ns::WSX,
            ns::WSA
        );
        let m = read(&text).unwrap();
        assert_eq!(m.references, ["http://example.com/meta", "http://10.0.0.2/m"]);
    }

    #[test]
    fn long_strings_are_capped() {
        let long = "x".repeat(2000);
        let text = format!(
            "<wsx:Metadata xmlns:wsx=\"{}\" xmlns:wsdp=\"{}\"><wsx:MetadataSection Dialect=\"{DIALECT_THIS_DEVICE}\"><wsdp:ThisDevice><wsdp:FriendlyName>{long}</wsdp:FriendlyName></wsdp:ThisDevice></wsx:MetadataSection></wsx:Metadata>",
            ns::WSX,
            ns::WSDP
        );
        assert_eq!(read(&text).unwrap().device.unwrap().friendly_name.len(), MAX_FIELD);
    }

    #[test]
    fn request_builders_are_well_formed() {
        assert_eq!(get_metadata_body(None), "<wsx:GetMetadata/>");
        let body = get_metadata_body(Some("urn:a&b"));
        assert!(body.contains("urn:a&amp;b"));
        let text = format!("<r xmlns:wsx=\"{}\">{body}</r>", ns::WSX);
        let doc = xml::parse(&text).unwrap();
        let gm = xml::find(doc.root_element(), ns::WSX, "GetMetadata").unwrap();
        assert_eq!(xml::child_text(gm, ns::WSX, "Dialect").as_deref(), Some("urn:a&b"));
        assert_eq!(transfer_get_call().action(), ACTION_TRANSFER_GET);
        assert_eq!(get_metadata_call(None).action(), ACTION_GET_METADATA);
    }
}
