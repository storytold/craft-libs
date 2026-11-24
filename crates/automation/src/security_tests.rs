#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Behaviour tests of `security`.

use super::*;
use craft_ui::testing::trial;

const TOKEN: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn names() -> (crate::args::FlagNames, crate::args::EnvNames) {
    (crate::args::FlagNames::default(), crate::args::EnvNames::for_app("PHOTOCRAFT"))
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("craft-automation-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One frame through a fresh handshake: the reply and whether it authenticated.
fn one_frame(line: &str, token: &str) -> (Value, bool) {
    let (reply, step) = ServerHandshake::new().reply(line, token);
    (reply, step == AuthStep::Authenticated)
}

fn auth_line(token: &str) -> String {
    format!(r#"{{"id": 7, "method": "auth", "params": {{"token": "{token}"}}}}"#)
}

fn challenge_line(client_nonce: &str) -> String {
    serde_json::json!({"id": 1, "method": CHALLENGE_METHOD, "params": {"clientNonce": client_nonce}}).to_string()
}

fn proof_line(proof: &str) -> String {
    serde_json::json!({"id": 2, "method": AUTH_METHOD, "params": {"proof": proof}}).to_string()
}

// The one-frame `auth {token}` still authenticates; anything else as a first line is refused.
#[trial]
fn the_one_frame_token_auth_still_authenticates() {
    let refused = |id: Value| serde_json::json!({"id": id, "ok": false, "error": "authentication required"});

    let (reply, ok) = one_frame(&auth_line(TOKEN), TOKEN);
    assert!(ok);
    assert_eq!(reply, serde_json::json!({"id": 7, "ok": true, "result": {"authenticated": true}}));

    // The server's copy is lower-cased; a client sends the token as configured.
    assert!(one_frame(&auth_line(&TOKEN.to_uppercase()), TOKEN).1);

    let wrong = TOKEN.replace('0', "1");
    assert_eq!(one_frame(&auth_line(&wrong), TOKEN), (refused(7.into()), false));
    assert_eq!(one_frame(&auth_line("short"), TOKEN), (refused(7.into()), false));
    let other_method = format!(r#"{{"id": 8, "method": "ui.click", "params": {{"token": "{TOKEN}"}}}}"#);
    assert_eq!(one_frame(&other_method, TOKEN), (refused(8.into()), false));
    assert_eq!(one_frame("not json", TOKEN), (refused(Value::Null), false));

    let already = |id: i64| serde_json::json!({"id": id, "ok": false, "error": "already authenticated"});
    assert_eq!(repeat_auth_reply(&auth_line(TOKEN)).unwrap(), already(7));
    assert_eq!(repeat_auth_reply(&challenge_line(TOKEN)).unwrap(), already(1));
    assert_eq!(repeat_auth_reply(r#"{"id": 1, "method": "ui.click"}"#), None);
    assert_eq!(repeat_auth_reply("not json"), None);
}

// A client proves it holds the token, and learns that the server does, without the token crossing
// the wire.
#[trial]
fn challenge_authentication_proves_the_token_both_ways_without_sending_it() {
    let token = generate_token().unwrap();
    let client_nonce = generate_nonce().unwrap();
    let mut server = ServerHandshake::new();
    let (reply, step) = server.reply(&challenge_line(&client_nonce), &token);
    assert_eq!(step, AuthStep::Challenged, "{reply}");
    assert!(!reply.to_string().contains(&token), "the challenge reply must not carry the token");
    let server_nonce = reply["result"]["serverNonce"].as_str().unwrap();
    let server_proof = reply["result"]["serverProof"].as_str().unwrap();
    assert!(is_hex256(server_nonce) && server_nonce != client_nonce);
    assert!(proof_matches(&token, ProofRole::Server, &client_nonce, server_nonce, server_proof));
    // A server holding another token cannot produce that proof.
    let other = generate_token().unwrap();
    assert!(!proof_matches(&other, ProofRole::Server, &client_nonce, server_nonce, server_proof));
    // The server's proof is not the client's: replaying it back does not authenticate.
    assert!(!proof_matches(&token, ProofRole::Client, &client_nonce, server_nonce, server_proof));

    let proof = handshake_proof(&token, ProofRole::Client, &client_nonce, server_nonce).unwrap();
    assert_ne!(proof, token);
    let (reply, step) = server.reply(&proof_line(&proof), &token);
    assert_eq!(step, AuthStep::Authenticated, "{reply}");
    assert_eq!(reply["result"]["authenticated"], true);
}

// After a challenge, a wrong, missing or token-shaped proof, the token itself, a second challenge
// and a control method are all refused, and a malformed nonce gets no challenge.
#[trial]
fn challenge_authentication_refuses_a_wrong_or_missing_proof() {
    let token = generate_token().unwrap();
    let client_nonce = generate_nonce().unwrap();
    let wrong = handshake_proof(&generate_token().unwrap(), ProofRole::Client, &client_nonce, &client_nonce).unwrap();
    let bad_proofs = [proof_line(&wrong), proof_line(&token), serde_json::json!({"id": 2, "method": AUTH_METHOD, "params": {}}).to_string()];
    for line in bad_proofs {
        let mut server = ServerHandshake::new();
        assert_eq!(server.reply(&challenge_line(&client_nonce), &token).1, AuthStep::Challenged);
        let (reply, step) = server.reply(&line, &token);
        assert_eq!(step, AuthStep::Rejected, "{line}: {reply}");
        assert_eq!(reply["error"], "authentication required");
    }
    for line in [auth_line(&token), challenge_line(&client_nonce), r#"{"id":3,"method":"methods"}"#.to_owned()] {
        let mut server = ServerHandshake::new();
        assert_eq!(server.reply(&challenge_line(&client_nonce), &token).1, AuthStep::Challenged);
        assert_eq!(server.reply(&line, &token).1, AuthStep::Rejected, "{line}");
    }
    for nonce in ["", "short", &"g".repeat(64), &"a".repeat(65)] {
        let (reply, step) = ServerHandshake::new().reply(&challenge_line(nonce), &token);
        assert_eq!(step, AuthStep::Rejected, "{nonce}: {reply}");
        assert!(reply.get("result").is_none());
    }
}

// Tokens are 64 lower-case hex; a file token is trimmed; both supplied is a conflict.
#[trial]
fn server_token_sources_and_validation() {
    let generated = generate_token().unwrap();
    validate_token(&generated).unwrap();
    assert_ne!(generated, generate_token().unwrap());
    assert!(generated.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));

    let bad = validate_token(&TOKEN[1..]).unwrap_err();
    assert_eq!(bad.to_string(), "control token must contain exactly 64 hexadecimal characters");
    assert!(validate_token(&format!("{}g", &TOKEN[1..])).is_err());

    assert_eq!(server_token(Some(&TOKEN.to_uppercase()), None).unwrap(), TOKEN);
    let dir = scratch_dir("token");
    let file = dir.join("nested").join("token");
    let both = server_token(Some(TOKEN), Some(&file)).unwrap_err();
    assert_eq!(both.to_string(), "use either a control token or a control token file, not both");

    // A missing file is created with the generated token, readable by its owner only.
    let created = server_token(None, Some(&file)).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("{created}\n"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }
    #[cfg(windows)]
    {
        use craft_platform::owner_only;
        let sddl = owner_only::sddl_of(&file).unwrap();
        assert_eq!(owner_only::foreign_access(&sddl, &owner_only::current_user_sid().unwrap()), None, "{sddl}");
    }
    assert_eq!(server_token(None, Some(&file)).unwrap(), created);

    // An existing file's content is trimmed and lower-cased.
    std::fs::write(&file, format!("  {}\n\n", TOKEN.to_uppercase())).unwrap();
    assert_eq!(server_token(None, Some(&file)).unwrap(), TOKEN);
    let (flags, env) = names();
    assert_eq!(client_token(None, Some(&file), &flags, &env).unwrap(), TOKEN);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        let err = server_token(None, Some(&file)).unwrap_err().to_string();
        assert!(err.starts_with("I/O: ") && err.contains("readable by group or others (mode 640)"), "{err}");
    }
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        client_token(None, None, &flags, &env).unwrap_err().to_string(),
        "bridge mode needs --control-token, --control-token-file, PHOTOCRAFT_CONTROL_TOKEN, or PHOTOCRAFT_CONTROL_TOKEN_FILE"
    );
}

// A token file create that runs out of time fails with its own error, even when it left a file at
// the path: that file is not read as another process's. A create that another process beat to the
// path reads that process's token.
#[trial]
fn a_token_file_create_that_runs_out_of_time_is_an_error() {
    let dir = scratch_dir("token-limit");
    let file = dir.join("token");
    let late = |p: &Path, _: &str| {
        std::fs::write(p, "partial").unwrap();
        Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("{}: powershell.exe did not finish within 10 s and was ended", p.display())))
    };
    let err = server_token_with(None, Some(&file), late).unwrap_err().to_string();
    assert!(err.starts_with("I/O: ") && err.ends_with("did not finish within 10 s and was ended"), "{err}");
    std::fs::remove_file(&file).unwrap();
    let beaten = |p: &Path, _: &str| {
        create_token_file(p, TOKEN).unwrap();
        Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "the file already exists"))
    };
    assert_eq!(server_token_with(None, Some(&file), beaten).unwrap(), TOKEN);
    let _ = std::fs::remove_dir_all(&dir);
}
