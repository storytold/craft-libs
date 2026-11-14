//! Builds the reference app's model for every project file in `TC_IMPORT_CORPUS` (skipped without it). Every project
//! must build, keep no object raw, have a layer stack per texture set that reaches the stored layers, and hold no field
//! the reader's field table (`texturecraft_import_reader::model::fields`) doesn't list: a newer version's fields fail here until each
//! is classified as read, ignored (with the reason) or unhandled (with its default).

use texturecraft_import_reader::model::StackSummary;
use texturecraft_import_reader::project::ProjectFile;

use crate::common;

#[test]
#[ignore = "needs TC_IMPORT_CORPUS"]
fn every_corpus_project_builds_a_model() {
    let Some(files) = common::files() else {
        return;
    };
    let mut failures = Vec::new();
    for path in &files {
        let built = ProjectFile::open(path).and_then(|p| Ok((p.model()?, p.document_census()?)));
        match built {
            Ok(((doc, report), census)) => {
                assert_eq!(report.total(), census.total(), "{}", path.display());
                assert_eq!(report.raw(), 0, "{}: {:?}", path.display(), report.raw_classes());
                assert!(report.unknown_fields.is_empty(), "{}: fields missing from the field table: {:?}", path.display(), report.unknown_fields);
                let layers = census.count("DataLayerColor") + census.count("DataLayerGroup");
                assert_eq!(StackSummary::of(&doc).layers, layers, "{}", path.display());
            }
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
