// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Checking CMS SignedData (RFC 5652 §5) as S/MIME carries it (RFC 8551 §3.5): the signer's
//! digest of what was signed, their signature over it, and whether their certificate is trusted
//! and for the message's sender.

use cms::signed_data::{EncapsulatedContentInfo, SignerIdentifier, SignerInfo};
use const_oid::ObjectIdentifier;
use der::asn1::OctetString;
use der::{Decode, Encode};
use mailcraft_model::digital_signature::{Coverage, DigitalSignature, SignatureFormat, Signer, Verdict, Why};
use mailcraft_time::Timestamp;

use crate::algorithms::{self, Fault, Hash};
use crate::cert::Cert;
use crate::chain::{self, Anchors};
use mailcraft_ber::{self as ber, Tlv};

/// The most certificates read from one message.
pub const MAX_CERTS: usize = 32;
/// The most signers of one message that are checked.
pub const MAX_SIGNERS: usize = 8;
/// The most signatures checked for one message, its signers' and their certificates' together
/// (#30): an RSA key of 8192 bits takes a while, and a message may carry many.
pub const MAX_VERIFICATIONS: usize = 32;
/// The most certificates tried for one signer: those that its identifier names (the message
/// may carry several of one name; only the one whose key made the signature is the signer's).
pub const MAX_CANDIDATES: usize = 4;

const SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");

/// What a check is made against.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    /// The certificates the host trusts.
    pub anchors: &'a Anchors,
    /// The message's From address.
    pub from: &'a str,
    /// The time of the check, seconds since 1970.
    pub now: Timestamp,
    /// Certificates kept for the sender already, in DER: a message that doesn't carry its
    /// signer's certificate may be checked with one of them.
    pub known: &'a [&'a [u8]],
    /// How much of the message the signed part is: the whole of it only when it is the
    /// message's top level. A part is never verified for the message.
    pub coverage: Coverage,
}

/// A Digitally signed message's two halves.
#[derive(Clone, Copy, Debug)]
pub enum Signed<'a> {
    /// `multipart/signed`: the signed part as it came (its MIME header and body) and the
    /// signature part's bytes (the CMS SignedData, decoded from base64).
    Detached { content: &'a [u8], signature: &'a [u8] },
    /// `application/pkcs7-mime; smime-type=signed-data`: the CMS SignedData holding what was
    /// signed (decoded from base64).
    Opaque(&'a [u8]),
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    pub signature: DigitalSignature,
    /// The signer's certificate, in DER, when the signature over the message is the signer's
    /// and the certificate is for the From address: worth keeping for that address (#30 R9).
    pub certificate: Option<Vec<u8>>,
}

/// Check `signed` against `context`. `None` when the CMS isn't SignedData at all (an Encrypted
/// message's enveloped data, say): not a Digital signature. Never fails otherwise: what can't be
/// read is [`Verdict::Unchecked`].
pub fn check(signed: Signed<'_>, context: &Context<'_>) -> Option<Checked> {
    check_within(signed, context, MAX_VERIFICATIONS)
}

