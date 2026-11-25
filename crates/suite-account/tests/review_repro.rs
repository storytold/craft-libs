//! Conformance tests for `craft-suite-account` (PR #665), adapted from the maintainer's review
//! reproduction gist for this crate (skylartaylor, rev `2e9c84f`). Each test asserts the behaviour
//! the crate's docs and RFC-0025 promise.
//!
//! The first two tests below once drove their race through a hook on the [`Memory`] keychain's
//! `put`, timed to land between [`Store::update_token`]'s keychain write and its second, locked
//! check. That write now happens only inside the lock, in the second check's own success arm (see
//! the module docs). Nothing sits between the two checks to hook into any more. A hook that tried
//! would be asking `update_token` to take a lock it already holds. Real threads racing on a
//! [`std::sync::Barrier`] replace it, the same device [`concurrent_sign_ins_from_two_threads_lose_no_rows`]
//! already uses below. The lock makes both outcomes this PR promises hold for every interleaving,
//! never only the one picked in advance. A race no longer needs to land in one exact window to
//! prove it.
//!
//! Not adapted here: the gist's `review_h2_forget_during_refresh_is_undone`. `forget` is, by
//! RFC-0025's "Cross-app lifecycle guarantees" (point 4), a deliberately generation-agnostic,
//! keychain-only, local operation. It does not read or write the accounts list at all, so nothing
//! in a locked read-change-write over that list can observe it racing a refresh. Fixing that
//! interleaving would mean giving `forget` list/generation semantics it does not have today, a
//! broader behaviour change than the three gates this PR closes (see the PR comment).

use std::sync::{Arc, Barrier};

use craft_suite_account::{Error, Memory, NewAccount, Secret, Store, TokenStore};

const SERVICE: &str = "https://craft.example.com";
const NOW: i64 = 1_760_000_000;

fn acct(user: &str) -> NewAccount {
    NewAccount { service: SERVICE.to_owned(), user: user.to_owned(), display: user.to_owned(), email: None, linked: true }
}

fn tok(s: &str) -> Secret {
    Secret::new(s.to_owned())
}

// ---- gate: locked read-change-write on every list mutation -----------------

/// H1b: app A refreshes X while app B signs in Y, on real threads racing to start at the same
/// moment. A's write must not land a stale copy of the list that was read before B's sign-in, or
/// Y's row is lost even though Y's token sits in the keychain. `update_token`'s own lock always
/// re-reads the list fresh right before it commits. That holds for either thread landing first.
#[test]
fn refresh_racing_another_sign_in_does_not_lose_that_row() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let app_a = Store::at(Some(dir.path().to_owned()), keychain.clone());
    let app_b = Store::at(Some(dir.path().to_owned()), keychain.clone());

    let x = app_a.sign_in(&acct("x"), &tok("x1"), NOW).unwrap();

    let start = Arc::new(Barrier::new(2));
    let (a_start, b_start) = (start.clone(), start.clone());
    let a_id = x.id.clone();
    let a = std::thread::spawn(move || {
        a_start.wait();
        app_a.update_token(&a_id, x.generation, &tok("x2"), NOW + 1)
    });
    let b = std::thread::spawn(move || {
        b_start.wait();
        app_b.sign_in(&acct("y"), &tok("y1"), NOW)
    });
    a.join().unwrap().unwrap();
    let y = b.join().unwrap().unwrap();

    let list = Store::at(Some(dir.path().to_owned()), Memory::new()).accounts().unwrap();
    let ids: Vec<_> = list.accounts.iter().map(|a| a.user.clone()).collect();
    assert!(
        list.get(&y.id).is_some(),
        "Y signed in successfully but its row is gone after X's refresh: accounts on file = {ids:?}, \
         Y token in keychain = {}",
        keychain.has(&y.id)
    );
}

