use craft_aaf::omf::bento::{Container, Writer, sniff};

#[test]
fn external_consumer_reads_named_values_and_references() {
    let mut writer = Writer::new();
    let first = writer.new_object();
    let second = writer.new_object();
    writer.set(first, "test:value", "test:type", b"long value");
    writer.set_references(first, &[second]);
    let bytes = writer.finish();
    assert!(sniff(&bytes));
    let container = Container::open(&bytes).unwrap();
    assert!(container.object_ids().any(|id| id == first));
    assert_eq!(container.get(first, "test:value").as_deref(), Some(b"long value".as_slice()));
    assert_eq!(container.resolve(first, second), second);
    assert!(Container::open(&bytes[..bytes.len() - 1]).is_err());
}
