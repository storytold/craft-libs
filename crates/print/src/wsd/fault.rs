//! SOAP faults from a print service, as a typed error.
//!
//! The structure is read by the WSD core's fault parser; this module classifies the sub-code.
//!
//! The service reports problems as a SOAP 1.2 fault whose sub-code names the cause: fifteen codes
//! in the `wprt` namespace plus WS-Addressing's `ActionNotSupported`. A vendor may send others;
//! those arrive as [`FaultCode::Other`] and are never an error in themselves.

use std::fmt;

use roxmltree::Node;

use super::ns;

/// The sub-code of a fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FaultCode {
    /// `wsa:ActionNotSupported`: the endpoint does not know the action.
    ActionNotSupported,
    InvalidArgs,
    OperationFailed,
    ServerErrorTemporaryError,
    ServerErrorInternalError,
    ServerErrorNotAcceptingJobs,
    ClientErrorJobIdNotFound,
    ClientErrorMultipleDocumentsNotSupported,
    ClientErrorLastDocumentAlreadySent,
    ClientErrorFormatNotSupported,
    ClientErrorCompressionNotSupported,
    ClientErrorDocumentURLSchemeNotSupported,
    ClientErrorDocumentURLNotFound,
    ServerErrorAddDocumentNotSupported,
    ServerErrorDocumentActive,
    ServerErrorJobCancelled,
    /// Documented for conflicting MustHonor settings but missing from the schema's list.
    ClientErrorConflictingRequiredParameters,
    /// Anything else: the namespace (if any) and local name.
    Other(Option<String>, String),
}

/// The `wprt` sub-codes, in the order the service description lists them.
pub static WPRT_CODES: [(&str, FaultCode); 16] = [
    ("InvalidArgs", FaultCode::InvalidArgs),
    ("OperationFailed", FaultCode::OperationFailed),
    ("ServerErrorTemporaryError", FaultCode::ServerErrorTemporaryError),
    ("ServerErrorInternalError", FaultCode::ServerErrorInternalError),
    ("ServerErrorNotAcceptingJobs", FaultCode::ServerErrorNotAcceptingJobs),
    ("ClientErrorJobIdNotFound", FaultCode::ClientErrorJobIdNotFound),
    ("ClientErrorMultipleDocumentsNotSupported", FaultCode::ClientErrorMultipleDocumentsNotSupported),
    ("ClientErrorLastDocumentAlreadySent", FaultCode::ClientErrorLastDocumentAlreadySent),
    ("ClientErrorFormatNotSupported", FaultCode::ClientErrorFormatNotSupported),
    ("ClientErrorCompressionNotSupported", FaultCode::ClientErrorCompressionNotSupported),
    ("ClientErrorDocumentURLSchemeNotSupported", FaultCode::ClientErrorDocumentURLSchemeNotSupported),
    ("ClientErrorDocumentURLNotFound", FaultCode::ClientErrorDocumentURLNotFound),
    ("ServerErrorAddDocumentNotSupported", FaultCode::ServerErrorAddDocumentNotSupported),
    ("ServerErrorDocumentActive", FaultCode::ServerErrorDocumentActive),
    ("ServerErrorJobCancelled", FaultCode::ServerErrorJobCancelled),
    ("ClientErrorConflictingRequiredParameters", FaultCode::ClientErrorConflictingRequiredParameters),
];

impl FaultCode {
    /// Classify a resolved sub-code QName.
    pub fn from_qname(ns_uri: Option<&str>, local: &str) -> Self {
        if ns_uri == Some(ns::WPRT)
            && let Some((_, code)) = WPRT_CODES.iter().find(|(n, _)| *n == local)
        {
            return code.clone();
        }
        if local == "ActionNotSupported" && (ns_uri == Some(ns::WSA) || ns_uri == Some(ns::WSA_FINAL)) {
            return Self::ActionNotSupported;
        }
        Self::Other(ns_uri.map(str::to_owned), local.to_owned())
    }

    /// The local name of the sub-code.
    pub fn name(&self) -> &str {
        match self {
            Self::ActionNotSupported => "ActionNotSupported",
            Self::Other(_, local) => local,
            other => WPRT_CODES.iter().find(|(_, c)| c == other).map(|(n, _)| *n).unwrap_or("Unknown"),
        }
    }

    /// The code as a [`crate::NativeCode`] number and name: 1 to 16 in the order of [`WPRT_CODES`],
    /// 100 for `wsa:ActionNotSupported`, and 0 (no name) for a vendor code.
    pub fn native(&self) -> (i64, Option<&'static str>) {
        match self {
            Self::ActionNotSupported => (100, Some("ActionNotSupported")),
            Self::Other(..) => (0, None),
            other => match WPRT_CODES.iter().enumerate().find(|(_, (_, c))| c == other) {
                Some((i, (name, _))) => (i as i64 + 1, Some(*name)),
                None => (0, None),
            },
        }
    }

    /// May the same request succeed later without any change?
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::ServerErrorTemporaryError | Self::ServerErrorNotAcceptingJobs | Self::ServerErrorDocumentActive)
    }
}

pub use super::adapter::Role;

/// A parsed fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub role: Role,
    pub code: FaultCode,
    /// The first reason text (English, per the service description), capped.
    pub reason: String,
    /// The text of the detail, if any, capped.
    pub detail: String,
    /// The wait the printer asked for with an HTTP `Retry-After` on the reply that carried the fault.
    pub retry_after: Option<std::time::Duration>,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the printer refused: {}", self.code.name())?;
        if !self.reason.is_empty() {
            write!(f, " ({})", self.reason)?;
        }
        if !self.detail.is_empty() {
            write!(f, " [{}]", self.detail)?;
        }
        Ok(())
    }
}

