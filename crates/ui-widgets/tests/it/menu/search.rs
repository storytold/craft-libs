//! The help search's ranking: every word of a query, and the command id.

use craft_i18n::Folder;
use craft_ui::menu::search::{MAX_RESULTS, search_order};
use craft_ui::testing::trial;

/// Runs `query` over `texts` in English.
fn order(query: &str, texts: &[[&str; 4]]) -> Vec<usize> {
    search_order(&Folder::new("en"), query, texts)
}

#[trial]
fn every_word_of_a_query_must_match_in_any_order_and_the_command_id_ranks_with_the_path() {
    let table = [["Table", "Table", "Insert › Tables", "insert.table"]];
    assert_eq!(order("insert table", &table), vec![0]);
    assert_eq!(order("  Table   INSERT ", &table), vec![0], "any order, any case, extra spaces");
    assert!(order("insert chart", &table).is_empty(), "every word has to match");
    assert!(order("", &table).is_empty());
    assert!(order("   ", &table).is_empty());

    let one = [["Other", "", "Format › Italic", ""], ["Italic", "", "Format", ""]];
    assert_eq!(order("italic", &one), vec![1, 0], "a label match ranks before a path-only match");

    let ids = [["Slanted", "", "Format", "format.italic"], ["Format.italic tool", "", "Tools", "tools.x"]];
    assert_eq!(order("format.italic", &ids), vec![1, 0], "the id matches, ranking after a label match");
    assert_eq!(order("FORMAT.ITALIC", &ids), vec![1, 0]);
    assert_eq!(order("slanted format.italic", &ids), vec![0], "a word in the label and a word in the id");

    let labels: Vec<String> = (0..MAX_RESULTS + 5).map(|n| format!("Item {n}")).collect();
    let many: Vec<[&str; 4]> = labels.iter().map(|label| [label.as_str(), "", "", ""]).collect();
    assert_eq!(order("item", &many).len(), MAX_RESULTS);
}