/// H1c: the same lost update with no hook at all: real threads, separate `Store`s over one folder
/// (as two applications in one process would be), each signing in its own account. Every sign-in
/// that returned Ok must be on the list afterwards, and none should fail.
#[test]
fn concurrent_sign_ins_from_two_threads_lose_no_rows() {
    let rounds = 200;
    let (mut lost, mut failed) = (0, 0);
    let mut examples = Vec::new();
    for round in 0..rounds {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Memory::new();
        let start = Arc::new(Barrier::new(2));
        let handles: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|who| {
                let (path, kc, start) = (dir.path().to_owned(), keychain.clone(), start.clone());
                std::thread::spawn(move || {
                    let store = Store::at(Some(path), kc);
                    start.wait();
                    store.sign_in(&acct(&format!("{who}-{round}")), &tok(who), NOW)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let list = Store::at(Some(dir.path().to_owned()), Memory::new()).accounts().unwrap();
        for r in &results {
            match r {
                Ok(a) if list.get(&a.id).is_none() => {
                    lost += 1;
                    if examples.len() < 3 {
                        examples.push(format!("round {round}: sign_in Ok but row missing ({} of 2 rows on file)", list.accounts.len()));
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    failed += 1;
                    if examples.len() < 3 {
                        examples.push(format!("round {round}: sign_in Err({e})"));
                    }
                }
            }
        }
    }
    assert!(
        lost == 0 && failed == 0,
        "{rounds} rounds x 2 concurrent sign-ins: {lost} Ok sign-ins lost their row, {failed} sign-ins failed; e.g. {examples:#?}"
    );
}

/// Helper for the two-process case below: when REVIEW_DIR is set, sign in REVIEW_N accounts named
/// REVIEW_WHO-i into that folder. Does nothing in a normal run.
#[test]
fn concurrent_sign_in_helper() {
    let (Ok(dir), Ok(who)) = (std::env::var("REVIEW_DIR"), std::env::var("REVIEW_WHO")) else { return };
    let n: usize = std::env::var("REVIEW_N").ok().and_then(|n| n.parse().ok()).unwrap_or(100);
    let store = Store::at(Some(dir.into()), Memory::new());
    for i in 0..n {
        store.sign_in(&acct(&format!("{who}-{i}")), &tok(&who), NOW).unwrap();
    }
}

/// The gate's named case: two OS processes (this test binary re-run as the helper above), each
/// signing in 100 distinct accounts into one suite folder. All 200 succeed (Ok), so 200 rows must
/// be on file: real processes rather than simulated interleaving, since this crate's own test
/// binary can spawn itself and CI here can run child processes.
#[test]
fn concurrent_sign_ins_from_two_processes_lose_no_rows() {
    if std::env::var("REVIEW_DIR").is_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let children: Vec<_> = ["p", "q"]
        .into_iter()
        .map(|who| {
            std::process::Command::new(&exe)
                .args(["--exact", "concurrent_sign_in_helper", "--test-threads=1"])
                .env("REVIEW_DIR", dir.path())
                .env("REVIEW_WHO", who)
                .env("REVIEW_N", "100")
                .spawn()
                .unwrap()
        })
        .collect();
    for mut c in children {
        assert!(c.wait().unwrap().success(), "a helper process failed a sign_in");
    }
    let list = Store::at(Some(dir.path().to_owned()), Memory::new()).accounts().unwrap();
    assert_eq!(
        list.accounts.len(),
        200,
        "two processes each completed 100 successful sign_in calls, but only {} rows are on file",
        list.accounts.len()
    );
}

// ---- gate: generations monotonic across removal and re-add -----------------

/// The gate's named case "refresh-racing-discard": app A refreshes X's token while app B signs X
/// out (`discard`), on real threads racing to start at the same moment. Removal must win (RFC-0025
/// lifecycle guarantee 2) whichever one actually lands first: neither the row nor the keychain
/// entry may come back, whether the refresh commits before the discard removes it, or the discard
/// lands first and the refresh's own second check then refuses a tombstoned account.
#[test]
fn refresh_racing_discard_resurrects_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let app_a = Store::at(Some(dir.path().to_owned()), keychain.clone());
    let app_b = Store::at(Some(dir.path().to_owned()), keychain.clone());

    let x = app_a.sign_in(&acct("x"), &tok("x-session-1"), NOW).unwrap();

    let start = Arc::new(Barrier::new(2));
    let (a_start, b_start) = (start.clone(), start.clone());
    let a_id = x.id.clone();
    let a = std::thread::spawn(move || {
        a_start.wait();
        app_a.update_token(&a_id, x.generation, &tok("x-session-1-refreshed"), NOW + 1)
    });
    let b_id = x.id.clone();
    let b = std::thread::spawn(move || {
        b_start.wait();
        app_b.discard(&b_id)
    });
    let refreshed = a.join().unwrap();
    b.join().unwrap().unwrap();

    let after = Store::at(Some(dir.path().to_owned()), Memory::new()).accounts().unwrap();
    let row_back = after.get(&x.id).is_some();
    let token_back = keychain.has(&x.id);
    assert!(
        !row_back && !token_back,
        "sign-out (discard) must win over an in-flight refresh; instead update_token returned {:?}, \
         row present after discard = {row_back}, keychain entry present after discard = {token_back}",
        refreshed.as_ref().map(|a| a.generation)
    );
}

/// The gate's named case "stale-refresh-after-discard-and-re-add": a refresh that started under
/// session 1 lands after the account was discarded and signed in again (session 2). Generations
/// monotonic across removal and re-add means the re-added row never restarts at 0, so the stale
/// refresh is refused and session 2's token survives.
#[test]
fn stale_refresh_after_discard_and_re_add_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_owned()), keychain.clone());

    let s1 = store.sign_in(&acct("x"), &tok("session-1"), NOW).unwrap();
    let seen_generation = s1.generation; // a refresher reads this, then goes to the network
    store.discard(&s1.id).unwrap(); // the person signs out ...
    let s2 = store.sign_in(&acct("x"), &tok("session-2"), NOW + 10).unwrap(); // ... and back in

    let stale = store.update_token(&s1.id, seen_generation, &tok("session-1-refreshed"), NOW + 20);
    let now_token = keychain.get(&s1.id).unwrap().map(|t| t.expose().to_owned());
    assert_eq!(
        stale,
        Err(Error::StaleGeneration { id: s1.id.clone(), expected: seen_generation, current: s2.generation }),
        "refresh from the discarded session (generation {seen_generation}) must be refused as stale, \
         never Unknown: re-added session has generation {}; keychain now holds {:?}",
        s2.generation,
        now_token
    );
    assert_eq!(now_token, Some("session-2".to_owned()), "session 2's token must survive the stale refresh");
}

