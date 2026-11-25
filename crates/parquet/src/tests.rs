#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use parquet::{
    data_type::{ByteArray, ByteArrayType, Int64Type},
    file::properties::WriterProperties,
    schema::parser::parse_message_type,
};

/// A spreadsheet's limits: 1,048,576 rows including the header, 16,384 columns, 5,000,000 cells.
pub(crate) const LIMITS: TableLimits = TableLimits { max_rows: 1_048_575, max_columns: 16_384, max_cells: 5_000_000 };

fn read(bytes: &[u8]) -> Result<Table> {
    let table = super::read(bytes, &LIMITS)?;
    let owned = super::read_owned(bytes.to_vec(), &LIMITS)?;
    assert_eq!(table, owned);
    Ok(table)
}

const NAMES: [&str; 3] = ["Name", "Amount", "Active"];

fn write_rows(rows: &[Vec<Cell<&str>>]) -> Result<Vec<u8>> {
    write("test", &NAMES, rows.len() as u32, |row, col| Ok(rows[row as usize][col].clone()))
}

fn expected(cell: &Cell<&str>) -> Value {
    match cell {
        Cell::Empty => Value::Null,
        Cell::Text(s) => Value::Text((*s).into()),
        Cell::Number(n) => Value::Float(*n),
        Cell::Bool(b) => Value::Bool(*b),
    }
}

#[test]
fn typed_values_nulls_empty_strings_and_formula_like_text_roundtrip() {
    let rows = vec![
        vec![Cell::Text("=SUM(1,2)"), Cell::Number(-12.5), Cell::Empty],
        vec![Cell::Text(""), Cell::Empty, Cell::Bool(false)],
        vec![Cell::Text("Unicode: café 日本語"), Cell::Number(0.0), Cell::Bool(true)],
    ];
    let table = read(&write_rows(&rows).unwrap()).unwrap();
    assert_eq!(table.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), NAMES);
    assert_eq!(table.rows.len(), rows.len());
    for (got, want) in table.rows.iter().zip(&rows) {
        assert_eq!(got, &want.iter().map(expected).collect::<Vec<_>>());
    }
}

#[test]
fn multiple_row_groups_and_header_only_table() {
    let table = read(&write_rows(&[]).unwrap()).unwrap();
    assert_eq!(table.columns.len(), 3);
    assert!(table.rows.is_empty());
    let bytes = write("test", &NAMES, 8200, |row, col| Ok(if col == 1 { Cell::Number(f64::from(row + 1)) } else { Cell::<&str>::Empty })).unwrap();
    let reader = SerializedFileReader::new(Bytes::from(bytes.clone())).unwrap();
    assert_eq!(reader.num_row_groups(), 3);
    let table = read(&bytes).unwrap();
    assert_eq!(table.rows[8199][1], Value::Float(8200.0));
}

fn integers(converted: ConvertedType, values: &[i64], compression: parquet::basic::Compression) -> Vec<u8> {
    let field =
        Type::primitive_type_builder("value", Physical::INT64).with_repetition(Repetition::REQUIRED).with_converted_type(converted).build().unwrap();
    let schema = Arc::new(Type::group_type_builder("test").with_fields(vec![Arc::new(field)]).build().unwrap());
    let properties = Arc::new(WriterProperties::builder().set_compression(compression).build());
    let mut writer = SerializedFileWriter::new(Vec::new(), schema, properties).unwrap();
    let mut group = writer.next_row_group().unwrap();
    let mut col = group.next_column().unwrap().unwrap();
    col.typed::<Int64Type>().write_batch(values, None, None).unwrap();
    col.close().unwrap();
    group.close().unwrap();
    writer.into_inner().unwrap()
}

#[test]
fn large_integers_come_back_exact() {
    let bytes = integers(ConvertedType::INT_64, &[9_007_199_254_740_992, 9_007_199_254_740_993, i64::MIN], parquet::basic::Compression::SNAPPY);
    let table = read(&bytes).unwrap();
    let got: Vec<_> = table.rows.iter().map(|r| r[0].clone()).collect();
    assert_eq!(got, [Value::Int(9_007_199_254_740_992), Value::Int(9_007_199_254_740_993), Value::Int(i64::MIN)]);
}

#[test]
fn common_compression_codecs() {
    use parquet::basic::Compression as C;
    let codecs = [C::UNCOMPRESSED, C::SNAPPY, C::GZIP(Default::default()), C::BROTLI(Default::default()), C::LZ4_RAW];
    for codec in codecs {
        let bytes = integers(ConvertedType::INT_64, &[5, 10], codec);
        assert!(read(&bytes).is_ok(), "{codec:?}");
    }
    #[cfg(not(target_arch = "wasm32"))]
    assert!(read(&integers(ConvertedType::INT_64, &[5], C::ZSTD(Default::default()))).is_ok());
}

#[test]
fn export_refuses_mixed_types_and_bad_headers() {
    let mixed = [vec![Cell::Empty, Cell::Number(1.0), Cell::Empty], vec![Cell::Empty, Cell::Text("2"), Cell::Empty]];
    assert!(write_rows(&mixed).unwrap_err().to_string().contains("consistent type"));
    let names = ["Name", "Name", "Active"];
    assert!(write("test", &names, 0, |_, _| Ok(Cell::<&str>::Empty)).unwrap_err().to_string().contains("unique"));
}

