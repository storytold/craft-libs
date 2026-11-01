//! WS-Addressing endpoint references (EPRs): where a service, a subscription manager or an event
//! sink lives, plus the reference parameters a sender must echo back as header blocks.
//!
//! Every piece of a reply is bounded: the address, the number and size of reference parameters.
//! Parameters that are not plain text elements are dropped, because the clients here only ever
//! echo simple identifiers (`wse:Identifier`).

use std::net::IpAddr;

use roxmltree::Node;

use crate::error::{Error, Result};
use crate::policy::DeviceUrl;
use crate::{ns, xml};

const MAX_ADDRESS: usize = 2048;
const MAX_PARAMS: usize = 8;
const MAX_PARAM_TEXT: usize = 512;

/// One reference parameter: a namespaced element with text content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceParameter {
    pub ns: String,
    pub local: String,
    pub text: String,
}

impl ReferenceParameter {
    /// A parameter element `{ns}local` with `text` as its content; nothing is validated until it is written.
    pub fn new(ns: impl Into<String>, local: impl Into<String>, text: impl Into<String>) -> Self {
        Self { ns: ns.into(), local: local.into(), text: text.into() }
    }

    /// Whether the parameter can be serialised: a non-empty namespace and an NCName local name.
    fn writable(&self) -> bool {
        xml::is_ncname(&self.local) && !self.ns.is_empty()
    }
}

/// An endpoint reference: an address and its reference parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointRef {
    pub address: String,
    pub reference_parameters: Vec<ReferenceParameter>,
}

impl EndpointRef {
    /// An endpoint reference to `address` with no reference parameters.
    pub fn new(address: impl Into<String>) -> Self {
        Self { address: address.into(), reference_parameters: Vec::new() }
    }

    /// Append a reference parameter, to be echoed as a header block by [`Self::header_xml`].
    pub fn with_parameter(mut self, p: ReferenceParameter) -> Self {
        self.reference_parameters.push(p);
        self
    }

    /// Read an EPR element (`wsa:EndpointReference`, `wse:NotifyTo`, ... any element with a
    /// `wsa:Address` child). Exactly one non-empty address is required; two are ambiguous and refused.
    pub fn parse(node: Node<'_, '_>) -> Result<Self> {
        let mut addresses = node.children().filter(|c| xml::is(*c, ns::WSA, "Address") || xml::is(*c, ns::WSA_FINAL, "Address"));
        let first = addresses.next().ok_or_else(|| Error::Protocol("an endpoint reference has no Address".into()))?;
        if addresses.next().is_some() {
            return Err(Error::Protocol("an endpoint reference has more than one Address".into()));
        }
        let address = xml::text(first);
        if address.is_empty() || address.len() > MAX_ADDRESS {
            return Err(Error::Protocol("an endpoint reference Address is empty or too long".into()));
        }
        let mut reference_parameters = Vec::new();
        for holder in node
            .children()
            .filter(|c| ["ReferenceParameters", "ReferenceProperties"].iter().any(|l| xml::is(*c, ns::WSA, l) || xml::is(*c, ns::WSA_FINAL, l)))
        {
            for p in holder.children().filter(|c| c.is_element()) {
                let simple = p.children().all(|c| !c.is_element());
                let (Some(pns), true) = (p.tag_name().namespace(), simple) else { continue };
                let text = xml::text(p);
                if text.len() <= MAX_PARAM_TEXT && reference_parameters.len() < MAX_PARAMS {
                    reference_parameters.push(ReferenceParameter::new(pns, p.tag_name().name(), text));
                }
            }
        }
        Ok(Self { address, reference_parameters })
    }

    /// The first reference parameter `{ns}local`, as text.
    pub fn parameter(&self, ns: &str, local: &str) -> Option<&str> {
        self.reference_parameters.iter().find(|p| p.ns == ns && p.local == local).map(|p| p.text.as_str())
    }

