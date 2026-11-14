//! Tests for the store, the migration and the fallback when nobody is signed in.
//!
//! A keychain is awkward to drive from a test. Everything above it therefore runs against
//! [`Memory`](crate::store::Memory).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::accounts::{self, Account};
use crate::paths::{self, Probe, Rule};
use crate::store::{Memory, TokenStore};
use crate::{Adopted, Error, NewAccount, Secret, Shared, Store};

const SERVICE: &str = "https://craft.example.com";
const USER: &str = "e9b1c0e4-0000-4000-8000-000000000001";
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const NOW: i64 = 1_760_000_000;

fn probe(env: &[(&str, &str)], exe_dir: Option<&str>, config: Option<&str>) -> Probe {
    let env: Vec<(String, String)> = env.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    Probe {
        env: Box::new(move |name| env.iter().find(|(k, _)| k == name).map(|(_, v)| OsString::from(v))),
        exe_dir: exe_dir.map(PathBuf::from),
        platform_config_dir: config.map(PathBuf::from),
    }
}

fn new_account() -> NewAccount {
    NewAccount { service: SERVICE.to_owned(), user: USER.to_owned(), display: "Ada Lovelace".to_owned(), email: None, linked: true }
}

// ---- where the store lives -------------------------------------------------

#[test]
fn the_variable_names_the_folder() {
    let r = paths::resolve(&probe(&[(paths::ENV_VAR, "/somewhere/else")], None, Some("/home/a/.config")));
    assert_eq!(r.rule, Rule::EnvVar);
    assert_eq!(r.dir.as_deref(), Some(Path::new("/somewhere/else")));
}

#[test]
fn an_empty_variable_keeps_nothing() {
    let r = paths::resolve(&probe(&[(paths::ENV_VAR, "")], None, Some("/home/a/.config")));
    assert_eq!(r.rule, Rule::Isolated);
    assert_eq!(r.dir, None);
}

#[test]
fn the_platform_folder_is_the_default() {
    let r = paths::resolve(&probe(&[], None, Some("/home/a/.config")));
    assert_eq!(r.rule, Rule::Platform);
    assert_eq!(r.dir.as_deref(), Some(Path::new("/home/a/.config").join(paths::DIR).as_path()));
}

#[test]
fn a_portable_marker_wins_over_the_platform_folder() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("portable.txt"), b"").unwrap();
    let exe_dir = dir.path().display().to_string();
    let r = paths::resolve(&probe(&[], Some(&exe_dir), Some("/home/a/.config")));
    assert_eq!(r.rule, Rule::Portable);
    assert_eq!(r.dir.as_deref(), Some(dir.path().join(paths::PORTABLE_DIR).as_path()));
}

#[test]
fn no_platform_folder_keeps_nothing() {
    let r = paths::resolve(&probe(&[], None, None));
    assert_eq!(r.rule, Rule::Nowhere);
    assert_eq!(r.dir, None);
}

#[test]
fn the_handle_is_stable_and_separates_accounts() {
    let a = accounts::handle(SERVICE, USER);
    assert_eq!(a, accounts::handle(SERVICE, USER), "the same account twice is the same entry");
    // A trailing slash is the same deployment typed differently.
    assert_eq!(a, accounts::handle("https://craft.example.com/", USER));
    assert_ne!(a, accounts::handle("https://other.example.com", USER), "another deployment is another account");
    assert_ne!(a, accounts::handle(SERVICE, "somebody-else"), "another person is another account");
    assert_eq!(a.len(), 24);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
}

// ---- signing in and out ----------------------------------------------------

#[test]
fn a_sign_in_can_be_read_back_by_the_next_application() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    let account = store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap();

    // A second application, its own Store over the same folder and keychain.
    let other = Store::at(Some(dir.path().to_path_buf()), keychain);
    match other.shared() {
        Shared::Session { account: found, token } => {
            assert_eq!(found.id, account.id);
            assert_eq!(found.service, SERVICE);
            assert_eq!(found.display, "Ada Lovelace");
            assert_eq!(token, Secret::new(TOKEN));
        }
        other => panic!("expected a session, got {other:?}"),
    }
}

#[test]
fn the_accounts_file_never_holds_the_token() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().to_path_buf()), Memory::new());
    store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap();
    let on_disk = std::fs::read_to_string(store.path().unwrap()).unwrap();
    assert!(!on_disk.contains(TOKEN), "the token reached the accounts file");
    assert!(on_disk.contains(SERVICE), "the file should still name the deployment");
}

#[test]
fn signing_in_again_replaces_the_entry_rather_than_adding_one() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    let first = store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap();
    let second = store.sign_in(&new_account(), &Secret::new("a-newer-token"), NOW + 60).unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(first.added, second.added, "the first sign-in's date is kept");
    assert_eq!(keychain.count(), 1);
    assert_eq!(store.accounts().unwrap().accounts.len(), 1);
    match store.shared() {
        Shared::Session { token, .. } => assert_eq!(token, Secret::new("a-newer-token")),
        other => panic!("expected a session, got {other:?}"),
    }
}

