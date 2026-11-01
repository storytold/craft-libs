//! Namespaces shared by every WSD service. Service-specific namespaces and action tables live in
//! the service crates.

/// SOAP 1.2 envelope.
pub const SOAP: &str = "http://www.w3.org/2003/05/soap-envelope";
/// WS-Addressing as WSD uses it (the 2004/08 draft).
pub const WSA: &str = "http://schemas.xmlsoap.org/ws/2004/08/addressing";
/// The final WS-Addressing namespace; tolerated on input, never written.
pub const WSA_FINAL: &str = "http://www.w3.org/2005/08/addressing";
/// XML Optimized Packaging includes.
pub const XOP: &str = "http://www.w3.org/2004/08/xop/include";
/// The anonymous reply address.
pub const ANONYMOUS: &str = "http://schemas.xmlsoap.org/ws/2004/08/addressing/role/anonymous";
/// Action carried by a WS-Addressing fault reply.
pub const FAULT_ACTION: &str = "http://schemas.xmlsoap.org/ws/2004/08/addressing/fault";

/// WS-Discovery as WSDAPI speaks it (the 2005/04 draft); the namespace every message is written in.
pub const WSD: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery";
/// The OASIS WS-Discovery 1.1 namespace; tolerated on input, never written.
pub const WSD_11: &str = "http://docs.oasis-open.org/ws-dd/ns/discovery/2009/01";
/// Devices Profile for Web Services: `ThisModel`, `ThisDevice`, `Relationship`, `Host`, `Hosted`, `Types`.
pub const WSDP: &str = "http://schemas.xmlsoap.org/ws/2006/02/devprof";
/// WS-MetadataExchange (`wsx:Metadata`, `wsx:GetMetadata`).
pub const WSX: &str = "http://schemas.xmlsoap.org/ws/2004/09/mex";
/// WS-Transfer (`Get`).
pub const WXF: &str = "http://schemas.xmlsoap.org/ws/2004/09/transfer";
/// WS-Eventing (the 2004/08 submission shapes Windows devices use).
pub const WSE: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing";
/// Plug-and-Play Extensions (`pnpx:HardwareId`, `pnpx:CompatibleId`, `pnpx:DeviceCategory`).
pub const PNPX: &str = "http://schemas.microsoft.com/windows/pnpx/2005/10";
/// The XML namespace (`xml:lang`).
pub const XML: &str = "http://www.w3.org/XML/1998/namespace";

/// The `wsa:To` of multicast discovery messages (Hello, Bye, Probe, Resolve).
pub const DISCOVERY_TO: &str = "urn:schemas-xmlsoap-org:ws:2005:04:discovery";