/// [`check`] with at most `verifications` signatures checked (for the tests, which spend a
/// smaller budget).
pub(crate) fn check_within(signed: Signed<'_>, context: &Context<'_>, verifications: usize) -> Option<Checked> {
    let blob = match signed {
        Signed::Detached { signature, .. } => signature,
        Signed::Opaque(blob) => blob,
    };
    let unreadable = || Some(unchecked(Why::Unreadable));
    let Some(der) = ber::to_der(blob) else { return unreadable() };
    let Some(parts) = content_info(&der) else { return unreadable() };
    if parts.content_type != SIGNED_DATA {
        return None;
    }
    let Some(data) = signed_data(parts.content) else { return unreadable() };
    let Ok(eci) = EncapsulatedContentInfo::from_der(data.encap.whole) else { return unreadable() };
    // What mail signs is MIME, id-data (RFC 8551 §3.5); a detached signature carries none of it
    // and an opaque one all of it.
    if eci.econtent_type != DATA {
        return unreadable();
    }
    let content: &[u8] = match (signed, econtent(&data.encap)) {
        (Signed::Detached { content, .. }, None) if eci.econtent.is_none() => content,
        (Signed::Opaque(_), Some(c)) => c,
        _ => return unreadable(),
    };
    let mut pool: Vec<Cert> = data.certificates.iter().take(MAX_CERTS).filter_map(|c| Cert::from_der(c.whole)).collect();
    pool.extend(context.known.iter().take(MAX_CERTS).filter_map(|c| Cert::from_der(c)));
    let mut digests = Digests::new(content);
    let mut budget = verifications;
    let mut best: Option<Checked> = None;
    for signer in data.signers.iter().take(MAX_SIGNERS) {
        let found = one_signer(signer, &eci, &mut digests, &pool, context, &mut budget);
        best = Some(match best {
            Some(b) if b.signature.verdict <= found.signature.verdict => b,
            _ => found,
        });
    }
    best.or_else(unreadable).map(|mut c| {
        c.signature = c.signature.covering(context.coverage);
        c
    })
}

/// What was signed and its digests, each worked out once (#30: a long message isn't hashed
/// again for every signer and key tried), as it came and in its canonical form.
struct Digests<'a> {
    content: &'a [u8],
    /// The content with lone LFs made CRLF, when it has any: worked out at its first use.
    canonical: Option<Option<Vec<u8>>>,
    plain: Vec<(Hash, Vec<u8>)>,
    crlf: Vec<(Hash, Vec<u8>)>,
}

impl<'a> Digests<'a> {
    fn new(content: &'a [u8]) -> Self {
        Digests { content, canonical: None, plain: Vec::new(), crlf: Vec::new() }
    }

    fn of(&mut self, hash: Hash) -> Vec<u8> {
        if let Some((_, d)) = self.plain.iter().find(|(h, _)| *h == hash) {
            return d.clone();
        }
        let d = hash.digest(self.content);
        self.plain.push((hash, d.clone()));
        d
    }

    /// The digest of the canonical form; `None` when the content has no lone LF (it is its own).
    fn of_canonical(&mut self, hash: Hash) -> Option<Vec<u8>> {
        if let Some((_, d)) = self.crlf.iter().find(|(h, _)| *h == hash) {
            return Some(d.clone());
        }
        let content = self.content;
        let canonical = self.canonical.get_or_insert_with(|| canonical(content)).as_ref()?;
        let d = hash.digest(canonical);
        self.crlf.push((hash, d.clone()));
        Some(d)
    }

    /// Whether `said` is the digest of the content as it came or in its canonical form.
    fn matches(&mut self, hash: Hash, said: &[u8]) -> bool {
        self.of(hash) == said || self.of_canonical(hash).is_some_and(|d| d == said)
    }
}

fn unchecked(why: Why) -> Checked {
    Checked { signature: DigitalSignature::bare(SignatureFormat::Smime, Verdict::Unchecked, why), certificate: None }
}

struct ContentInfoParts<'a> {
    content_type: ObjectIdentifier,
    content: &'a [u8],
}

/// ContentInfo: its type, and the value inside its `[0] EXPLICIT`.
fn content_info(der: &[u8]) -> Option<ContentInfoParts<'_>> {
    let (outer, rest) = ber::read(der)?;
    if outer.tag != 0x30 || !rest.is_empty() {
        return None;
    }
    let fields = ber::children(outer.value)?;
    let [oid, explicit] = fields.as_slice() else { return None };
    if oid.tag != 0x06 || explicit.tag != 0xa0 {
        return None;
    }
    let content_type = ObjectIdentifier::from_der(oid.whole).ok()?;
    Some(ContentInfoParts { content_type, content: explicit.value })
}