#[test]
fn two_deployments_are_two_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap();
    let elsewhere = NewAccount { service: "https://other.example.com".to_owned(), ..new_account() };
    let second = store.sign_in(&elsewhere, &Secret::new("another-token"), NOW).unwrap();
    assert_eq!(keychain.count(), 2);
    let list = store.accounts().unwrap();
    assert_eq!(list.accounts.len(), 2);
    assert_eq!(list.active.as_deref(), Some(second.id.as_str()), "the newest sign-in becomes current");
}

#[test]
fn the_linked_flag_is_persisted_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().to_path_buf()), Memory::new());
    let linked = NewAccount { linked: true, ..new_account() };
    let account = store.sign_in(&linked, &Secret::new(TOKEN), NOW).unwrap();
    assert!(account.linked, "sign_in's own return value should carry it");
    assert!(store.accounts().unwrap().get(&account.id).unwrap().linked, "the row on file should carry it too");

    let unlinked = NewAccount { linked: false, ..new_account() };
    let account = store.sign_in(&unlinked, &Secret::new(TOKEN), NOW + 1).unwrap();
    assert!(!account.linked, "a later sign-in's value replaces the earlier one");
}

#[test]
fn discarding_an_account_clears_both_the_entry_and_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    let account = store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap();

    store.discard(&account.id).unwrap();
    assert!(!keychain.has(&account.id), "the keychain entry should be gone");
    assert!(store.accounts().unwrap().accounts.is_empty());
    assert!(matches!(store.shared(), Shared::None));
}

#[test]
fn forgetting_an_account_clears_only_the_entry_and_leaves_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    let account = store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap();

    store.forget(&account.id).unwrap();
    assert!(!keychain.has(&account.id), "the keychain entry should be gone");
    assert_eq!(store.accounts().unwrap().accounts.len(), 1, "forgetting leaves the row; discard removes it too");
    assert!(matches!(store.shared(), Shared::None), "no readable token behind the row still reads as nobody signed in");
}

#[test]
fn an_isolated_store_refuses_to_forget_a_credential_it_does_not_own() {
    let keychain = Memory::new();
    keychain.put("somebody-elses-handle", &Secret::new(TOKEN)).unwrap();
    let store: Store<Memory> = Store::at(None, keychain.clone());

    assert_eq!(store.forget("somebody-elses-handle").unwrap_err(), Error::Isolated);
    assert!(keychain.has("somebody-elses-handle"), "an isolated store must never reach another application's entry");
}

#[test]
fn an_isolated_store_refuses_to_read_has_token_too() {
    let keychain = Memory::new();
    keychain.put("somebody-elses-handle", &Secret::new(TOKEN)).unwrap();
    let store: Store<Memory> = Store::at(None, keychain);

    assert_eq!(store.has_token("somebody-elses-handle").unwrap_err(), Error::Isolated);
}

#[test]
fn discarding_an_unknown_account_is_a_quiet_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().to_path_buf()), Memory::new());
    store.discard("nobody").unwrap();
}

// ---- the generation guards a stale write -----------------------------------

#[test]
fn a_write_against_a_stale_generation_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    let first = store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap();
    assert_eq!(first.generation, 0);

    // Somebody else signs the same account in again, elsewhere, before the refresh below lands.
    let second = store.sign_in(&new_account(), &Secret::new("rotated-elsewhere"), NOW + 30).unwrap();
    assert_eq!(second.id, first.id);
    assert_eq!(second.generation, 1);

    // A refresh that read the account before that second sign-in still carries generation 0.
    let stale = store.update_token(&first.id, first.generation, &Secret::new("late-refresh"), NOW + 60);
    assert_eq!(stale.unwrap_err(), Error::StaleGeneration { id: first.id.clone(), expected: 0, current: 1 });

    // The write never landed: the token on file is still the one from the second sign-in.
    match store.shared() {
        Shared::Session { token, .. } => assert_eq!(token, Secret::new("rotated-elsewhere")),
        other => panic!("expected a session, got {other:?}"),
    }
}

#[test]
fn a_write_against_the_current_generation_succeeds_and_advances_it() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    let account = store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap();

    let updated = store.update_token(&account.id, account.generation, &Secret::new("refreshed-token"), NOW + 30).unwrap();
    assert_eq!(updated.generation, account.generation + 1);
    match store.shared() {
        Shared::Session { token, .. } => assert_eq!(token, Secret::new("refreshed-token")),
        other => panic!("expected a session, got {other:?}"),
    }
}

