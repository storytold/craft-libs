use craft_history::{ArchivedDocument, History, HistoryState, Policy, StateIdentity};
use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Debug)]
struct Sheet {
    id: u64,
    cells: Arc<Vec<u8>>,
    selection: bool,
}
#[derive(Clone, Debug)]
struct Sheets;
impl Policy for Sheets {
    type Document = Sheet;
    type Target = Vec<usize>;
    type Id = u64;
    fn has_selection(doc: &Sheet) -> bool {
        doc.selection
    }
    fn remap_id(doc: &mut Arc<Sheet>, id: u64) {
        if doc.id != id {
            Arc::make_mut(doc).id = id;
        }
    }
    fn document_bytes(doc: &Sheet, seen: &mut HashSet<usize>) -> usize {
        if seen.insert(Arc::as_ptr(&doc.cells) as usize) { doc.cells.capacity() } else { 0 }
    }
    fn target_bytes(target: &Vec<usize>) -> usize {
        target.capacity() * size_of::<usize>()
    }
}
fn sheet(value: u8) -> Arc<Sheet> {
    Arc::new(Sheet { id: 1, cells: Arc::new(vec![value; 32]), selection: value.is_multiple_of(2) })
}
fn edit(history: &mut History<Sheets>, current: &mut Arc<Sheet>, value: u8) {
    history.record(format!("Cell {value}"), current.clone(), vec![usize::from(value)]);
    *current = sheet(value);
}
#[derive(Debug)]
struct Archive {
    document: Arc<Sheet>,
    reads: Arc<AtomicUsize>,
    fail: bool,
}
impl ArchivedDocument<Sheets> for Archive {
    fn load(&self) -> Result<Arc<Sheet>, String> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if self.fail { Err("Storage unavailable".into()) } else { Ok(self.document.clone()) }
    }
}
#[test]
fn branches_survive_navigation_and_checkpoint_restore() {
    let mut history = History::<Sheets>::default();
    let mut current = sheet(0);
    edit(&mut history, &mut current, 1);
    edit(&mut history, &mut current, 2);
    let alternate = history.current_id();
    current = history.try_undo(current).unwrap().unwrap();
    edit(&mut history, &mut current, 3);
    assert_eq!(history.timeline().len(), 4);
    let checkpoint = history.checkpoint();
    let mut restored = History::<Sheets>::from_checkpoint(checkpoint).unwrap();
    let target = restored.state_by_id(current.clone(), alternate).unwrap().load_document().unwrap();
    assert_eq!(target.cells[0], 2);
    restored.jump_loaded(current, alternate, target).unwrap();
    assert_eq!(restored.current_id(), alternate);
    assert_eq!(restored.current_target(), &vec![2]);
    assert_eq!(restored.timeline().len(), 4);
}
#[test]
fn cold_navigation_reads_only_destination_and_menus_read_nothing() {
    let mut history = History::<Sheets>::default();
    let mut current = sheet(0);
    for value in 1..80 {
        edit(&mut history, &mut current, value);
    }
    let reads = Arc::new(AtomicUsize::new(0));
    for state in history.checkpoint().undo {
        let document = state.load_document().unwrap();
        history.replace_resident(&document, Arc::new(Archive { document: document.clone(), reads: reads.clone(), fail: false }));
    }
    let _ = history.timeline();
    let _ = history.checkpoint();
    assert!(history.has_past_selection());
    assert!(history.latest_selection_state().is_some());
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    let target = history.state_by_id(current.clone(), 0).unwrap().load_document().unwrap();
    history.jump_loaded(current, 0, target).unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert_eq!(history.archived_states(), 78);
}
#[test]
fn failed_undo_and_redo_leave_navigation_unchanged() {
    for redo in [false, true] {
        let mut history = History::<Sheets>::default();
        let mut current = sheet(0);
        edit(&mut history, &mut current, 1);
        if redo {
            current = history.try_undo(current).unwrap().unwrap();
        }
        let target = history.state_by_id(current.clone(), history.adjacent_id(redo).unwrap()).unwrap().load_document().unwrap();
        history.replace_resident(&target, Arc::new(Archive { document: target.clone(), reads: Arc::new(AtomicUsize::new(0)), fail: true }));
        let before = history.timeline_stamp();
        let checkpoint = history.checkpoint();
        let result = if redo { history.try_redo(current) } else { history.try_undo(current) };
        assert!(result.is_err());
        assert_eq!(history.timeline_stamp(), before);
        assert_eq!(history.checkpoint().current_label, checkpoint.current_label);
    }
}
#[test]
fn shared_allocations_are_counted_once_and_targets_count_as_metadata() {
    let current = sheet(0);
    let mut history = History::<Sheets>::default();
    history.record("First", current.clone(), vec![3, 4]);
    history.record("Second", Arc::new((*current).clone()), vec![5]);
    assert_eq!(history.pixel_bytes(&current), 32);
    assert_eq!(history.unique_bytes(&current), 0);
    assert!(history.metadata_bytes() > 0);
    assert_eq!(history.spillable_states().len(), 2);
}
#[test]
fn cold_identity_remap_is_lazy_and_preserves_original() {
    let mut history = History::<Sheets>::default();
    let mut current = sheet(0);
    edit(&mut history, &mut current, 1);
    let original = history.try_state(0).unwrap().unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    history.replace_resident(&original, Arc::new(Archive { document: original.clone(), reads: reads.clone(), fail: false }));
    history.remap_document_id(99);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(history.try_state(0).unwrap().unwrap().id, 99);
    assert_eq!(original.id, 1);
}
#[test]
fn unlimited_default_and_explicit_limits_preserve_current() {
    let mut history = History::<Sheets>::default();
    let mut current = sheet(0);
    for value in 1..120 {
        edit(&mut history, &mut current, value);
    }
    assert_eq!(history.past_len(), 119);
    let id = history.current_id();
    history.max_states = 2;
    assert_eq!(history.enforce_state_limit(), 117);
    assert_eq!(history.past_len(), 2);
    assert_eq!(history.current_id(), id);
    history.max_bytes = 1;
    assert_eq!(history.trim(&current), 1);
    assert!(history.can_undo());
}
#[test]
fn recovery_rejects_duplicate_and_invalid_identity_and_oversized_labels() {
    let mut history = History::<Sheets>::default();
    let mut current = sheet(0);
    edit(&mut history, &mut current, 1);
    let good = history.checkpoint();
    let mut bad = good.clone();
    bad.undo[0].identity = bad.current_identity;
    assert!(History::<Sheets>::from_checkpoint(bad).is_err());
    let mut bad = good.clone();
    bad.current_identity.parent = Some(bad.current_identity.id);
    assert!(History::<Sheets>::from_checkpoint(bad).is_err());
    let mut bad = good.clone();
    bad.current_identity.id = u64::MAX;
    assert!(History::<Sheets>::from_checkpoint(bad).is_err());
    let mut bad = good;
    bad.current_label = "X".repeat(4097);
    assert!(History::<Sheets>::from_checkpoint(bad).is_err());
}
#[test]
fn legacy_checkpoint_gets_a_valid_linear_identity_path() {
    let mut history = History::<Sheets>::default();
    let mut current = sheet(0);
    edit(&mut history, &mut current, 1);
    edit(&mut history, &mut current, 2);
    let mut checkpoint = history.checkpoint();
    checkpoint.current_identity = StateIdentity::default();
    for state in &mut checkpoint.undo {
        state.identity = StateIdentity::default();
    }
    let recovered = History::<Sheets>::from_checkpoint(checkpoint).unwrap();
    assert_eq!(recovered.timeline().iter().map(|entry| entry.identity.id).collect::<Vec<_>>(), vec![0, 1, 2]);
}
#[test]
fn stale_spill_result_does_not_replace_different_snapshot() {
    let mut history = History::<Sheets>::default();
    let mut current = sheet(0);
    edit(&mut history, &mut current, 1);
    let other = sheet(0);
    assert!(!history.replace_resident(&other, Arc::new(Archive { document: other.clone(), reads: Arc::new(AtomicUsize::new(0)), fail: true })));
    assert!(history.try_undo(current).unwrap().is_some());
}
#[derive(Clone, Debug)]
struct Slides;
impl Policy for Slides {
    type Document = String;
    type Target = (usize, usize);
    type Id = ();
    fn has_selection(_document: &String) -> bool {
        false
    }
    fn remap_id(_document: &mut Arc<String>, _id: ()) {}
    fn document_bytes(document: &String, _seen: &mut HashSet<usize>) -> usize {
        document.capacity()
    }
}
#[test]
fn independent_text_document_and_slide_target_use_same_engine() {
    let mut history = History::<Slides>::default();
    history.set_current_target((0, 0));
    history.record("Slide title", Arc::new("Original".to_owned()), (2, 3));
    let current = Arc::new("Updated".to_owned());
    let restored = history.try_undo(current).unwrap().unwrap();
    assert_eq!(&*restored, "Original");
    assert_eq!(history.current_target(), &(0, 0));
    let updated = history.try_redo(restored).unwrap().unwrap();
    assert_eq!(&*updated, "Updated");
    assert_eq!(history.current_target(), &(2, 3));
    assert!(!history.has_past_selection());
    let state = HistoryState::<Slides>::from_document("Preview", updated);
    assert!(!state.has_selection());
}

