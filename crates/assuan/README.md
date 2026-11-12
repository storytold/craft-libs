# craft-assuan

The Assuan protocol and gpg-agent's operations, without sockets.

OpenPGP private keys stay in GnuPG (on disk or on a smartcard) and are used through gpg-agent,
which asks for the PIN or passphrase in its own pinentry. This crate speaks to the agent over any
`Read + Write` stream the caller opened.

- **`Client`**: Assuan's line framing (lines of at most `MAX_LINE` bytes, percent-escaping), the
  server's `OK`, `ERR`, `S`, `D`, `INQUIRE` and `#` lines, and answers to an inquiry (`D` … `END`,
  or `CAN`).
- **`GpgAgent`**: whether a key is there (`HAVEKEY`, `KEYINFO`, `READKEY`), signing a digest
  (`SIGKEY`, `SETHASH`, `PKSIGN`; SHA-256, SHA-384 and SHA-512 only), decrypting a session key
  (`SETKEY`, `PKDECRYPT`), and the hints that let the agent's pinentry show on the owner's screen.
- **`sexp`**: a bounded reader and writer of canonical S-expressions, the form the agent hands
  keys, signatures and session keys over in.
- **`socket_file`**: the two kinds of file that may stand where the agent's socket is named: a
  redirect, and (on Windows) the loopback port and nonce of Assuan's socket emulation.

Finding the agent and opening the socket is the caller's. Everything the agent sends is treated
as hostile: lines, data, status lines, inquiries and S-expressions are capped (`MAX_LINE`,
`MAX_DATA`, `MAX_LINES`, `MAX_STATUS`, `MAX_INQUIRIES`, `MAX_INQUIRY_DATA`, `MAX_KEYS`), and
nothing here panics on junk. A session key that comes back is wiped from memory when it is
dropped and is never shown by `Debug`.

Written from the public documentation of the Assuan protocol and of gpg-agent's commands. Moved
from MailCraft's `mailcraft-assuan`.