#[test]
fn refreshing_an_account_does_not_steal_activation_from_another() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    let a = store.sign_in(&new_account(), &Secret::new("a-token"), NOW).unwrap();
    let elsewhere = NewAccount { service: "https://other.example.com".to_owned(), ..new_account() };
    let b = store.sign_in(&elsewhere, &Secret::new("b-token"), NOW + 1).unwrap();
    assert_eq!(store.accounts().unwrap().active.as_deref(), Some(b.id.as_str()), "B is the one picked by signing in last");

    store.update_token(&a.id, a.generation, &Secret::new("a-token-refreshed"), NOW + 2).unwrap();

    assert_eq!(store.accounts().unwrap().active.as_deref(), Some(b.id.as_str()), "refreshing A in the background must not steal B's selection");
    match store.shared() {
        Shared::Session { account, token } => {
            assert_eq!(account.id, b.id);
            assert_eq!(token, Secret::new("b-token"));
        }
        other => panic!("expected B's session, got {other:?}"),
    }
}

#[test]
fn a_write_against_an_unknown_account_says_so_rather_than_claiming_a_stale_generation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().to_path_buf()), Memory::new());
    assert_eq!(store.update_token("nobody", 0, &Secret::new(TOKEN), NOW).unwrap_err(), Error::Unknown("nobody".to_owned()));
}

// ---- a lost race never touches the keychain --------------------------------

/// A newer sign-in for the same handle wins the accounts lock first. Its own token is already in
/// the keychain. It finishes its commit while a stale refresh is still blocked waiting on that
/// same lock. The refresh's second check must then see the new generation and refuse, having
/// never touched the keychain at all. The race is set up by holding the real lock on a
/// background thread, releasing it only once the refresh (on the calling thread) has had the
/// chance to take its own unlocked first read of the old, pre-race state.
#[test]
fn a_refresh_racing_a_newer_sign_in_for_the_same_handle_never_overwrites_its_token() {
    use std::sync::mpsc;

    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    let first = store.sign_in(&new_account(), &Secret::new("session-1"), NOW).unwrap();
    let seen_generation = first.generation;

    let (ready_tx, ready_rx) = mpsc::sync_channel::<()>(0);
    let (release_tx, release_rx) = mpsc::sync_channel::<()>(0);
    let hold_dir = dir.path().to_path_buf();
    let hold_keychain = keychain.clone();
    let hold_id = first.id.clone();
    let hold_handle = std::thread::spawn(move || -> Result<(), Error> {
        let path = paths::file(&hold_dir);
        crate::lock::with_lock(&path, || {
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            // Stands in for a second sign-in's own unlocked token write, already landed, now
            // committing its row while the refresh below is still waiting on this same lock.
            hold_keychain.put(&hold_id, &Secret::new("session-2")).map_err(Error::Keychain)?;
            let mut list = accounts::read(&path)?;
            let generation = list.get(&hold_id).map_or(0, |a| a.generation) + 1;
            list.put(Account { id: hold_id.clone(), generation, ..Account::default() });
            accounts::write(&path, &list)
        })
    });

    ready_rx.recv().unwrap();
    release_tx.send(()).unwrap();
    let refresh = store.update_token(&first.id, seen_generation, &Secret::new("session-1-refreshed"), NOW + 20);
    hold_handle.join().unwrap().unwrap();

    assert!(matches!(refresh, Err(Error::StaleGeneration { .. })), "the refresh must lose to the newer sign-in: {refresh:?}");
    assert!(keychain.has(&first.id), "the new session's token must still be in the keychain");
    assert_eq!(
        keychain.get(&first.id).unwrap(),
        Some(Secret::new("session-2")),
        "the stale refresh must never have overwritten the new session's token"
    );
    match store.shared() {
        Shared::Session { token, .. } => assert_eq!(token, Secret::new("session-2")),
        other => panic!("expected session 2's session, got {other:?}"),
    }
}

