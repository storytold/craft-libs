// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Whether a signer's certificate rests on a trust anchor (RFC 5280 §6, as S/MIME needs it): a
//! path from it through certification authorities' certificates that came with the message, to
//! a certificate the host trusts. Each link's signature is checked, and every certificate on the
//! path must be valid at the time of the check and keep its authorities' constraints. Bounded: a
//! path is at most [`MAX_PATH`] long, and the signatures checked come out of the message's
//! budget ([`crate::MAX_VERIFICATIONS`]).

use std::collections::HashMap;

use mailcraft_model::digital_signature::Why;
use mailcraft_time::Timestamp;

use crate::algorithms::Fault;
use crate::cert::{Cert, Constraints, dir_within, mail_within};

/// The most certificates between a signer's and a trust anchor.
pub const MAX_PATH: usize = 6;
/// The most trust anchors kept.
pub const MAX_ANCHORS: usize = 4_000;

/// The certificates the host trusts (the system's certification authorities, handed in: this
/// crate never reads a store of its own).
#[derive(Clone, Debug, Default)]
pub struct Anchors {
    certs: Vec<Cert>,
    by_subject: HashMap<Vec<u8>, Vec<usize>>,
}

impl Anchors {
    /// Anchors from certificates in DER; one that isn't a certificate is left out, as is
    /// everything past [`MAX_ANCHORS`].
    pub fn new<I, B>(ders: I) -> Anchors
    where
        I: IntoIterator<Item = B>,
        B: AsRef<[u8]>,
    {
        let mut anchors = Anchors::default();
        for der in ders {
            if anchors.certs.len() >= MAX_ANCHORS {
                break;
            }
            if let Some(cert) = Cert::from_der(der.as_ref()) {
                anchors.by_subject.entry(cert.subject().to_vec()).or_default().push(anchors.certs.len());
                anchors.certs.push(cert);
            }
        }
        anchors
    }

    pub fn len(&self) -> usize {
        self.certs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.certs.is_empty()
    }

    fn named(&self, subject: &[u8]) -> impl Iterator<Item = &Cert> {
        self.by_subject.get(subject).into_iter().flatten().filter_map(|i| self.certs.get(*i))
    }

    fn holds(&self, cert: &Cert) -> bool {
        self.named(cert.subject()).any(|a| a == cert)
    }
}

/// Whether `leaf` rests on one of `anchors` through the certificates of `pool`, at `now`,
/// checking at most `budget` signatures (taken from it). Every path to an anchor is tried until
/// one holds (RFC 5280 §6.1): each certificate on it valid at `now`, none with a critical
/// extension not known here, each authority's path length and name constraints kept, and every
/// authority's extended key usage, when it has one, allowing mail.
///
/// (Validity is checked at the time of the check, not the signing time the signer gives: that is
/// the signer's own word, and a certificate that ran out or was replaced since is said.)
pub fn trust(leaf: &Cert, pool: &[Cert], anchors: &Anchors, now: Timestamp, budget: &mut usize) -> Result<(), Why> {
    let mut search = Search { leaf, pool, anchors, now, budget, weak: false, refused: None, ran_out: false };
    let mut path = Vec::new();
    if search.walk(leaf, &mut path) {
        return Ok(());
    }
    Err(match search.refused {
        // (A path left untried may hold: no answer, rather than the first refusal's.)
        _ if search.ran_out => Why::TooMuch,
        Some(why) => why,
        None if search.weak => Why::WeakAlgorithm,
        None if leaf.self_issued() => Why::SelfSigned,
        None => Why::UnknownIssuer,
    })
}

struct Search<'a, 'b> {
    leaf: &'a Cert,
    pool: &'a [Cert],
    anchors: &'a Anchors,
    now: Timestamp,
    budget: &'b mut usize,
    /// A link was signed with an algorithm that's no longer safe.
    weak: bool,
    /// Why the first path found to an anchor didn't hold.
    refused: Option<Why>,
    /// The budget ran out before every path was tried.
    ran_out: bool,
}

