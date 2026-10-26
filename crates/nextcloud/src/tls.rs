//! TLS with trust on first use: the OS verifies certificates unless the account pins one.
//!
//! ureq has no hook for a custom verifier, and its rustls error drops the leaf certificate that trust on first
//! use must show, so this is a connector of its own. [`TofuTls`] and [`TlsTransport`] follow ureq 3.5.0's
//! `src/tls/rustls.rs` (MIT OR Apache-2.0, Copyright (c) 2019 Martin Algesten; see NOTICE).

use std::fmt;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, PoisonError};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};
use rustls_platform_verifier::Verifier;
use ureq::unversioned::transport::{Buffers, ConnectionDetails, Connector, Either, LazyBuffers, NextTimeout, Transport, TransportAdapter};

use crate::{Error, Fingerprint};

/// The account's pin and the leaf certificate the last handshake rejected, shared by the verifier and the error mapping.
#[derive(Clone, Debug, Default)]
pub(crate) struct TlsState {
    pin: Option<Fingerprint>,
    seen: Arc<Mutex<Option<Fingerprint>>>,
}

impl TlsState {
    pub(crate) fn new(pin: Option<Fingerprint>) -> TlsState {
        TlsState { pin, seen: Arc::default() }
    }

    /// What a certificate rejection means: a changed certificate when pinned, else the trust prompt for the leaf
    /// the last handshake rejected; `None` when no leaf was rejected (the certificate failed some other check).
    pub(crate) fn rejected(&self) -> Option<Error> {
        if self.pin.is_some() {
            return Some(Error::CertificateChanged);
        }
        self.seen.lock().unwrap_or_else(PoisonError::into_inner).map(|fingerprint| Error::UnknownCertificate { fingerprint })
    }

    fn see(&self, leaf: Option<Fingerprint>) {
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = leaf;
    }
}

/// Accepts exactly the pinned leaf, or else what the OS accepts, recording the leaf it rejects.
#[derive(Debug)]
struct TofuVerifier {
    state: TlsState,
    /// The OS verifier; `None` when pinned, for plain HTTP, or when the OS has no CA certificates.
    inner: Option<Verifier>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl TofuVerifier {
    /// Loads the OS verifier only for an unpinned `https` account; a failure to load it (no CA certificates)
    /// leaves every certificate unknown, so the user still gets the trust prompt.
    fn new(provider: &Arc<CryptoProvider>, state: &TlsState, https: bool) -> TofuVerifier {
        let inner = if https && state.pin.is_none() { Verifier::new(provider.clone()).ok() } else { None };
        TofuVerifier { state: state.clone(), inner, algorithms: provider.signature_verification_algorithms }
    }
}

impl ServerCertVerifier for TofuVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let leaf = Fingerprint::of(end_entity);
        self.state.see(None);
        match self.state.pin {
            // The user accepted this exact certificate, so its name and dates are not checked.
            Some(pin) if pin == leaf => Ok(ServerCertVerified::assertion()),
            Some(_) => Err(rustls::Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)),
            None => match &self.inner {
                Some(inner) => inner.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now),
                None => Err(rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer)),
            }
            .inspect_err(|_| self.state.see(Some(leaf))),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Wraps `https` connections in TLS checked by the trust-on-first-use verifier; passes `http` through.
#[derive(Debug)]
pub(crate) struct TofuTls(Arc<ClientConfig>);

impl TofuTls {
    /// `https`: whether the account's server is `https://`; plain HTTP never loads the OS verifier.
    pub(crate) fn new(provider: Arc<CryptoProvider>, state: &TlsState, https: bool) -> Result<TofuTls, Error> {
        let verifier = TofuVerifier::new(&provider, state, https);
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| Error::Unreachable(format!("TLS setup failed: {e}")))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        Ok(TofuTls(Arc::new(config)))
    }
}

impl<In: Transport> Connector<In> for TofuTls {
    type Out = Either<In, TlsTransport>;

    fn connect(&self, details: &ConnectionDetails, chained: Option<In>) -> Result<Option<Self::Out>, ureq::Error> {
        let Some(transport) = chained else { return Ok(None) };
        if !details.needs_tls() || transport.is_tls() {
            return Ok(Some(Either::A(transport)));
        }
        let host = details.uri.host().unwrap_or_default().trim_start_matches('[').trim_end_matches(']');
        let name = ServerName::try_from(host.to_string()).map_err(|_| ureq::Error::Tls("invalid TLS server name"))?;
        let mut conn = ClientConnection::new(self.0.clone(), name)?;
        let mut sock = TransportAdapter::new(Box::new(transport) as Box<dyn Transport>, details.current_time.clone());
        sock.set_timeout(details.timeout);
        conn.complete_io(&mut sock)?;
        let buffers = LazyBuffers::new(details.config.input_buffer_size(), details.config.output_buffer_size());
        Ok(Some(Either::B(TlsTransport { buffers, stream: StreamOwned::new(conn, sock) })))
    }
}

/// A TLS session over the chained transport.
pub(crate) struct TlsTransport {
    buffers: LazyBuffers,
    stream: StreamOwned<ClientConnection, TransportAdapter>,
}

impl Transport for TlsTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.stream.get_mut().set_timeout(timeout);
        let output = self.buffers.output();
        self.stream.write_all(&output[..amount.min(output.len())])?;
        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.stream.get_mut().set_timeout(timeout);
        let input = self.buffers.input_append_buf();
        let amount = self.stream.read(input)?;
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        self.stream.get_mut().get_mut().is_open()
    }

    fn is_tls(&self) -> bool {
        true
    }
}

impl fmt::Debug for TlsTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsTransport").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CERT: &[u8] = include_bytes!("testing/cert.der");

    fn provider() -> Arc<CryptoProvider> {
        Arc::new(rustls_rustcrypto::provider())
    }

    fn verify(verifier: &TofuVerifier) -> Result<ServerCertVerified, rustls::Error> {
        let name = ServerName::try_from("localhost").unwrap();
        verifier.verify_server_cert(&CertificateDer::from(CERT), &[], &name, &[], UnixTime::now())
    }

    #[test]
    fn os_verifier_only_for_unpinned_https() {
        assert!(TofuVerifier::new(&provider(), &TlsState::new(None), false).inner.is_none());
        assert!(TofuVerifier::new(&provider(), &TlsState::new(Some(Fingerprint([0; 32]))), true).inner.is_none());
    }

    #[test]
    fn no_ca_store_asks_the_user() {
        // Plain HTTP builds no OS verifier, the same state as a machine without CA certificates.
        let state = TlsState::new(None);
        let verifier = TofuVerifier::new(&provider(), &state, false);
        assert!(matches!(verify(&verifier), Err(rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer))));
        assert!(matches!(state.rejected(), Some(Error::UnknownCertificate { fingerprint }) if fingerprint == Fingerprint::of(CERT)));
    }

    #[test]
    fn each_handshake_clears_the_seen_leaf() {
        let state = TlsState::new(Some(Fingerprint::of(CERT)));
        *state.seen.lock().unwrap() = Some(Fingerprint([1; 32]));
        assert!(verify(&TofuVerifier::new(&provider(), &state, true)).is_ok());
        assert_eq!(*state.seen.lock().unwrap(), None);
    }
}