/// The same race, with the newer sign-in preceded by a discard: the handle is removed and
/// tombstoned, then signed in again, both landing while the refresh is blocked on the lock. Both
/// steps complete before the refresh ever gets the lock back. Its second check lands on the
/// re-added row's new, active generation, never the tombstone in between.
#[test]
fn a_refresh_racing_a_discard_and_re_add_for_the_same_handle_never_overwrites_its_token() {
    use std::sync::mpsc;

    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    let first = store.sign_in(&new_account(), &Secret::new("session-1"), NOW).unwrap();
    let seen_generation = first.generation;

    let (ready_tx, ready_rx) = mpsc::sync_channel::<()>(0);
    let (release_tx, release_rx) = mpsc::sync_channel::<()>(0);
    let hold_dir = dir.path().to_path_buf();
    let hold_keychain = keychain.clone();
    let hold_id = first.id.clone();
    let hold_handle = std::thread::spawn(move || -> Result<(), Error> {
        let path = paths::file(&hold_dir);
        crate::lock::with_lock(&path, || {
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            // Discard, then a fresh sign-in for the same handle: both land together while the
            // refresh below is still waiting on this same lock.
            let _ = hold_keychain.remove(&hold_id);
            let mut list = accounts::read(&path)?;
            let old_generation = list.get(&hold_id).map(|a| a.generation).unwrap_or(0);
            list.remove_with_tombstone(&hold_id, old_generation)?;
            hold_keychain.put(&hold_id, &Secret::new("session-2")).map_err(Error::Keychain)?;
            let generation = list.tombstone_generation(&hold_id).unwrap_or(0) + 1;
            list.put(Account { id: hold_id.clone(), generation, ..Account::default() });
            accounts::write(&path, &list)
        })
    });

    ready_rx.recv().unwrap();
    release_tx.send(()).unwrap();
    let refresh = store.update_token(&first.id, seen_generation, &Secret::new("session-1-refreshed"), NOW + 20);
    hold_handle.join().unwrap().unwrap();

    assert!(refresh.is_err(), "the refresh must lose to the discard and re-add: {refresh:?}");
    assert!(keychain.has(&first.id), "the re-added session's token must still be in the keychain");
    assert_eq!(
        keychain.get(&first.id).unwrap(),
        Some(Secret::new("session-2")),
        "the stale refresh must never have overwritten the re-added session's token"
    );
    match store.shared() {
        Shared::Session { token, .. } => assert_eq!(token, Secret::new("session-2")),
        other => panic!("expected session 2's session, got {other:?}"),
    }
}

#[test]
fn sign_in_restores_its_token_when_a_concurrent_discard_takes_it_first() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let plain = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    let first = plain.sign_in(&new_account(), &Secret::new("session-1"), NOW).unwrap();

    /// Fires once, on this sign-in's own first token write, before it asks for the accounts
    /// lock: runs a full discard for the same handle, taking the entry this call just wrote with
    /// it and leaving a tombstone behind it. By the time the sign-in reaches its own lock, the
    /// row it is about to commit has no keychain entry behind it unless something restores one.
    ///
    /// Armed for a single call only. The fix's own re-assertion put runs inside the lock and must
    /// land plainly. Firing discard again from inside it would try to take the same accounts
    /// lock this call already holds. That is the lock module's own deadlock case. This test
    /// means to cover a different race, the one between two separate lock holders.
    struct DiscardAfterPut {
        inner: Memory,
        dir: PathBuf,
        id: String,
        fired: std::sync::atomic::AtomicBool,
    }
    impl TokenStore for DiscardAfterPut {
        fn available(&self) -> Result<(), String> {
            self.inner.available()
        }
        fn get(&self, h: &str) -> Result<Option<Secret>, String> {
            self.inner.get(h)
        }
        fn put(&self, h: &str, t: &Secret) -> Result<(), String> {
            let r = self.inner.put(h, t);
            if !self.fired.swap(true, std::sync::atomic::Ordering::SeqCst) {
                Store::at(Some(self.dir.clone()), self.inner.clone()).discard(&self.id).unwrap();
            }
            r
        }
        fn remove(&self, h: &str) -> Result<(), String> {
            self.inner.remove(h)
        }
    }

    let racing = Store::at(
        Some(dir.path().to_path_buf()),
        DiscardAfterPut {
            inner: keychain.clone(),
            dir: dir.path().to_path_buf(),
            id: first.id.clone(),
            fired: std::sync::atomic::AtomicBool::new(false),
        },
    );
    let account = racing.sign_in(&new_account(), &Secret::new("session-2"), NOW + 10).unwrap();

    assert_eq!(account.id, first.id, "the same deployment and subject is the same handle");
    assert!(
        keychain.has(&account.id),
        "a discard landed between this sign-in's own token write and its row commit; the row this call reported Ok for must not be left without a token"
    );
    match Store::at(Some(dir.path().to_path_buf()), keychain.clone()).shared() {
        Shared::Session { token, .. } => assert_eq!(token, Secret::new("session-2")),
        other => panic!("expected a session, got {other:?}"),
    }
}

// ---- generations never wrap ------------------------------------------------

/// What a crafted accounts file at the limit looks like: one account, holding `generation`.
fn account_at(id: &str, generation: u32) -> Account {
    Account {
        id: id.to_owned(),
        service: SERVICE.to_owned(),
        user: USER.to_owned(),
        display: "Ada Lovelace".to_owned(),
        generation,
        ..Account::default()
    }
}

#[test]
fn remove_with_tombstone_refuses_to_wrap_past_the_maximum_generation() {
    let mut list = accounts::Accounts::default();
    let err = list.remove_with_tombstone("x", u32::MAX).unwrap_err();
    assert_eq!(err, Error::GenerationExhausted("x".to_owned()));
    assert!(list.tombstones.is_empty(), "a refused bump must leave no tombstone behind");
}