impl From<super::adapter::WireFault> for Fault {
    /// Classify the generic fault's sub-code against the print codes.
    fn from(f: super::adapter::WireFault) -> Self {
        Self {
            role: f.role,
            code: FaultCode::from_qname(f.code.ns.as_deref(), &f.code.local),
            reason: f.reason,
            detail: f.detail,
            retry_after: f.retry_after,
        }
    }
}

/// Read a `soap:Fault` element. Missing parts degrade to `Other`/empty; this never fails.
pub fn parse_fault(fault: Node<'_, '_>) -> Fault {
    super::adapter::parse_fault(fault).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsd::adapter::xml;

    fn fault_xml(sub: &str, extra_ns: &str) -> String {
        format!(
            "<s:Envelope xmlns:s=\"{}\" xmlns:wprt=\"{}\" xmlns:wsa=\"{}\"{extra_ns}><s:Body><s:Fault><s:Code><s:Value>s:Sender</s:Value><s:Subcode><s:Value>{sub}</s:Value></s:Subcode></s:Code><s:Reason><s:Text xml:lang=\"en\">bad</s:Text></s:Reason><s:Detail><wprt:JobId>7</wprt:JobId></s:Detail></s:Fault></s:Body></s:Envelope>",
            ns::SOAP,
            ns::WPRT,
            ns::WSA
        )
    }

    fn parse_one(xml_text: &str) -> Fault {
        let doc = xml::parse(xml_text).unwrap();
        let f = doc.descendants().find(|n| xml::is(*n, ns::SOAP, "Fault")).unwrap();
        parse_fault(f)
    }

    #[test]
    fn every_wprt_subcode_and_action_not_supported_is_recognised() {
        for (name, code) in WPRT_CODES.iter() {
            let f = parse_one(&fault_xml(&format!("wprt:{name}"), ""));
            assert_eq!(&f.code, code, "{name}");
            assert_eq!(f.code.name(), *name);
            assert_eq!(f.role, Role::Sender);
            assert_eq!(f.reason, "bad");
            assert_eq!(f.detail, "7");
        }
        let f = parse_one(&fault_xml("wsa:ActionNotSupported", ""));
        assert_eq!(f.code, FaultCode::ActionNotSupported);
    }

    #[test]
    fn prefixes_do_not_matter_namespaces_do() {
        let f = parse_one(&fault_xml("q:ClientErrorJobIdNotFound", &format!(" xmlns:q=\"{}\"", ns::WPRT)));
        assert_eq!(f.code, FaultCode::ClientErrorJobIdNotFound);
        // A look-alike in another namespace is not a print fault.
        let f = parse_one(&fault_xml("q:InvalidArgs", " xmlns:q=\"urn:other\""));
        assert_eq!(f.code, FaultCode::Other(Some("urn:other".into()), "InvalidArgs".into()));
        // An undeclared prefix, an empty subcode and a missing one all degrade without panicking.
        assert!(matches!(parse_one(&fault_xml("zz:InvalidArgs", "")).code, FaultCode::Other(None, ref s) if s.is_empty()));
        assert!(matches!(parse_one(&fault_xml("", "")).code, FaultCode::Other(..)));
    }

    #[test]
    fn a_bare_fault_and_receiver_roles() {
        let bare = format!("<s:Envelope xmlns:s=\"{}\"><s:Body><s:Fault/></s:Body></s:Envelope>", ns::SOAP);
        let f = parse_one(&bare);
        assert_eq!((f.role, f.reason.as_str(), f.detail.as_str()), (Role::Other, "", ""));
        let recv = fault_xml("wprt:ServerErrorInternalError", "").replace("s:Sender", "s:Receiver");
        assert_eq!(parse_one(&recv).role, Role::Receiver);
        assert!(FaultCode::ServerErrorTemporaryError.is_retryable());
        assert!(!FaultCode::InvalidArgs.is_retryable());
        assert!(parse_one(&recv).to_string().contains("ServerErrorInternalError"));
    }

    #[test]
    fn a_generic_fault_is_classified_against_the_print_codes() {
        let generic = crate::wsd::adapter::WireFault::new(
            Role::Receiver,
            crate::wsd::adapter::SubCode::new(Some(ns::WPRT), "ServerErrorNotAcceptingJobs"),
            "busy",
            "",
        )
        .with_retry_after(Some(std::time::Duration::from_secs(9)));
        let f = Fault::from(generic);
        assert_eq!(f.retry_after, Some(std::time::Duration::from_secs(9)));
        assert_eq!(f.code, FaultCode::ServerErrorNotAcceptingJobs);
        assert!(f.code.is_retryable());
        assert_eq!(f.to_string(), "the printer refused: ServerErrorNotAcceptingJobs (busy)");
        let e: crate::wsd::error::WsdError = crate::wsd::adapter::WireError::from_fault(crate::wsd::adapter::WireFault::new(
            Role::Sender,
            crate::wsd::adapter::SubCode::new(Some(ns::WPRT), "InvalidArgs"),
            "",
            "",
        ))
        .into();
        assert_eq!(e.fault().map(|f| f.code.clone()), Some(FaultCode::InvalidArgs));
    }
}