/// SignedData's parts as they came, so that signatures are checked over the bytes the signer
/// wrote: what is inside, the certificates, and the signers.
struct SignedDataParts<'a> {
    encap: Tlv<'a>,
    certificates: Vec<Tlv<'a>>,
    signers: Vec<Tlv<'a>>,
}

/// SignedData read strictly (RFC 5652 §5.1): its version one of those defined, its digest
/// algorithms a SET of algorithm identifiers, encapContentInfo, the certificates and the
/// revocation lists when they are there, in that order, and the signers last: nothing else, and
/// nothing after.
fn signed_data(content: &[u8]) -> Option<SignedDataParts<'_>> {
    let (outer, rest) = ber::read(content)?;
    if outer.tag != 0x30 || !rest.is_empty() {
        return None;
    }
    let fields = ber::children(outer.value)?;
    let mut fields = fields.into_iter();
    let version = fields.next().filter(|f| f.tag == 0x02)?;
    if !matches!(version.value, [1] | [3] | [4] | [5]) {
        return None;
    }
    let digests = fields.next().filter(|f| f.tag == 0x31)?;
    for d in ber::children(digests.value)? {
        spki::AlgorithmIdentifierOwned::from_der(d.whole).ok()?;
    }
    let encap = fields.next().filter(|f| f.tag == 0x30)?;
    let mut next = fields.next()?;
    let mut certificates = Vec::new();
    if next.tag == 0xa0 {
        // (Other kinds of certificate, [0]..[3] IMPLICIT, are left out; they are never a path.)
        certificates = ber::children(next.value)?.into_iter().filter(|c| c.tag == 0x30).collect();
        next = fields.next()?;
    }
    if next.tag == 0xa1 {
        next = fields.next()?;
    }
    if next.tag != 0x31 || fields.next().is_some() {
        return None;
    }
    Some(SignedDataParts { encap, certificates, signers: ber::children(next.value)? })
}

/// The OCTET STRING inside encapContentInfo's `[0] EXPLICIT eContent`, when there is one (and
/// nothing else is there).
fn econtent<'a>(encap: &Tlv<'a>) -> Option<&'a [u8]> {
    let fields = ber::children(encap.value)?;
    let explicit = fields.get(1).filter(|f| f.tag == 0xa0)?;
    let (octets, rest) = ber::read(explicit.value)?;
    (octets.tag == 0x04 && rest.is_empty()).then_some(octets.value)
}

/// The value of the signed attribute `oid`, in DER (its one value), when there is one.
fn attribute(info: &SignerInfo, oid: ObjectIdentifier) -> Option<Vec<u8>> {
    let attrs = info.signed_attrs.as_ref()?;
    let attr = attrs.iter().find(|a| a.oid == oid)?;
    if attr.values.len() != 1 {
        return None;
    }
    attr.values.iter().next()?.to_der().ok()
}

/// The bytes a signer with signed attributes signed (RFC 5652 §5.4): the attributes as they came,
/// with their `[0] IMPLICIT` tag made the SET OF's.
fn signed_attributes(raw: &Tlv<'_>) -> Option<Vec<u8>> {
    let fields = ber::children(raw.value)?;
    let attrs = fields.iter().find(|f| f.tag == 0xa0)?;
    let mut out = attrs.whole.to_vec();
    *out.first_mut()? = 0x31;
    Some(out)
}

/// `content` with every lone LF made CRLF (RFC 8551 §3.1.1's canonical form), when it has one.
fn canonical(content: &[u8]) -> Option<Vec<u8>> {
    let mut lone = false;
    let mut out = Vec::with_capacity(content.len().saturating_add(content.len() / 32));
    let mut previous = 0u8;
    for &b in content {
        if b == b'\n' && previous != b'\r' {
            out.push(b'\r');
            lone = true;
        }
        out.push(b);
        previous = b;
    }
    lone.then_some(out)
}

