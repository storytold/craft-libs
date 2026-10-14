# craft-nextcloud

Treat files on a Nextcloud server as documents an app can list, preview and open, without mounting the server.
Blocking, native-only. First used by LightCraft. Requires Rust 1.95. The design is in `docs/nextcloud.md`.

**This crate owns:** Login Flow v2 sign-in and trusting a self-signed certificate, the `nextcloud://` path format,
WebDAV listing, server previews, Range reads, downloads and new files (never replacing one), the local cache of originals, optional Memories metadata
and (feature `keyring`) app passwords in the OS keyring.

**The app owns:** where remote files appear, when a download is allowed, what it renders from a preview, its dialogs
and progress UI, and persisting `Account` with its other settings.

```rust,no_run
use std::sync::{Arc, atomic::AtomicBool};
use craft_nextcloud::{Client, Error, OriginalsCache, RemotePath, SignIn, SignInState, rustls::crypto::CryptoProvider};

// The app passes the crypto provider (for example rustls-rustcrypto, which is pure Rust).
fn run(provider: Arc<CryptoProvider>) -> Result<(), Error> {
    let mut sign_in = SignIn::start("cloud.example", "MyApp (Linux)", provider.clone());
    let (account, password) = loop {
        match sign_in.state() {
            SignInState::NeedsTrust { fingerprint } => {
                // show the fingerprint and ask the user, then:
                let _ = fingerprint;
                sign_in.confirm_trust(true);
            }
            SignInState::Waiting { login_url } => {
                // open login_url in the browser, then poll about every 2 s
                let _ = login_url;
                sign_in.poll();
            }
            SignInState::Done { account, app_password } => break (account.clone(), app_password.clone()),
            SignInState::Failed(_) => return Ok(()), // show the error
        }
    };

    let client = Client::new(account.clone(), password, provider)?;
    let dir = RemotePath::parse("nextcloud://me@cloud.example/Photos")?;
    for entry in client.list(&dir)? {
        println!("{} {}", entry.name, entry.size);
    }

    let cache = OriginalsCache::open(std::path::Path::new("/var/cache/app"), 20 << 30)?;
    let file = dir.join("IMG_0001.ARW")?;
    let _hold = cache.hold(&file);
    let fetched = cache.fetch(&client, &file, &AtomicBool::new(false), &mut |done, total| {
        let _ = (done, total);
    })?;
    println!("{}", fetched.path.display());
    Ok(())
}
```

## Features

- `keyring`: `store_password(app, account, password)`, `load_password` and `delete_password`. Native stores only
  (Apple Keychain, Windows Credential Manager, Secret Service), no OpenSSL. With no keyring they return an error that
  says so; there is no plaintext fallback.
- `testing`: `FakeServer`, an in-process scripted Nextcloud for integration tests, with failure injection.

## Limits

- Response caps: 64 MiB per listing, 32 MiB per preview, 64 MiB for a Range read, 256 MiB for the Memories timeline.
- Timeouts: 10 s to connect; downloads have a stall limit (30 s, `Client::with_stall_limit`) and no overall limit;
  sign-in ends after 20 minutes. `429` and `503` are retried at most 3 times, except in maintenance mode.

## Known limitations

- A login name that differs from the user id (email or LDAP logins) is not supported.
- No proxy support: an environment proxy is ignored.
- The crate never logs the app password, but `ureq` and `ureq_proto` log raw HTTP bytes at TRACE, including the
  `Authorization` header and the sign-in poll's `appPassword`: cap those two log targets below TRACE in the app's
  logger.
- No writing back (PUT, sidecars), sync or push notifications.
