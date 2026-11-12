//! The chip field's draft rules.

use craft_behavior::Codec as _;
use craft_ui::testing::trial;
use craft_ui::text::ChipCodec;

fn chips(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// A draft names one chip per piece between separators, trimmed; pieces already in the list and empty pieces add
/// nothing, and the new chips follow the old ones in the order typed.
#[trial]
fn a_draft_adds_its_pieces_after_the_existing_chips() {
    let codec = ChipCodec::default();
    let start = chips(&["kite"]);
    assert_eq!(codec.parse("red, beach ;kite,, \n sky", &start), Ok(chips(&["kite", "red", "beach", "sky"])));
    assert_eq!(codec.parse("  ,; ", &start), Ok(start.clone()), "a draft of separators adds nothing");
    assert_eq!(codec.parse("red, red", &Vec::new()), Ok(chips(&["red"])), "a piece the draft names twice is added once");
    assert_eq!(codec.parse("two words", &Vec::new()), Ok(chips(&["two words"])), "a space does not end a chip");
}

mod field {
    use craft_ui::contract::{Bind, Role, Show as _};
    use craft_ui::testing::access::name;
    use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, pointer, trial};
    use craft_ui::text::ChipField;
    use egui::{Key, Modifiers, Ui};

    fn keywords() -> Vec<String> {
        ["kite", "red", "beach", "sky", "sand", "dune", "wave", "tide"].map(String::from).to_vec()
    }

    /// Eight chips in one line at most, with Clear All; the field's report goes to the story data.
    fn field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
        let out = ChipField::new("Keywords", Bind::bound(data.value("chips", keywords()))).max_lines(1).clear_all().show(ui);
        if let Some(at) = out.activated {
            *data.value("activated", None::<usize>) = Some(at);
        }
        if let Some(at) = out.context_menu {
            *data.value("menu", None::<usize>) = Some(at);
        }
        out.edit.into()
    }

    fn fixture() -> Fixture {
        let mut fixture = Fixture::story(&Story::new("chip_field", field));
        fixture.room(egui::vec2(480.0, 400.0));
        fixture.settle();
        fixture
    }

    fn chips(fixture: &mut Fixture) -> Vec<String> {
        fixture.data().value("chips", keywords()).clone()
    }

    fn button(fixture: &Fixture, starts: &str) -> craft_ui::testing::Target {
        let tree = fixture.tree();
        let found = fixture.by_role(Role::BUTTON).into_iter().find(|node| name(tree, node.node).is_some_and(|name| name.starts_with(starts)));
        found.unwrap_or_else(|| panic!("a button named {starts}…"))
    }

    /// Past the line limit the rest of the chips sit behind "+N more", whose menu removes the chip chosen there; Clear
    /// All removes every chip.
    #[trial]
    fn more_lists_the_hidden_chips_and_clear_all_removes_every_chip() {
        let mut fixture = fixture();
        let shown = fixture.by_role(Role::LIST_ITEM).len();
        assert!(shown > 0 && shown < 8, "one line shows some of the chips ({shown})");
        let more = button(&fixture, "+");
        fixture.click(&more);
        fixture.settle();
        let hidden = keywords()[shown..].to_vec();
        let row = fixture.by_label(&hidden[0]).expect("the menu lists the first hidden chip");
        fixture.click(&row);
        fixture.settle();
        let left = chips(&mut fixture);
        assert!(!left.contains(&hidden[0]) && left.len() == 7, "choosing a hidden chip removes it: {left:?}");
        let clear = button(&fixture, "Clear All");
        fixture.click(&clear);
        fixture.settle();
        assert!(chips(&mut fixture).is_empty(), "Clear All removes every chip");
    }

    /// A click on a chip is reported, and a secondary click on another chip asks for that chip's menu, not the focused
    /// one's.
    #[trial]
    fn a_chip_reports_its_click_and_its_context_menu() {
        let mut fixture = fixture();
        let items = fixture.by_role(Role::LIST_ITEM);
        fixture.click(&items[0]);
        fixture.settle();
        assert_eq!(*fixture.data().value("activated", None::<usize>), Some(0), "a click on a chip reports it");
        let at = items[1].center();
        fixture.frame(vec![pointer::moved(at)]);
        fixture.frame(vec![pointer::secondary(at, true)]);
        fixture.frame(vec![pointer::secondary(at, false)]);
        fixture.settle();
        assert_eq!(*fixture.data().value("menu", None::<usize>), Some(1), "a secondary click asks for the menu of the chip under the pointer");
        *fixture.data().value("menu", None::<usize>) = None;
        // Tab reaches the draft, and Backspace in the empty draft focuses the last shown chip.
        fixture.key(Key::Tab, Modifiers::NONE);
        fixture.key(Key::Backspace, Modifiers::NONE);
        fixture.key(Key::F10, Modifiers::SHIFT);
        fixture.settle();
        let last = items.len() - 1;
        assert_eq!(*fixture.data().value("menu", None::<usize>), Some(last), "Shift+F10 asks for the focused chip's menu");
    }
}