#[test]
fn a_refresh_against_an_account_already_at_the_maximum_generation_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(paths::FILE);
    let id = accounts::handle(SERVICE, USER);
    let list =
        accounts::Accounts { format: accounts::FORMAT, accounts: vec![account_at(&id, u32::MAX)], active: Some(id.clone()), tombstones: Vec::new() };
    accounts::write(&path, &list).unwrap();
    let keychain = Memory::new();
    keychain.put(&id, &Secret::new(TOKEN)).unwrap();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain);

    let err = store.update_token(&id, u32::MAX, &Secret::new("refreshed"), NOW + 1).unwrap_err();
    assert_eq!(err, Error::GenerationExhausted(id));
}

#[test]
fn signing_in_over_a_tombstone_at_the_maximum_generation_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(paths::FILE);
    let id = accounts::handle(SERVICE, USER);
    let list = accounts::Accounts {
        format: accounts::FORMAT,
        accounts: Vec::new(),
        active: None,
        tombstones: vec![accounts::Tombstone { id: id.clone(), generation: u32::MAX }],
    };
    accounts::write(&path, &list).unwrap();
    let store = Store::at(Some(dir.path().to_path_buf()), Memory::new());

    let err = store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap_err();
    assert_eq!(err, Error::GenerationExhausted(id));
}

// ---- the list never grows past what the next reader can open ---------------

#[test]
fn write_refuses_a_list_too_large_for_read_to_ever_open_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(paths::FILE);
    let oversized = accounts::Accounts {
        format: accounts::FORMAT,
        accounts: vec![Account { display: "x".repeat(2 << 20), ..account_at("huge", 0) }],
        active: None,
        tombstones: Vec::new(),
    };

    assert!(matches!(accounts::write(&path, &oversized), Err(Error::Malformed(_))));
    assert!(!path.exists(), "a write that cannot be read back must never land on disk");
}

#[test]
fn write_leaves_a_previously_good_file_untouched_when_the_new_one_is_too_large() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(paths::FILE);
    let good =
        accounts::Accounts { format: accounts::FORMAT, accounts: vec![account_at("a", 0)], active: Some("a".to_owned()), tombstones: Vec::new() };
    accounts::write(&path, &good).unwrap();

    let oversized = accounts::Accounts {
        format: accounts::FORMAT,
        accounts: vec![Account { display: "x".repeat(2 << 20), ..account_at("huge", 0) }],
        active: None,
        tombstones: Vec::new(),
    };
    assert!(accounts::write(&path, &oversized).is_err());

    assert_eq!(accounts::read(&path).unwrap(), good, "the earlier, readable file must still be exactly what was there before");
}

#[test]
fn a_sign_in_with_an_oversized_display_name_fails_without_bricking_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap();

    let huge = NewAccount { display: "x".repeat(2 << 20), ..NewAccount { service: "https://other.example.com".to_owned(), ..new_account() } };
    assert!(store.sign_in(&huge, &Secret::new("another-token"), NOW + 1).is_err());

    // The list a moment ago is still exactly readable. The first account is still there.
    // `shared()` still answers normally. Without the guard, every later call would fail on a
    // file nothing can parse back.
    let list = store.accounts().unwrap();
    assert_eq!(list.accounts.len(), 1);
    assert!(matches!(store.shared(), Shared::Session { .. }));
}

// ---- nobody is signed in ---------------------------------------------------

#[test]
fn an_empty_suite_offers_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().to_path_buf()), Memory::new());
    assert!(matches!(store.shared(), Shared::None));
    assert!(store.accounts().unwrap().accounts.is_empty());
}

#[test]
fn an_isolated_store_offers_nothing_and_signs_nobody_in() {
    let store: Store<Memory> = Store::at(None, Memory::new());
    assert!(matches!(store.shared(), Shared::None));
    assert_eq!(store.usable().unwrap_err(), Error::Isolated);
    assert_eq!(store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap_err(), Error::Isolated);
}

#[test]
fn a_machine_with_no_keychain_signs_nobody_in() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().to_path_buf()), Memory::missing("no Secret Service on this machine"));
    assert!(matches!(store.shared(), Shared::None));
    match store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap_err() {
        Error::Keychain(why) => assert!(why.contains("no Secret Service")),
        other => panic!("expected a keychain failure, got {other:?}"),
    }
    assert!(!store.path().unwrap().exists(), "a refused sign-in should write no file");
}

#[test]
fn an_unreadable_keychain_says_so_rather_than_pretending_nobody_signed_in() {
    let dir = tempfile::tempdir().unwrap();
    // Sign in against a working keychain, then read the same folder through a locked one.
    let store = Store::at(Some(dir.path().to_path_buf()), Memory::new());
    let account = store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap();

    let locked = Store::at(Some(dir.path().to_path_buf()), Memory::refusing("the keychain is locked"));
    match locked.shared() {
        Shared::Unreadable { account: found, why } => {
            assert_eq!(found.id, account.id);
            assert!(why.contains("locked"));
        }
        other => panic!("expected an unreadable session, got {other:?}"),
    }
}

