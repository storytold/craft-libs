//! Which services share this core, and the facts that identify each: namespace, the hosted
//! service type and the device type devices announce, and how operation names map to action URIs.
//!
//! The descriptor is a table, not a model: print and scan keep their own operation enumerations,
//! tickets and job states in their own crates. Everything here is what discovery, metadata and the
//! generic SOAP layer need to find and address a service.

use crate::soap::Call;
use crate::xml::QName;

/// A kind of WSD service this core can find and address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServiceKind {
    Print,
    Scan,
}

/// The identifying facts of one service kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceDescriptor {
    pub kind: ServiceKind,
    /// Lowercase name, stable for logs and tool output.
    pub name: &'static str,
    /// The service's schema namespace; operation bodies and action URIs live under it.
    pub namespace: &'static str,
    /// The conventional prefix for the namespace (`wprt`, `wscn`).
    pub prefix: &'static str,
    /// Local name of the hosted-service type in the `Hosted/Types` of the device metadata.
    pub service_type: &'static str,
    /// Local name of the device-level type announced in WS-Discovery `Types`.
    pub device_type: &'static str,
}

const PRINT: ServiceDescriptor = ServiceDescriptor {
    kind: ServiceKind::Print,
    name: "print",
    namespace: "http://schemas.microsoft.com/windows/2006/08/wdp/print",
    prefix: "wprt",
    service_type: "PrinterServiceType",
    device_type: "PrintDeviceType",
};

const SCAN: ServiceDescriptor = ServiceDescriptor {
    kind: ServiceKind::Scan,
    name: "scan",
    namespace: "http://schemas.microsoft.com/windows/2006/08/wdp/scan",
    prefix: "wscn",
    service_type: "ScannerServiceType",
    device_type: "ScanDeviceType",
};

impl ServiceKind {
    pub const ALL: [ServiceKind; 2] = [ServiceKind::Print, ServiceKind::Scan];

    pub const fn descriptor(self) -> &'static ServiceDescriptor {
        match self {
            Self::Print => &PRINT,
            Self::Scan => &SCAN,
        }
    }

    /// The kind whose hosted-service type is `q`, if any.
    pub fn from_service_type(q: &QName) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.descriptor().service_type_qname() == *q)
    }

    /// The kind whose device type is `q`, if any.
    pub fn from_device_type(q: &QName) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.descriptor().device_type_qname() == *q)
    }

    /// The kind owning this namespace, if any.
    pub fn from_namespace(ns: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.descriptor().namespace == ns)
    }
}

impl ServiceDescriptor {
    /// The hosted-service type as a qualified name.
    pub fn service_type_qname(&self) -> QName {
        QName::new(self.namespace, self.service_type)
    }

    /// The device-level type as a qualified name.
    pub fn device_type_qname(&self) -> QName {
        QName::new(self.namespace, self.device_type)
    }

    /// The request `wsa:Action` of operation `op` (`CreateScanJob`, `GetPrinterElements`, ...).
    pub fn action(&self, op: &str) -> String {
        format!("{}/{op}", self.namespace)
    }

    /// The reply `wsa:Action` of operation `op`.
    pub fn response_action(&self, op: &str) -> String {
        format!("{}/{op}Response", self.namespace)
    }

    /// A [`Call`] for operation `op` with `body`, wired for this service: its action and reply
    /// actions, its namespace declared under its conventional prefix, and `<op>Response` as the
    /// expected reply element. The body must use the descriptor's prefix.
    pub fn call(&self, op: &str, body: impl Into<String>) -> Call<'static> {
        Call::new(self.action(op), self.response_action(op))
            .reply_element(self.namespace, format!("{op}Response"))
            .namespace(self.prefix, self.namespace)
            .body(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_identify_their_service() {
        let print = ServiceKind::Print.descriptor();
        assert_eq!(print.action("CreatePrintJob"), "http://schemas.microsoft.com/windows/2006/08/wdp/print/CreatePrintJob");
        assert_eq!(print.response_action("CancelJob"), "http://schemas.microsoft.com/windows/2006/08/wdp/print/CancelJobResponse");
        let scan = ServiceKind::Scan.descriptor();
        assert_eq!(scan.action("CreateScanJob"), "http://schemas.microsoft.com/windows/2006/08/wdp/scan/CreateScanJob");
        assert_eq!(scan.service_type_qname().to_string(), "{http://schemas.microsoft.com/windows/2006/08/wdp/scan}ScannerServiceType");
        // Each kind is found by its own types and namespace and by nobody else's.
        for k in ServiceKind::ALL {
            let d = k.descriptor();
            assert_eq!(d.kind, k);
            assert_eq!(ServiceKind::from_service_type(&d.service_type_qname()), Some(k));
            assert_eq!(ServiceKind::from_device_type(&d.device_type_qname()), Some(k));
            assert_eq!(ServiceKind::from_namespace(d.namespace), Some(k));
            assert_eq!(ServiceKind::from_service_type(&d.device_type_qname()), None);
        }
        assert_eq!(ServiceKind::from_service_type(&QName::new("urn:other", "PrinterServiceType")), None);
        assert_eq!(ServiceKind::from_namespace("urn:other"), None);
        assert_ne!(print.name, scan.name);
    }
}
