//! Memo keys: equal props key alike, and every input feeds the key.

use craft_keymap::CommandRef;
use craft_ui::contract::{MemoKey, MemoProps, Stateless, Tracked};
use craft_ui::runtime::env::Label;
use craft_ui::testing::trial;

#[derive(Clone, MemoProps)]
struct Inner {
    size: f32,
    names: Vec<String>,
}

#[derive(Clone, MemoProps)]
struct Pair(u8, u8);

#[derive(Clone, MemoProps)]
enum Mode {
    Off,
    Count(u32),
    Range { from: u32, to: u32 },
}

#[derive(Clone, MemoProps)]
struct Props<'a, #[memo_props(stateless)] F> {
    title: &'a str,
    rest: &'a str,
    shown: Option<u32>,
    inner: Inner,
    pair: Pair,
    mode: Mode,
    choice: Option<Mode>,
    grid: Vec<Vec<u8>>,
    key: u32,
    label: Label,
    command: CommandRef,
    model: &'a Tracked<Vec<String>>,
    path: std::path::PathBuf,
    #[memo_props(hash = "four bytes; the derived Hash feeds them all")]
    address: std::net::Ipv4Addr,
    english: Stateless<F>,
}

fn english() -> &'static str {
    "English"
}

fn other() -> &'static str {
    "Other"
}

fn props<'a, F>(model: &'a Tracked<Vec<String>>, english: Stateless<F>) -> Props<'a, F> {
    Props {
        title: "ab",
        rest: "",
        shown: None,
        inner: Inner { size: 1.0, names: vec!["x".to_owned()] },
        pair: Pair(1, 2),
        mode: Mode::Count(3),
        choice: Some(Mode::Off),
        grid: vec![vec![1, 2], vec![3]],
        key: 7,
        label: Label::Static("Open"),
        command: CommandRef::with_params("file.open", serde_json::json!({ "recent": 1 })),
        model,
        path: "a/b".into(),
        address: std::net::Ipv4Addr::new(10, 0, 0, 1),
        english,
    }
}

fn base(model: &Tracked<Vec<String>>) -> Props<'_, impl Copy + 'static> {
    props(model, Stateless::new(english))
}

/// Props built twice key alike; changing any one input, a nested field and an enum variant or its field included, gives
/// a key no other change gives.
#[trial]
fn equal_props_give_equal_keys_and_any_change_gives_a_new_key() {
    let model = Tracked::new(vec!["a".to_owned()]);
    let start = MemoKey::of(&base(&model));
    assert_eq!(MemoKey::of(&base(&model)), start, "equal props key alike");
    assert_eq!(MemoKey::of(&base(&model).clone()), start, "a clone keys alike");
    let mut shared = base(&model);
    shared.label = Label::Shared("Open".into());
    assert_eq!(MemoKey::of(&shared), start, "a label keys by its text");

    let mut keys = vec![start, MemoKey::of(&props(&model, Stateless::new(other)))];
    let mut changed = |change: &dyn Fn(&mut Props<'_, _>)| {
        let mut props = base(&model);
        change(&mut props);
        keys.push(MemoKey::of(&props));
    };
    changed(&|props| props.title = "abc");
    changed(&|props| (props.title, props.rest) = ("a", "b"));
    changed(&|props| props.shown = Some(0));
    changed(&|props| props.inner.size = 2.0);
    changed(&|props| props.inner.names[0].push('y'));
    changed(&|props| props.inner.names.push(String::new()));
    changed(&|props| props.pair = Pair(2, 1));
    changed(&|props| props.mode = Mode::Count(4));
    changed(&|props| props.mode = Mode::Off);
    changed(&|props| props.mode = Mode::Range { from: 3, to: 0 });
    changed(&|props| props.label = Label::Static("Close"));
    changed(&|props| props.command = CommandRef::with_params("file.open", serde_json::json!({ "recent": 2 })));
    changed(&|props| props.command = CommandRef::new("file.open"));
    changed(&|props| props.path = "a/c".into());
    changed(&|props| props.choice = None);
    changed(&|props| props.choice = Some(Mode::Count(0)));
    changed(&|props| props.choice = Some(Mode::Range { from: 0, to: 0 }));
    changed(&|props| props.grid = vec![vec![1], vec![2, 3]]);
    changed(&|props| props.grid = vec![vec![1, 2, 3]]);
    changed(&|props| props.key = 8);
    changed(&|props| props.address = std::net::Ipv4Addr::new(10, 0, 0, 2));

    let mut written = model.clone();
    let cloned = MemoKey::of(&base(&written));
    written.push("b".to_owned());
    keys.extend([cloned, MemoKey::of(&base(&written))]);

    let mut distinct = keys.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), keys.len(), "every change gives its own key: {keys:x?}");

    // Props key by their values only: two empty props types key alike. The memoized view's own key adds the props
    // type's `TypeId`.
    #[derive(MemoProps)]
    struct Empty;
    #[derive(MemoProps)]
    struct Nothing {}
    assert_eq!(MemoKey::of(&Empty), MemoKey::of(&Nothing {}));
}
