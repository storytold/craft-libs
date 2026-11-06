//! The process-wide registry with two backends registered through the free functions.
//! (One scenario per file: the global registry cannot be reset within a process.)

mod common;

use common::escl_like::{AdfState, ESCL_ID, EsclLike};
use common::request;
use common::sane_like::{SANE_ID, SaneLike};
use craft_scan::{RegisterError, Source};

#[test]
fn free_functions_aggregate_and_route_across_registered_backends() {
    craft_scan::register(Box::new(SaneLike::new(1))).expect("sane");
    craft_scan::register(Box::new(EsclLike::new(AdfState::Loaded, 2))).expect("escl");
    assert!(craft_scan::supported());
    assert_eq!(craft_scan::backend_names(), ["sane", "escl"]);

    // The same name again is an error, not a panic, and changes nothing.
    assert_eq!(craft_scan::register(Box::new(SaneLike::new(5))), Err(RegisterError::DuplicateName { name: "sane" }));
    assert_eq!(craft_scan::backend_names(), ["sane", "escl"]);

    let list = craft_scan::scanner_list().expect("list");
    let found: Vec<&str> = list.items.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(found, [format!("sane:{SANE_ID}"), format!("escl:{ESCL_ID}")]);
    assert_eq!(craft_scan::scanners().expect("scanners").items.len(), 2);
    assert_eq!(craft_scan::scanner_details(&format!("escl:{ESCL_ID}")).expect("details").id, format!("escl:{ESCL_ID}"));
    assert!(craft_scan::status(&format!("sane:{SANE_ID}")).expect("status").is_none());
    assert!(craft_scan::status(&format!("escl:{ESCL_ID}")).expect("status").is_some());
    assert_eq!(craft_scan::acquire(&request(&format!("escl:{ESCL_ID}"), Source::Feeder, 2)).expect("acquire").pages.len(), 2);
    assert!(craft_scan::diagnostic_dump("").expect("dump").contains("== escl =="));
}
