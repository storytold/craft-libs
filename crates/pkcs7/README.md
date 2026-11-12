# craft-pkcs7

The formats of digital signatures, read, verified and written: what a signed PDF, a signed mail
message or a digital ID file is made of.

- **`der`**: a small BER/DER reader and writer (ITU-T X.690). A `Tlv` borrows from the input and
  keeps its exact encoding, so a signed structure is verified over the bytes that were signed.
  Reading is strict DER; `parse_ber` also takes indefinite lengths and segmented strings, which
  PKCS #12 files from Windows use. Nesting is bounded.
- **`x509`**: certificates (RFC 5280): names, validity, key, the extensions that matter for
  signing (basic constraints, key usage, extended key usage, key identifiers, OCSP and CRL
  locations), `build_chain` / `build_chain_noted` (issuers that may issue, path length, validity,
  unprocessed critical extensions), `load_certificates` (DER, PEM, PKCS #7 bundles), `to_pem`,
  and `Certificate::self_signed`. An extension that is present but unreadable restricts: an
  unreadable key usage allows no usage, an unreadable extended key usage no purpose.
- **`cms`**: CMS SignedData (RFC 5652): `SignedData::parse` and `verify_signature`,
  `sign_detached` / `sign_encapsulated`, `certificate_bundle`, and `attach_timestamp_token`.
- **`pkcs12`**: `.p12` / `.pfx` (RFC 7292): `open` reads PBES2 (PBKDF2 with HMAC-SHA-1/2;
  AES-CBC or 3DES-CBC) and the legacy PBE schemes (SHA-1 with 3DES or RC2) and checks the MAC,
  stopping at a fixed amount of key-derivation work; `write` uses PBES2 AES-256-CBC with
  PBKDF2-HMAC-SHA-256.
- **`timestamp`**: RFC 3161 requests, responses and tokens, and a responder for a local
  stamping identity. No transport: `TimestampAuthority` is the caller's.
- **`revocation`**: CRLs (RFC 5280) and OCSP responses (RFC 6960): parsed, their signatures
  verified, and a certificate's status looked up. Nothing is fetched.
- **`keys`**: digests (SHA-1, SHA-2, SHA-3, RIPEMD-160), public keys for verification (RSA
  PKCS #1 v1.5 and PSS, ECDSA on P-256, P-384, P-521 and the Brainpool curves, Ed25519) and
  private keys for signing (RSA, P-256, P-384, or a key held elsewhere behind `ExternalKey`).

What a signature means for a document or a message stays with the caller: which bytes it covers,
what changed after it, which certificates are trusted, and how a result is worded.

## Cryptography

The primitives are RustCrypto's; RSA private-key operations use aws-lc-rs on native targets.
Two things are done in this crate, both verification only and on public data, because no audited
crate offers them: the RSA signature paddings for any digest, MGF1 hash and salt length
(`rsa_pad`, over the `rsa` crate's raw public-key operation), and ECDSA on brainpoolP512r1
(`ec512`, over `crypto-bigint`). Both are checked against signatures made by OpenSSL.

Weak algorithms are read, not refused, because old signed documents use them:
`DigestAlg::weakness` and `PublicKey::weakness` say when one is weak, and the caller decides.

Moved from PdfCraft's `pdfcraft-sign`.
