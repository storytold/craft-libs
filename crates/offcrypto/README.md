# craft-offcrypto

Clean-room Office document cryptography: password-encrypted OOXML packages and the password
hashes of document and sheet protection. No `unsafe`. No third-party implementation was
consulted; some expected keys in the tests were computed with msoffcrypto-tool and Python's
hashlib as independent checks.

| Document | Used for |
|---|---|
| [MS-OFFCRYPTO] Office Document Cryptography Structure (Microsoft Open Specifications) | §2.1 data spaces; §2.3.2, §2.3.3, §2.3.4.5-§2.3.4.9 standard encryption; §2.3.4.10-§2.3.4.15 agile encryption; §2.4.2.4 the ISO write-protection hash |
| ECMA-376 Part 1 §17.15.1.29, §18.2.29; Part 4 §14.8.1 | where protection hashes are stored; the legacy 32-bit password key |
| [MS-CFB] | the compound file, through `craft-cfb` |

## Encrypted packages

A password-protected `.docx`, `.xlsx` or `.pptx` is a compound file with an `EncryptionInfo`
stream and an `EncryptedPackage` stream. The crate turns that file into the zip package and
back. It does not look inside the package: `craft-opc` reads what `decrypt` returns.

```rust
if craft_offcrypto::is_encrypted(&bytes) {
    craft_offcrypto::check(&bytes)?;                       // Unsupported("RC4"), ... before asking
    let package = craft_offcrypto::decrypt(&bytes, Some(password))?;   // WrongPassword, Malformed
}
let file = craft_offcrypto::encrypt(&package, password)?;
```

| | Read | Written |
|---|---|---|
| Agile (Office 2010 and later): AES-128/192/256 CBC in 4096-byte segments, SHA-1/256/384/512 | yes; the HMAC over the encrypted stream is required and checked | AES-256, SHA-512, 100,000 spins, with the data-spaces streams Office expects |
| Standard (Office 2007): AES-128/192/256 ECB, SHA-1, 50,000 spins | yes | for tests only (`encrypt_standard`) |
| RC4, extensible, certificate-only, rights management | refused by name (`Error::Unsupported`) | no |

`decrypt(bytes, None)` returns `Error::PasswordRequired` once it knows the file is one it can
decrypt; `check` is that question on its own.

## Protection passwords

`protection::hash_password` makes the `algorithmName` / `hashValue` / `saltValue` / `spinCount`
values of a new protection password; `protection::check_password_hash` and
`check_legacy_password_hash` (the `cryptAlgorithmSid` form, through `legacy_key`) check a
password against stored values. Which attributes hold them is the format's business. A hash
that cannot be checked is `PasswordCheck::Unsupported`, never a match.

## Hostile input

Stream reads are capped (`EncryptionInfo` at 1 MiB, the package at the file's size), the XML
descriptor is parsed with `craft-xml` under the same cap, every length is checked before
slicing, a stated package size larger than the data is damage and never drives an allocation,
and spin counts above 10,000,000 are refused. Verifier and integrity values are compared
without stopping at the first difference. Passwords, keys and derived hashes are wiped when
dropped (`zeroize`).

## Origin

Moved from WordCraft's `crates/docx/src/crypt.rs` (at `07f97737`), the fullest of three copies:
it reads both schemes, writes agile encryption and has the protection hashes. From GridCraft's
`crates/xlsx/src/crypto.rs` (at `5cf52c5f`) come `check`, the naming of unsupported schemes
(`Error::Unsupported`), rights-managed files, choosing the password key encryptor by its URI,
and the independent key vectors in the tests. DeckCraft's `crates/pptx/src/cfb.rs` only
detected encrypted files, which `is_encrypted` does.
