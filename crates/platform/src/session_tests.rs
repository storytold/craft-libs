//! Reopen last session.

use crate::session::*;

fn doc(path: &str) -> SessionDocument {
    SessionDocument { path: path.to_string(), ..Default::default() }
}

#[test]
// After quit_started, closing documents doesn't change to_save(true, ..); quit_cancelled makes to_save return the open session again.
fn quit_keeps_the_session_from_its_start() {
    let two = Session { documents: vec![doc("/a"), doc("/b")], active: Some("/a".into()) };
    let one = Session { documents: vec![doc("/b")], active: None };
    let mut keeper = SessionKeeper::default();
    keeper.quit_started(|| two.clone());
    keeper.quit_started(|| one.clone());
    assert_eq!(keeper.to_save(true, one.clone()), two);
    assert_eq!(keeper.to_save(false, one.clone()), one);
    keeper.quit_cancelled();
    assert_eq!(keeper.to_save(true, one.clone()), one);
}