    /// The address as a policy-checked device URL: numeric private-network `http` only.
    ///
    /// This knows nothing of who sent the reference. For an address read from a device's reply use
    /// [`Self::url_from`], which also refuses a loopback address from a non-loopback sender.
    pub fn url(&self) -> Result<DeviceUrl> {
        DeviceUrl::parse_as(&self.address, "endpoint")
    }

    /// [`Self::url`] for a reference that arrived in a message from `from`: a loopback address is
    /// kept only when `from` is loopback too, the rule [`crate::discovery::Target::device_urls`]
    /// applies, so a device cannot aim the caller at a service on this machine.
    pub fn url_from(&self, from: IpAddr) -> Result<DeviceUrl> {
        let url = self.url()?;
        if url.reachable_from(from) {
            Ok(url)
        } else {
            Err(Error::Policy(format!("endpoint address {url} is this machine, but the reply came from {from}", url = url.as_url())))
        }
    }

    /// The element `name` (a prefixed name such as `wse:NotifyTo`; its prefix must be declared by
    /// the enclosing envelope, and `wsa` always is) holding this reference.
    pub fn to_xml(&self, name: &str) -> String {
        let params = self.header_xml();
        let params = if params.is_empty() { String::new() } else { format!("<wsa:ReferenceParameters>{params}</wsa:ReferenceParameters>") };
        format!("<{name}><wsa:Address>{}</wsa:Address>{params}</{name}>", xml::esc(&self.address))
    }