#[test]
fn an_account_whose_entry_is_gone_reads_as_nobody_signed_in() {
    let dir = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(dir.path().to_path_buf()), keychain.clone());
    let account = store.sign_in(&new_account(), &Secret::new(TOKEN), NOW).unwrap();
    // Somebody deleted the credential from the platform's own window.
    keychain.remove(&account.id).unwrap();
    assert!(matches!(store.shared(), Shared::None));
}

#[test]
fn an_active_account_that_is_no_longer_listed_reads_as_nobody_signed_in() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(paths::FILE);
    let list = accounts::Accounts { format: accounts::FORMAT, accounts: Vec::new(), active: Some("vanished".to_owned()), tombstones: Vec::new() };
    accounts::write(&path, &list).unwrap();
    let store = Store::at(Some(dir.path().to_path_buf()), Memory::new());
    assert!(matches!(store.shared(), Shared::None));
}

#[test]
fn a_file_from_a_later_format_is_refused_rather_than_misread() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(paths::FILE);
    std::fs::write(&path, br#"{"format":99,"accounts":[],"active":null}"#).unwrap();
    let store = Store::at(Some(dir.path().to_path_buf()), Memory::new());
    assert!(matches!(store.accounts(), Err(Error::Malformed(_))));
    // An application still starts: the shared answer is that nobody is signed in.
    assert!(matches!(store.shared(), Shared::None));
}

#[test]
fn unreadable_json_does_not_stop_an_application_starting() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(paths::FILE), b"this is not JSON").unwrap();
    let store = Store::at(Some(dir.path().to_path_buf()), Memory::new());
    assert!(matches!(store.shared(), Shared::None));
}

#[test]
fn a_format_of_zero_is_refused_rather_than_silently_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(paths::FILE);
    std::fs::write(&path, br#"{"format":0,"accounts":[],"active":null,"tombstones":[]}"#).unwrap();
    assert!(matches!(accounts::read(&path), Err(Error::Malformed(_))));
}

#[test]
fn a_missing_format_marker_is_refused_rather_than_silently_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(paths::FILE);
    // No `format` key at all: `#[serde(default)]` would otherwise read this exactly as it reads
    // an explicit `"format":0` above. Both would have passed the old `> FORMAT` check.
    std::fs::write(&path, br#"{"accounts":[],"active":null,"tombstones":[]}"#).unwrap();
    assert!(matches!(accounts::read(&path), Err(Error::Malformed(_))));
}

#[test]
fn the_one_supported_format_still_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(paths::FILE);
    let body = format!(r#"{{"format":{},"accounts":[],"active":null,"tombstones":[]}}"#, accounts::FORMAT);
    std::fs::write(&path, body.as_bytes()).unwrap();
    assert!(accounts::read(&path).is_ok());
}

#[test]
fn a_read_error_does_not_name_the_profile_folder_it_lives_under() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("Users").join("a-real-persons-name").join(paths::DIR);
    std::fs::create_dir_all(&home).unwrap();
    let path = home.join(paths::FILE);
    std::fs::write(&path, b"this is not JSON").unwrap();

    let message = accounts::read(&path).unwrap_err().to_string();
    assert!(!message.contains("a-real-persons-name"), "the profile folder leaked into the error: {message}");
    assert!(message.contains(paths::FILE), "the error should still say which file: {message}");
}

#[test]
fn an_oversized_file_error_does_not_name_the_profile_folder_either() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("Users").join("another-real-name").join(paths::DIR);
    std::fs::create_dir_all(&home).unwrap();
    let path = home.join(paths::FILE);
    std::fs::write(&path, vec![b'x'; 2 << 20]).unwrap();

    let message = accounts::read(&path).unwrap_err().to_string();
    assert!(!message.contains("another-real-name"), "the profile folder leaked into the error: {message}");
}

// ---- taking over a per-application session file ----------------------------

/// What GridCraft's `legacy-session.json` is read as.
fn legacy_file(dir: &Path) -> PathBuf {
    let path = dir.join("legacy-session.json");
    let body = format!(r#"{{"service":"{SERVICE}","token":"{TOKEN}","user":"{USER}","display":"Ada Lovelace"}}"#);
    std::fs::write(&path, body.as_bytes()).unwrap();
    path
}

#[test]
fn a_per_application_session_moves_into_the_suite_store_and_the_file_goes() {
    let suite = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(suite.path().to_path_buf()), keychain.clone());
    let path = legacy_file(app.path());

    match crate::migrate::adopt(&store, &path, NOW).unwrap() {
        Adopted::Taken(account) => {
            assert_eq!(account.service, SERVICE);
            assert_eq!(account.user, USER);
            assert!(keychain.has(&account.id), "the token should be in the keychain");
        }
        other => panic!("expected the session to be taken over, got {other:?}"),
    }
    assert!(!path.exists(), "the session file should be gone");
    match store.shared() {
        Shared::Session { token, .. } => assert_eq!(token, Secret::new(TOKEN)),
        other => panic!("expected a session, got {other:?}"),
    }
}