#[test]
fn cell_callback_errors_and_nonfinite_numbers_stop_the_write() {
    let error = write("test", &NAMES, 1, |_, col| if col == 1 { Err(invalid("caller refused")) } else { Ok(Cell::<&str>::Empty) }).unwrap_err();
    assert_eq!(error.to_string(), "caller refused");
    assert!(write("test", &NAMES, 1, |_, _| Ok(Cell::<&str>::Number(f64::INFINITY))).is_err());
}

#[test]
fn table_limits_are_enforced() {
    let bytes = integers(ConvertedType::INT_64, &[1, 2], parquet::basic::Compression::SNAPPY);
    assert!(super::read(&bytes, &TableLimits { max_rows: 1, ..LIMITS }).unwrap_err().to_string().contains("1 data rows"));
    assert!(super::read(&bytes, &TableLimits { max_cells: 2, ..LIMITS }).unwrap_err().to_string().contains("2-cell"));
    assert_eq!(super::read(&bytes, &TableLimits { max_rows: 2, max_columns: 1, max_cells: 3 }).unwrap().rows.len(), 2);
}

#[test]
fn truncated_and_random_bytes_never_panic() {
    let bytes = integers(ConvertedType::INT_64, &[1, 2], parquet::basic::Compression::SNAPPY);
    for end in 0..bytes.len() {
        assert!(read(&bytes[..end]).is_err());
        assert!(super::read_owned(bytes[..end].to_vec(), &LIMITS).is_err());
    }
    let mut seed = 1u64;
    for len in 0..256 {
        let mut bytes = vec![0; len];
        for b in &mut bytes {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (seed >> 32) as u8;
        }
        assert!(read(&bytes).is_err());
    }
}

#[test]
fn nested_schema_is_rejected_before_decoder() {
    let schema = Arc::new(parse_message_type("message test { OPTIONAL group nested { REQUIRED INT64 n; } }").unwrap());
    let bytes = SerializedFileWriter::new(Vec::new(), schema, Default::default()).unwrap().into_inner().unwrap();
    assert!(read(&bytes).unwrap_err().to_string().contains("Nested"));
}

#[test]
fn text_and_nonfinite_limits_are_explicit() {
    assert!(value(Field::Double(f64::NAN)).is_err());
    assert!(value(Field::Str("x".repeat(MAX_TEXT_BYTES + 1))).is_err());
    let long = "x".repeat(MAX_TEXT_BYTES + 1);
    assert!(write("test", &NAMES, 1, |_, col| Ok(if col == 0 { Cell::Text(long.as_str()) } else { Cell::Empty })).is_err());
}

#[test]
fn mutated_real_envelopes_do_not_panic() {
    let original = integers(ConvertedType::INT_64, &[1, 2, 3], parquet::basic::Compression::SNAPPY);
    for at in 0..original.len() {
        let mut bytes = original.clone();
        bytes[at] ^= 0x80;
        let result = std::panic::catch_unwind(|| read(&bytes));
        assert!(result.is_ok(), "panic at mutated byte {at}");
        let owned = std::panic::catch_unwind(|| super::read_owned(bytes, &LIMITS));
        assert!(owned.is_ok(), "owned panic at mutated byte {at}");
    }
}

#[test]
fn a_repeated_dictionary_entry_cannot_grow_a_table_past_the_text_budget() {
    let field = Type::primitive_type_builder("text", Physical::BYTE_ARRAY)
        .with_repetition(Repetition::REQUIRED)
        .with_converted_type(ConvertedType::UTF8)
        .build()
        .unwrap();
    let schema = Arc::new(Type::group_type_builder("test").with_fields(vec![Arc::new(field)]).build().unwrap());
    let properties = Arc::new(WriterProperties::builder().set_dictionary_enabled(true).build());
    let mut writer = SerializedFileWriter::new(Vec::new(), schema, properties).unwrap();
    let mut group = writer.next_row_group().unwrap();
    let mut col = group.next_column().unwrap().unwrap();
    let entry = "x".repeat(64 << 10);
    let values = vec![ByteArray::from(entry.as_str()); 64];
    col.typed::<ByteArrayType>().write_batch(&values, None, None).unwrap();
    col.close().unwrap();
    group.close().unwrap();
    let bytes = writer.into_inner().unwrap();
    // 64 rows of one 64 KiB entry decode to 4 MiB from a file a small fraction of that size.
    assert!(bytes.len() < 256 << 10, "{} bytes", bytes.len());
    let footer_start = limits::footer(&bytes, &LIMITS).unwrap();
    let table = |budget| collect(Bytes::from(bytes.clone()), footer_start, &LIMITS, budget);
    assert!(table(1 << 20).unwrap_err().to_string().contains("decoded-data budget"));
    assert_eq!(table(4 << 20).unwrap().rows.len(), 64);
    assert_eq!(read(&bytes).unwrap().rows.len(), 64);
}