    /// The reference parameters as header blocks, each declaring its own namespace; what a message
    /// sent to this endpoint must carry in its SOAP header. Unwritable names are skipped.
    pub fn header_xml(&self) -> String {
        self.reference_parameters
            .iter()
            .filter(|p| p.writable())
            .enumerate()
            .map(|(i, p)| {
                format!("<rp{i}:{local} xmlns:rp{i}=\"{ns}\">{text}</rp{i}:{local}>", local = p.local, ns = xml::esc(&p.ns), text = xml::esc(&p.text))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WSE: &str = ns::WSE;

    /// Parse `inner` as the children of a wrapper element that declares `wsa` and `wse`.
    fn parse(inner: &str) -> Result<EndpointRef> {
        let text = format!("<e xmlns:wsa=\"{}\" xmlns:wse=\"{WSE}\">{inner}</e>", ns::WSA);
        let doc = xml::parse(&text).unwrap();
        EndpointRef::parse(doc.root_element())
    }

    #[test]
    fn an_address_and_reference_parameters_are_read() {
        let e = parse("<wsa:Address> http://127.0.0.1:5357/m </wsa:Address><wsa:ReferenceParameters><wse:Identifier>uuid:1</wse:Identifier><x:Ctx xmlns:x=\"urn:x\">c1</x:Ctx></wsa:ReferenceParameters>").unwrap();
        assert_eq!(e.address, "http://127.0.0.1:5357/m");
        assert_eq!(e.parameter(WSE, "Identifier"), Some("uuid:1"));
        assert_eq!(e.parameter("urn:x", "Ctx"), Some("c1"));
        assert_eq!(e.parameter(WSE, "Nope"), None);
        assert!(e.url().is_ok());
        // A loopback address is usable from a loopback sender only.
        assert!(e.url_from(IpAddr::from([127, 0, 0, 1])).is_ok());
        assert!(e.url_from("::ffff:127.0.0.1".parse().unwrap()).is_ok());
        assert!(matches!(e.url_from(IpAddr::from([192, 168, 1, 9])), Err(Error::Policy(_))));
        let lan = parse("<wsa:Address>http://192.168.1.20/m</wsa:Address>").unwrap();
        assert!(lan.url_from(IpAddr::from([192, 168, 1, 9])).is_ok());
        // The 2004/08 spelling and the final WS-Addressing namespace both read.
        let props =
            parse("<wsa:Address>urn:a</wsa:Address><wsa:ReferenceProperties><wse:Identifier>p</wse:Identifier></wsa:ReferenceProperties>").unwrap();
        assert_eq!(props.parameter(WSE, "Identifier"), Some("p"));
        let fin = format!("<e xmlns:a=\"{}\"><a:Address>urn:b</a:Address></e>", ns::WSA_FINAL);
        let doc = xml::parse(&fin).unwrap();
        assert_eq!(EndpointRef::parse(doc.root_element()).unwrap().address, "urn:b");
    }

    #[test]
    fn missing_duplicate_empty_and_oversized_addresses_are_refused() {
        assert!(matches!(parse(""), Err(Error::Protocol(_))));
        assert!(matches!(parse("<wsa:Address/>"), Err(Error::Protocol(_))));
        assert!(matches!(parse("<wsa:Address>a</wsa:Address><wsa:Address>b</wsa:Address>"), Err(Error::Protocol(_))));
        assert!(matches!(parse(&format!("<wsa:Address>{}</wsa:Address>", "a".repeat(3000))), Err(Error::Protocol(_))));
        // An Address in a foreign namespace is not an Address.
        assert!(matches!(parse("<x:Address xmlns:x=\"urn:x\">a</x:Address>"), Err(Error::Protocol(_))));
    }

    #[test]
    fn hostile_reference_parameters_are_bounded_or_dropped() {
        let many: String = (0..50).map(|i| format!("<p{i} xmlns=\"urn:p\">v</p{i}>")).collect();
        let e = parse(&format!("<wsa:Address>a</wsa:Address><wsa:ReferenceParameters>{many}</wsa:ReferenceParameters>")).unwrap();
        assert_eq!(e.reference_parameters.len(), MAX_PARAMS);
        let big = "x".repeat(MAX_PARAM_TEXT + 1);
        let e = parse(&format!(
            "<wsa:Address>a</wsa:Address><wsa:ReferenceParameters><wse:Identifier>{big}</wse:Identifier><wse:Nested><y xmlns=\"urn:y\"/></wse:Nested><plain>no namespace</plain></wsa:ReferenceParameters>"
        ))
        .unwrap();
        assert!(e.reference_parameters.is_empty(), "{e:?}");
    }

    #[test]
    fn written_references_round_trip_and_escape() {
        let e = EndpointRef::new("http://127.0.0.1:1/a?b=<&>")
            .with_parameter(ReferenceParameter::new(WSE, "Identifier", "uuid:<1>"))
            .with_parameter(ReferenceParameter::new("urn:x", "bad name", "dropped"))
            .with_parameter(ReferenceParameter::new("", "NoNs", "dropped"));
        let text = format!("<env xmlns:wsa=\"{}\" xmlns:wse=\"{WSE}\">{}</env>", ns::WSA, e.to_xml("wse:NotifyTo"));
        let doc = xml::parse(&text).unwrap();
        let back = EndpointRef::parse(xml::child(doc.root_element(), WSE, "NotifyTo").unwrap()).unwrap();
        assert_eq!(back.address, e.address);
        assert_eq!(back.reference_parameters, vec![ReferenceParameter::new(WSE, "Identifier", "uuid:<1>")]);
        let hdr = format!("<h>{}</h>", e.header_xml());
        let doc = xml::parse(&hdr).unwrap();
        assert_eq!(xml::text(xml::child(doc.root_element(), WSE, "Identifier").unwrap()), "uuid:<1>");
        assert_eq!(EndpointRef::new("urn:a").to_xml("wse:EndTo"), "<wse:EndTo><wsa:Address>urn:a</wsa:Address></wse:EndTo>");
    }

    #[test]
    fn urls_follow_the_device_policy() {
        assert!(EndpointRef::new("http://8.8.8.8/x").url().is_err());
        assert!(EndpointRef::new("http://printer.local/x").url().is_err());
        assert!(EndpointRef::new("urn:uuid:1").url().is_err());
        assert!(EndpointRef::new("http://192.168.1.5:80/x").url().is_ok());
    }
}