fn one_signer(
    raw: &Tlv<'_>,
    eci: &EncapsulatedContentInfo,
    digests: &mut Digests<'_>,
    pool: &[Cert],
    context: &Context<'_>,
    budget: &mut usize,
) -> Checked {
    let Ok(info) = SignerInfo::from_der(raw.whole) else { return unchecked(Why::Unreadable) };
    let hash = match Hash::of(&info.digest_alg) {
        Ok(hash) => hash,
        Err(fault) => return unchecked(why_not(fault)),
    };
    let candidates = signer_certs(&info.sid, pool);
    if candidates.is_empty() {
        return unchecked(Why::NoPublicKey);
    }
    let signature = info.signature.as_bytes();
    // What the signer's key signed: the signed attributes (which hold the content's digest), or
    // without them the content itself, as it came or in its canonical form.
    let attributes = match &info.signed_attrs {
        Some(_) => match signed_attributes(raw) {
            Some(bytes) => Some(bytes),
            None => return unchecked(Why::Unreadable),
        },
        None => None,
    };
    // The signer is the certificate whose key made the signature, of those its identifier names:
    // another one of the same name (any can be put in a message) makes no difference.
    let mut fault: Option<Fault> = None;
    let mut signer: Option<&Cert> = None;
    let mut ran_out = false;
    for cert in candidates {
        let Some(left) = budget.checked_sub(1) else {
            ran_out = true;
            break;
        };
        *budget = left;
        let verified = match &attributes {
            Some(bytes) => algorithms::verify(cert.public_key(), &info.signature_algorithm, Some(hash), bytes, signature),
            None => {
                let plain = algorithms::verify_hashed(cert.public_key(), &info.signature_algorithm, Some(hash), &mut |h| digests.of(h), signature);
                // (Signed in its canonical form, perhaps: another verification, from the budget.)
                if plain == Err(Fault::Invalid) && digests.of_canonical(hash).is_some() {
                    let Some(left) = budget.checked_sub(1) else {
                        ran_out = true;
                        break;
                    };
                    *budget = left;
                    algorithms::verify_hashed(
                        cert.public_key(),
                        &info.signature_algorithm,
                        Some(hash),
                        &mut |h| digests.of_canonical(h).unwrap_or_default(),
                        signature,
                    )
                } else {
                    plain
                }
            }
        };
        match verified {
            Ok(()) => {
                signer = Some(cert);
                break;
            }
            // (Of several faults, what says most: an algorithm, then a malformed key, then no
            // match.)
            Err(f) => fault = Some(fault.map_or(f, |g| worse(g, f))),
        }
    }
    let Some(cert) = signer else {
        return match fault {
            // (A certificate not tried may have made it: no answer.)
            _ if ran_out => unchecked(Why::TooMuch),
            // No key it names made it: changed after signing, or never signed by any of them;
            // nobody is named, since none of them fits.
            Some(Fault::Invalid) | None => {
                Checked { signature: DigitalSignature::bare(SignatureFormat::Smime, Verdict::Changed, Why::Altered), certificate: None }
            }
            Some(f) => unchecked(why_not(f)),
        };
    };
    let result = |verdict: Verdict, why: Option<Why>| Checked {
        signature: DigitalSignature {
            format: SignatureFormat::Smime,
            verdict,
            why,
            signer: Some(signer_of(cert)),
            signed_at: signing_time(&info),
            revocation_checked: false,
            // (`check` says, from where in the message the signed part sat.)
            coverage: Coverage::Part,
        },
        certificate: None,
    };
    if attributes.is_some() {
        // The attributes are the signer's own words now: what they say of the content holds.
        let Some(said_type) = attribute(&info, CONTENT_TYPE).and_then(|d| ObjectIdentifier::from_der(&d).ok()) else {
            return unchecked(Why::Unreadable);
        };
        let Some(said_digest) = attribute(&info, MESSAGE_DIGEST).and_then(|d| OctetString::from_der(&d).ok()) else {
            return unchecked(Why::Unreadable);
        };
        if said_type != eci.econtent_type || !digests.matches(hash, said_digest.as_bytes()) {
            return result(Verdict::Changed, Some(Why::Altered));
        }
    }
    // (An authority's certificate never signs mail, whatever else it allows.)
    let for_email = cert.for_email() && !cert.is_authority();
    let trust = chain::trust(cert, pool, context.anchors, context.now, budget)
        .and_then(|()| for_email.then_some(()).ok_or(Why::NotForEmail))
        .and_then(|()| if cert.is_for(context.from) { Ok(()) } else { Err(Why::AddressMismatch) });
    let mut r = match trust {
        Ok(()) => result(Verdict::Verified, None),
        Err(Why::TooMuch) => result(Verdict::Unchecked, Some(Why::TooMuch)),
        // (An algorithm that's no longer safe on the way to a trust anchor: the certificate can't
        // be relied on, but the message is as it was signed.)
        Err(why) => result(Verdict::NotVerified, Some(why)),
    };
    // Kept for the sender only when it is trusted, valid now and theirs: it is what mail to them
    // will be encrypted with.
    if trust.is_ok() {
        r.certificate = Some(cert.der().to_vec());
    }
    // The signer's address that is the sender's comes first: it is the one said.
    if let (Some(address), Some(signer)) = (cert.address_for(context.from), r.signature.signer.as_mut()) {
        let shown = mailcraft_model::digital_signature::signer_text(&address);
        signer.addresses.retain(|a| !a.eq_ignore_ascii_case(&shown));
        signer.addresses.insert(0, shown);
    }
    r
}

