# craft-openpgp

OpenPGP as mail carries it: Digital signatures checked and Public keys read. No I/O, no clock and
no keys of its own. The caller hands in the message's bytes, the Public keys it keeps, the time,
and whether the owner has verified a key.

- **`verify`** checks a PGP/MIME message (`multipart/signed;
  protocol="application/pgp-signature"`, RFC 3156 §5) over the signed part's canonical bytes, or a
  cleartext-signed text (RFC 9580 §7), and gives an `Outcome` (verified, valid but not verified,
  bad, or can't be checked) with what the signature covers (`Coverage`). MD5, SHA-1 and
  RIPEMD-160 signatures are refused as no longer safe.
- **`keys_in_message`** reads the keys a message carries: attached `application/pgp-keys` parts
  and the `Autocrypt:` header (Autocrypt Level 1). Nothing is fetched or sent.
- **`PublicKey`, `KeyRing`, `Fingerprint`, `KeyId`**: transferable public keys (armoured or
  binary) with their self-signatures, expiry and revocations checked.
- **`parse_autocrypt`**: one `Autocrypt:` header value.
- **`secret::read`, `secret::holds_secret`**: what a transferable secret key file is (its
  fingerprint, addresses, public half, whether every secret part has a passphrase, whether it can
  sign), without keeping or using the secret.

The packets and the cryptography are [rPGP](https://github.com/rpgp/rpgp)'s (`pgp`, MIT OR
Apache-2.0). The armour, the Cleartext Signature Framework and the little MIME needed to find the
signed bytes exactly as they came are read here, from RFC 2045/2046, RFC 3156 and RFC 9580, so
that every size is capped by this crate.

Everything from outside is hostile: messages past `MAX_MESSAGE` are not read, signatures, keys,
packets, user IDs, subkeys, MIME parts, nesting and the number of signature checks per call are
capped, work is linear in the input, and a panic inside rPGP is caught and reported as an
unreadable signature.

Moved from MailCraft's `mailcraft-openpgp`.
