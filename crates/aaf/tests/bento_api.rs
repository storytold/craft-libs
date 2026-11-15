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

#[test]
fn default_writer_builds_a_readable_empty_container() -> Result<(), String> {
    let bytes = Writer::default().finish();
    let container = Container::open(&bytes)?;
    let ids: Vec<_> = container.object_ids().collect();
    assert!(!ids.is_empty());
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    Ok(())
}
