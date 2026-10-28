use std::sync::Arc;
use std::time::{Duration, Instant};

use craft_nextcloud::testing::{FakeServer, Fault};
use craft_nextcloud::{Account, Client, Error, Fingerprint, SignIn, SignInState};

const NAME: &str = "LightCraft (Linux)";

fn provider() -> Arc<craft_nextcloud::rustls::crypto::CryptoProvider> {
    Arc::new(rustls_rustcrypto::provider())
}

#[test]
fn plain_flow_reaches_done() {
    let server = FakeServer::start();
    let mut sign_in = SignIn::start(&server.url(), NAME, provider());
    match sign_in.state() {
        SignInState::Waiting { login_url } => assert!(login_url.contains("/login/v2/flow/")),
        other => panic!("{other:?}"),
    }
    assert!(matches!(sign_in.poll(), SignInState::Waiting { .. }));
    server.approve_login();
    let SignInState::Done { account, app_password } = sign_in.poll() else { panic!("not done") };
    let (account, app_password) = (account.clone(), app_password.clone());
    assert_eq!(account.user, "alice");
    assert_eq!(app_password, FakeServer::PASSWORD);
    assert_eq!(account.trust, None);
    let client = Client::new(account.clone(), app_password, provider()).unwrap();
    client.stat(&account.root()).unwrap();
}

#[test]
fn tls_flow_asks_for_trust() {
    let server = FakeServer::start_tls(provider());
    let mut sign_in = SignIn::start(&server.url(), NAME, provider());
    match sign_in.state() {
        SignInState::NeedsTrust { fingerprint } => assert_eq!(Some(*fingerprint), server.fingerprint()),
        other => panic!("{other:?}"),
    }
    assert!(matches!(sign_in.confirm_trust(true), SignInState::Waiting { .. }));
    server.approve_login();
    match sign_in.poll() {
        SignInState::Done { account, .. } => assert_eq!(account.trust, server.fingerprint()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn sign_in_again_keeps_the_pin() {
    let server = FakeServer::start_tls(provider());
    let account = Account { trust: server.fingerprint(), ..server.account() };
    let mut sign_in = SignIn::start_for(&account, NAME, provider());
    assert!(matches!(sign_in.state(), SignInState::Waiting { .. }), "{:?}", sign_in.state());
    server.approve_login();
    match sign_in.poll() {
        SignInState::Done { account: done, .. } => assert_eq!(done, &account),
        other => panic!("{other:?}"),
    }
    // A different certificate is a hard error, not a trust prompt.
    let pinned_elsewhere = Account { trust: Some(Fingerprint([0; 32])), ..account };
    let sign_in = SignIn::start_for(&pinned_elsewhere, NAME, provider());
    assert!(matches!(sign_in.state(), SignInState::Failed(Error::CertificateChanged)), "{:?}", sign_in.state());
}

#[test]
fn declined_trust_fails() {
    let server = FakeServer::start_tls(provider());
    let mut sign_in = SignIn::start(&server.url(), NAME, provider());
    assert!(matches!(sign_in.confirm_trust(false), SignInState::Failed(Error::Cancelled)));
}

#[test]
fn cancel_fails() {
    let server = FakeServer::start();
    let mut sign_in = SignIn::start(&server.url(), NAME, provider());
    sign_in.cancel();
    assert!(matches!(sign_in.state(), SignInState::Failed(Error::Cancelled)));
    server.approve_login();
    assert!(matches!(sign_in.poll(), SignInState::Failed(Error::Cancelled)));
}

#[test]
fn times_out_after_twenty_minutes() {
    let server = FakeServer::start();
    let mut sign_in = SignIn::start(&server.url(), NAME, provider());
    sign_in.set_deadline_for_test(Instant::now() - Duration::from_secs(1));
    assert!(matches!(sign_in.poll(), SignInState::Failed(Error::Cancelled)));
}

#[test]
fn foreign_poll_endpoint_rejected() {
    // The second isn't a valid server address, which is the server's fault, not the user's.
    for endpoint in ["https://evil.example/poll", "http://evil.example/poll"] {
        let server = FakeServer::start();
        server.set_poll_endpoint(endpoint);
        let sign_in = SignIn::start(&server.url(), NAME, provider());
        assert!(matches!(sign_in.state(), SignInState::Failed(Error::Malformed(_))), "{endpoint}: {:?}", sign_in.state());
    }
}

#[test]
fn bad_server_url_fails() {
    let sign_in = SignIn::start("ftp://x", NAME, provider());
    assert!(matches!(sign_in.state(), SignInState::Failed(Error::InvalidAddress(_))));
}

#[test]
fn done_debug_redacts_password() {
    let server = FakeServer::start();
    let mut sign_in = SignIn::start(&server.url(), NAME, provider());
    server.approve_login();
    sign_in.poll();
    assert!(matches!(sign_in.state(), SignInState::Done { .. }));
    assert!(!format!("{:?}", sign_in.state()).contains(FakeServer::PASSWORD));
    assert!(!format!("{sign_in:?}").contains(FakeServer::PASSWORD));
}

#[test]
fn transient_poll_errors_keep_waiting() {
    let server = FakeServer::start();
    let mut sign_in = SignIn::start(&server.url(), NAME, provider());
    server.fail_next(Fault::Drop);
    assert!(matches!(sign_in.poll(), SignInState::Waiting { .. }));
    server.approve_login();
    assert!(matches!(sign_in.poll(), SignInState::Done { .. }));
}

#[test]
fn requests_carry_the_client_name() {
    let server = FakeServer::start();
    let mut sign_in = SignIn::start(&server.url(), NAME, provider());
    assert_eq!(server.last_user_agent().as_deref(), Some(NAME));
    server.approve_login();
    assert!(matches!(sign_in.poll(), SignInState::Done { .. }));
    assert_eq!(server.last_user_agent().as_deref(), Some(NAME));
    // A name that can't be a header value falls back rather than failing the sign-in.
    for bad in ["", "Light\nCraft"] {
        assert!(matches!(SignIn::start(&server.url(), bad, provider()).state(), SignInState::Waiting { .. }), "{bad:?}");
        assert_eq!(server.last_user_agent().as_deref(), Some("Craft"), "{bad:?}");
    }
}