/// The gate's named case "re-add-continues-generation": removing an account and signing it back in
/// must never restart its generation at 0. One bump for the tombstone the removal leaves, one for
/// the sign-in that follows it.
#[test]
fn re_add_continues_generation_past_the_tombstone() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().to_owned()), Memory::new());

    let s1 = store.sign_in(&acct("x"), &tok("session-1"), NOW).unwrap();
    assert_eq!(s1.generation, 0);
    store.discard(&s1.id).unwrap();
    let s2 = store.sign_in(&acct("x"), &tok("session-2"), NOW + 10).unwrap();

    assert_eq!(s2.id, s1.id, "the same deployment and subject is the same handle");
    assert!(
        s2.generation > s1.generation,
        "the re-added session's generation must be ahead of the discarded one, never restarting at 0: {} vs {}",
        s2.generation,
        s1.generation
    );
    assert_eq!(s2.generation, s1.generation + 2, "one bump for the tombstone, one for the sign-in");

    // A third sign-in should continue on from there rather than from the tombstone again.
    let s3 = store.sign_in(&acct("x"), &tok("session-3"), NOW + 20).unwrap();
    assert_eq!(s3.generation, s2.generation + 1);
}

// ---- gate: defined partial-failure recovery ---------------------------------

/// RFC-0025 guarantee 1: a partial failure leaves a state the next reader detects and repairs,
/// never a silent mismatch. Simulates the window `update_token` opens between writing the
/// refreshed token to the keychain and committing the matching generation bump to the list (see
/// the crate's module docs, "Concurrent mutations and partial-failure recovery"): the token store
/// receives the refreshed token directly, standing in for a crash right after that write and
/// before the list commit. The list is then one generation behind a keychain token that is already
/// the newer one. The next `update_token` against that same, still-current generation must still
/// succeed and bring both back into step, rather than getting stuck re-reporting the mismatch.
#[test]
fn torn_state_between_the_keychain_write_and_the_generation_commit_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_owned()), keychain.clone());

    let account = store.sign_in(&acct("x"), &tok("session-1"), NOW).unwrap();

    // Stand in for the crash: the keychain already holds the refreshed token, but the list was
    // never told, so it still shows the old generation.
    keychain.put(&account.id, &tok("session-1-refreshed")).unwrap();
    match store.shared() {
        craft_suite_account::Shared::Session { token, .. } => {
            assert_eq!(token, tok("session-1-refreshed"), "the torn state is detectable: the keychain is already ahead of the list");
        }
        other => panic!("expected a session, got {other:?}"),
    }

    // The next legitimate refresh, made against the generation the list still shows, must still
    // go through and bring both back into step.
    let recovered = store.update_token(&account.id, account.generation, &tok("session-2"), NOW + 30).unwrap();
    assert_eq!(recovered.generation, account.generation + 1);
    match store.shared() {
        craft_suite_account::Shared::Session { token, .. } => assert_eq!(token, tok("session-2")),
        other => panic!("expected a session, got {other:?}"),
    }
}