#[test]
fn mixed_edits_branches_jumps_and_recovery_preserve_every_revision() {
    let mut history = History::<Slides>::default();
    let mut current = Arc::new("Root".to_owned());
    let mut expected = std::collections::BTreeMap::from([(0, current.clone())]);
    let mut random = 7u64;
    for step in 0..256 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        match random % 4 {
            0 => {
                let before = current.clone();
                current = Arc::new(format!("Revision {step}"));
                history.record(format!("Edit {step}"), before, (step, 0));
                expected.insert(history.current_id(), current.clone());
            }
            1 | 2 => {
                let redo = random % 4 == 2;
                let destination = history.adjacent_id(redo);
                let restored = if redo { history.try_redo(current.clone()) } else { history.try_undo(current.clone()) }.unwrap();
                if let Some(restored) = restored {
                    current = restored;
                    assert_eq!(history.current_id(), destination.unwrap());
                } else {
                    assert!(destination.is_none());
                }
            }
            _ => {
                let index = (random as usize) % expected.len();
                let id = *expected.keys().nth(index).unwrap();
                let restored = history.state_by_id(current.clone(), id).unwrap().load_document().unwrap();
                history.jump_loaded(current, id, restored.clone()).unwrap();
                current = restored;
            }
        }
        assert_eq!(&current, expected.get(&history.current_id()).unwrap());
        assert_eq!(history.timeline().len(), expected.len());
        if step % 7 == 0 {
            history = History::<Slides>::from_checkpoint(history.checkpoint()).unwrap();
        }
        for (&id, original) in &expected {
            let restored = history.state_by_id(current.clone(), id).unwrap().load_document().unwrap();
            assert_eq!(&restored, original);
        }
    }
}

#[test]
fn pruning_one_alternate_path_preserves_other_branch_and_root() {
    let mut history = History::<Slides>::default();
    let root = Arc::new("Root".to_owned());
    history.record("A", root.clone(), (0, 0));
    let a = Arc::new("A".to_owned());
    history.record("B", a.clone(), (0, 0));
    let b = Arc::new("B".to_owned());
    history.record("C", b, (0, 0));
    let c = Arc::new("C".to_owned());
    history.jump_loaded(c, 1, a.clone()).unwrap();
    history.record("D", a, (0, 0));
    history.prune_descendants(2);
    assert_eq!(history.timeline().iter().map(|entry| entry.identity.id).collect::<Vec<_>>(), vec![0, 1, 4]);
    assert_eq!(history.current_id(), 4);
}