/// Of two faults, the one that says most of why a signature can't be relied on.
fn worse(a: Fault, b: Fault) -> Fault {
    let rank = |f: Fault| match f {
        Fault::Weak => 0,
        Fault::Unknown => 1,
        Fault::Malformed => 2,
        Fault::Invalid => 3,
    };
    if rank(b) < rank(a) { b } else { a }
}

fn why_not(fault: Fault) -> Why {
    match fault {
        Fault::Weak => Why::WeakAlgorithm,
        Fault::Unknown => Why::UnknownAlgorithm,
        Fault::Invalid => Why::Altered,
        Fault::Malformed => Why::Unreadable,
    }
}

/// The certificates in `pool` that `sid` names, at most [`MAX_CANDIDATES`].
fn signer_certs<'a>(sid: &SignerIdentifier, pool: &'a [Cert]) -> Vec<&'a Cert> {
    match sid {
        SignerIdentifier::IssuerAndSerialNumber(isn) => {
            let Ok(issuer) = isn.issuer.to_der() else { return Vec::new() };
            let serial = isn.serial_number.as_bytes();
            pool.iter().filter(|c| c.issuer() == issuer.as_slice() && c.serial() == serial).take(MAX_CANDIDATES).collect()
        }
        SignerIdentifier::SubjectKeyIdentifier(ski) => {
            let id = ski.0.as_bytes();
            pool.iter().filter(|c| c.key_id().as_deref() == Some(id)).take(MAX_CANDIDATES).collect()
        }
    }
}

fn signer_of(cert: &Cert) -> Signer {
    let (not_before, not_after) = cert.validity();
    Signer { name: cert.name(), addresses: cert.addresses(), fingerprint: cert.fingerprint(), issuer: cert.issuer_name(), not_before, not_after }
}

/// The signing time the signer gives (RFC 5652 §11.3): their word, unchecked.
fn signing_time(info: &SignerInfo) -> Option<Timestamp> {
    let der = attribute(info, SIGNING_TIME)?;
    let time = x509_cert::time::Time::from_der(&der).ok()?;
    Timestamp::try_from(time.to_unix_duration().as_secs()).ok()
}