impl<'a> Search<'a, '_> {
    /// Whether `cert` rests on an anchor by a path that holds; `path` has the certificates above
    /// the leaf so far, the anchor last once one is reached.
    fn walk(&mut self, cert: &'a Cert, path: &mut Vec<&'a Cert>) -> bool {
        // A certificate the host trusts itself ends the path, whether it holds or not.
        if self.anchors.holds(cert) {
            return self.holds(path);
        }
        for anchor in self.anchors.named(cert.issuer()) {
            if self.signed(cert, anchor) {
                path.push(anchor);
                if self.holds(path) {
                    return true;
                }
                path.pop();
            }
        }
        if path.len() >= MAX_PATH {
            return false;
        }
        for issuer in self.pool.iter().filter(|c| c.subject() == cert.issuer() && *c != cert && c.is_ca()) {
            // (No certificate twice on a path.)
            if path.contains(&issuer) || !self.signed(cert, issuer) {
                continue;
            }
            path.push(issuer);
            if self.walk(issuer, path) {
                return true;
            }
            path.pop();
        }
        false
    }

    /// Whether the path from the leaf through `above` (the anchor last) holds; why not is kept.
    fn holds(&mut self, above: &[&Cert]) -> bool {
        let full: Vec<&Cert> = std::iter::once(self.leaf).chain(above.iter().copied()).collect();
        match judge(&full, self.now) {
            Ok(()) => true,
            Err(why) => {
                self.refused.get_or_insert(why);
                false
            }
        }
    }

    fn signed(&mut self, cert: &Cert, by: &Cert) -> bool {
        if *self.budget == 0 {
            self.ran_out = true;
            return false;
        }
        *self.budget -= 1;
        match cert.signed_by(by) {
            Ok(()) => true,
            Err(Fault::Weak) => {
                self.weak = true;
                false
            }
            Err(_) => false,
        }
    }
}

/// Whether the path `full` (the leaf first, the anchor last) holds at `now` (RFC 5280 §6.1).
pub(crate) fn judge(full: &[&Cert], now: Timestamp) -> Result<(), Why> {
    if full.iter().any(|c| !c.valid_at(now)) {
        return Err(Why::Expired);
    }
    // (The anchor is the host's to vouch for, extensions and all.)
    let last = full.len().saturating_sub(1);
    if full.iter().take(last).any(|c| c.unknown_critical()) {
        return Err(Why::Constrained);
    }
    for (k, ca) in full.iter().enumerate().skip(1) {
        if !ca.may_vouch_for_mail() {
            return Err(Why::Constrained);
        }
        // The certificates below it that count: the leaf, and every authority's not issued by
        // itself (§6.1.4 (l), §6.1.3 (b)).
        let below: Vec<&Cert> = full.iter().take(k).enumerate().filter(|(j, c)| *j == 0 || !c.self_issued()).map(|(_, c)| *c).collect();
        if let Some(n) = ca.path_len()
            && below.len().saturating_sub(1) > usize::from(n)
        {
            return Err(Why::Constrained);
        }
        let Ok(constraints) = ca.constraints() else { return Err(Why::Constrained) };
        if let Some(c) = constraints
            && !below.iter().all(|cert| within(cert, &c))
        {
            return Err(Why::Constrained);
        }
    }
    Ok(())
}

/// Whether the names of `cert` are within `c`.
fn within(cert: &Cert, c: &Constraints) -> bool {
    let mail = cert.raw_addresses();
    if let Some(permitted) = &c.permitted_mail
        && !mail.iter().all(|a| permitted.iter().any(|p| mail_within(a, p)))
    {
        return false;
    }
    if mail.iter().any(|a| c.excluded_mail.iter().any(|e| mail_within(a, e))) {
        return false;
    }
    let rdns = cert.subject_rdns();
    if rdns.is_empty() {
        return true;
    }
    if let Some(permitted) = &c.permitted_dirs
        && !permitted.iter().any(|p| dir_within(&rdns, p))
    {
        return false;
    }
    !c.excluded_dirs.iter().any(|e| dir_within(&rdns, e))
}