#[test]
fn migration_repairs_an_orphan_keychain_token_with_the_legacy_file() {
    const LEGACY: &str = "legacy-session-token";
    let suite = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(suite.path().to_path_buf()), keychain.clone());
    let id = accounts::handle(SERVICE, USER);
    keychain.put(&id, &Secret::new("orphan-token")).unwrap();
    let path = legacy_file(app.path());
    std::fs::write(&path, format!(r#"{{"service":"{SERVICE}","token":"{LEGACY}","user":"{USER}","display":"Ada Lovelace"}}"#).as_bytes()).unwrap();

    match crate::migrate::adopt(&store, &path, NOW).unwrap() {
        Adopted::Taken(account) => assert_eq!(account.id, id),
        other => panic!("expected the legacy session to be taken over, got {other:?}"),
    }
    assert!(!path.exists());
    match store.shared() {
        Shared::Session { token, .. } => assert_eq!(token, Secret::new(LEGACY)),
        other => panic!("expected a session, got {other:?}"),
    }
}

#[test]
fn migration_leaves_a_legacy_file_alone_once_it_changed_after_being_read() {
    let suite = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let path = legacy_file(app.path());

    /// Stands in for another process still running the application's own, now-superseded
    /// writer: it rewrites the legacy file with a different token the moment the suite store
    /// writes this account's token, the exact window between `adopt`'s read of the file and its
    /// removal of it.
    struct RewriteFileOnPut {
        inner: Memory,
        path: PathBuf,
    }
    impl TokenStore for RewriteFileOnPut {
        fn available(&self) -> Result<(), String> {
            self.inner.available()
        }
        fn get(&self, h: &str) -> Result<Option<Secret>, String> {
            self.inner.get(h)
        }
        fn put(&self, h: &str, t: &Secret) -> Result<(), String> {
            let body = format!(r#"{{"service":"{SERVICE}","token":"refreshed-elsewhere","user":"{USER}","display":"Ada Lovelace"}}"#);
            std::fs::write(&self.path, body.as_bytes()).unwrap();
            self.inner.put(h, t)
        }
        fn remove(&self, h: &str) -> Result<(), String> {
            self.inner.remove(h)
        }
    }

    let store = Store::at(Some(suite.path().to_path_buf()), RewriteFileOnPut { inner: Memory::new(), path: path.clone() });
    let account = match crate::migrate::adopt(&store, &path, NOW).unwrap() {
        Adopted::Taken(account) => account,
        other => panic!("expected the session to be taken over, got {other:?}"),
    };
    assert_eq!(account.service, SERVICE, "the session that was actually read is still the one adopted");

    assert!(
        path.exists(),
        "the file changed after it was read; deleting it would discard the rewritten session with nothing left to recover it from"
    );
    let left_behind = std::fs::read_to_string(&path).unwrap();
    assert!(left_behind.contains("refreshed-elsewhere"), "the file on disk must still be the rewritten one: {left_behind}");
}

#[test]
fn a_second_migration_finds_nothing_to_do() {
    let suite = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let store = Store::at(Some(suite.path().to_path_buf()), Memory::new());
    let path = legacy_file(app.path());
    crate::migrate::adopt(&store, &path, NOW).unwrap();
    assert_eq!(crate::migrate::adopt(&store, &path, NOW).unwrap(), Adopted::Nothing);
}

#[test]
fn migration_with_no_keychain_leaves_the_file_alone() {
    let suite = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let store = Store::at(Some(suite.path().to_path_buf()), Memory::missing("no keychain here"));
    let path = legacy_file(app.path());

    match crate::migrate::adopt(&store, &path, NOW).unwrap() {
        Adopted::Kept { why } => assert!(why.contains("no keychain here")),
        other => panic!("expected the file to be kept, got {other:?}"),
    }
    assert!(path.exists(), "removing it would have signed the user out");
}

#[test]
fn migration_in_an_isolated_process_leaves_the_file_alone() {
    let app = tempfile::tempdir().unwrap();
    let store: Store<Memory> = Store::at(None, Memory::new());
    let path = legacy_file(app.path());
    assert!(matches!(crate::migrate::adopt(&store, &path, NOW).unwrap(), Adopted::Kept { .. }));
    assert!(path.exists());
}

#[test]
fn a_session_file_carrying_no_session_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-session.json");
    std::fs::write(&path, br#"{"service":"https://craft.example.com","token":"  "}"#).unwrap();
    assert!(matches!(crate::migrate::read(&path), Err(Error::Malformed(_))));
}

/// The four keys an application writes today, spelled exactly as it writes them.
#[test]
fn the_four_keys_an_application_writes_today_adopt_cleanly() {
    let suite = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(suite.path().to_path_buf()), keychain.clone());
    let path = app.path().join("legacy-session.json");
    let body = format!(r#"{{"token":"{TOKEN}","service":"{SERVICE}","subject":"{USER}","display":"Ada Lovelace"}}"#);
    std::fs::write(&path, body.as_bytes()).unwrap();

    match crate::migrate::adopt(&store, &path, NOW).unwrap() {
        Adopted::Taken(account) => {
            assert_eq!(account.service, SERVICE);
            assert_eq!(account.user, USER, "`subject` is the subject the handle is keyed on");
            assert_eq!(account.display, "Ada Lovelace");
            assert_eq!(account.id, accounts::handle(SERVICE, USER));
            assert!(keychain.has(&account.id));
        }
        other => panic!("expected the session to be taken over, got {other:?}"),
    }
    assert!(!path.exists());
}

/// What is on every machine that signed in before the application recorded anything else.
#[test]
fn a_token_only_session_file_is_left_alone_until_the_application_records_more() {
    let suite = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(suite.path().to_path_buf()), keychain.clone());
    let path = app.path().join("legacy-session.json");
    std::fs::write(&path, format!(r#"{{"token":"{TOKEN}"}}"#).as_bytes()).unwrap();

    // It parses, and it says nothing to key an account on.
    let legacy = crate::migrate::read(&path).unwrap().unwrap();
    assert_eq!(legacy.token, Secret::new(TOKEN));
    assert_eq!(legacy.service, "");
    assert_eq!(legacy.user, "", "the subject is never stood in for");
    assert_eq!(legacy.incomplete(), Some("which deployment it belongs to"));

    match crate::migrate::adopt(&store, &path, NOW).unwrap() {
        Adopted::Kept { why } => assert!(why.contains("does not say"), "{why}"),
        other => panic!("expected the file to be kept, got {other:?}"),
    }
    assert!(path.exists(), "removing it would have signed the user out");
    assert_eq!(keychain.count(), 0, "nothing should be filed under a made-up handle");
    assert!(store.accounts().unwrap().accounts.is_empty());
    assert!(matches!(store.shared(), Shared::None));

    // The application rewrites the file once it knows the rest. The next startup adopts it.
    let body = format!(r#"{{"token":"{TOKEN}","service":"{SERVICE}","subject":"{USER}","display":"Ada Lovelace"}}"#);
    std::fs::write(&path, body.as_bytes()).unwrap();
    assert!(matches!(crate::migrate::adopt(&store, &path, NOW).unwrap(), Adopted::Taken(_)));
    assert!(!path.exists());
    match store.shared() {
        Shared::Session { account, token } => {
            assert_eq!(account.id, accounts::handle(SERVICE, USER));
            assert_eq!(token, Secret::new(TOKEN));
        }
        other => panic!("expected a session, got {other:?}"),
    }
}

/// The deployment is recorded the moment the device flow completes. The subject only arrives when
/// `/api/v1/me` first answers, so a file can hold one without the other.
#[test]
fn a_session_file_with_no_subject_is_left_alone() {
    let suite = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let keychain = Memory::new();
    let store = Store::at(Some(suite.path().to_path_buf()), keychain.clone());
    let path = app.path().join("legacy-session.json");
    std::fs::write(&path, format!(r#"{{"token":"{TOKEN}","service":"{SERVICE}"}}"#).as_bytes()).unwrap();

    let legacy = crate::migrate::read(&path).unwrap().unwrap();
    assert_eq!(legacy.service, SERVICE);
    assert_eq!(legacy.incomplete(), Some("whose account it is"));

    match crate::migrate::adopt(&store, &path, NOW).unwrap() {
        Adopted::Kept { why } => assert!(why.contains("whose account it is"), "{why}"),
        other => panic!("expected the file to be kept, got {other:?}"),
    }
    assert!(path.exists());
    assert_eq!(keychain.count(), 0);
}

#[test]
fn a_session_file_spelled_the_other_way_still_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-session.json");
    let body = format!(r#"{{"base_url":"{SERVICE}/","access_token":"{TOKEN}","sub":"{USER}","name":"Ada"}}"#);
    std::fs::write(&path, body.as_bytes()).unwrap();
    let legacy = crate::migrate::read(&path).unwrap().unwrap();
    assert_eq!(legacy.service, SERVICE, "the trailing slash comes off");
    assert_eq!(legacy.user, USER);
    assert_eq!(legacy.display, "Ada");
    assert_eq!(legacy.token, Secret::new(TOKEN));
}

// ---- the secret itself -----------------------------------------------------

#[test]
fn a_secret_never_prints_itself() {
    let secret = Secret::new(TOKEN);
    let shown = format!("{secret:?}");
    assert!(!shown.contains(TOKEN), "Debug printed the token: {shown}");
    assert_eq!(shown, format!("Secret({} bytes)", TOKEN.len()));
    assert_eq!(secret.len(), TOKEN.len());
}

#[test]
fn an_account_record_prints_no_secret() {
    let account = Account { id: "abc".to_owned(), service: SERVICE.to_owned(), ..Account::default() };
    assert!(!format!("{account:?}").contains(TOKEN));
}
