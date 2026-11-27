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

#[test]
fn an_empty_row_group_reads() {
    let schema = Arc::new(parse_message_type("message test { OPTIONAL BYTE_ARRAY name (UTF8); OPTIONAL DOUBLE amount; }").unwrap());
    let mut writer = SerializedFileWriter::new(Vec::new(), schema, Default::default()).unwrap();
    for rows in [0, 2] {
        let mut group = writer.next_row_group().unwrap();
        let definitions = vec![1i16; rows];
        let mut col = group.next_column().unwrap().unwrap();
        col.typed::<ByteArrayType>().write_batch(&vec![ByteArray::from("a"); rows], Some(&definitions), None).unwrap();
        col.close().unwrap();
        let mut col = group.next_column().unwrap().unwrap();
        col.typed::<DoubleType>().write_batch(&vec![1.5; rows], Some(&definitions), None).unwrap();
        col.close().unwrap();
        group.close().unwrap();
    }
    let bytes = writer.into_inner().unwrap();
    let reader = SerializedFileReader::new(Bytes::from(bytes.clone())).unwrap();
    let empty = reader.metadata().row_group(0).column(0);
    assert_eq!((empty.data_page_offset(), empty.compressed_size()), (0, 0));
    let table = read(&bytes).unwrap();
    assert_eq!(table.rows, vec![vec![Value::Text("a".into()), Value::Float(1.5)]; 2]);
}

#[test]
fn a_row_callback_error_stops_the_read() {
    let bytes = write("test", &NAMES, 64, |row, col| Ok(if col == 1 { Cell::Number(f64::from(row)) } else { Cell::<&str>::Empty })).unwrap();
    for owned in [false, true] {
        let mut calls = 0;
        let on_row = |_: &[Value]| {
            calls += 1;
            if calls == 2 { Err(invalid("caller cancelled")) } else { Ok(()) }
        };
        let error =
            if owned { super::read_owned_rows(bytes.clone(), &LIMITS, on_row) } else { super::read_rows(&bytes, &LIMITS, on_row) }.unwrap_err();
        assert_eq!(error.to_string(), "caller cancelled");
        assert_eq!(calls, 2);
    }
    let mut calls = 0;
    let summary = super::read_rows(&bytes, &LIMITS, |_| {
        calls += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!((summary.rows, calls), (64, 64));
}

#[test]
fn the_largest_column_limit_still_reads() {
    let bytes = write_rows(&[vec![Cell::Text("a"), Cell::Number(1.0), Cell::Bool(true)]]).unwrap();
    let table = super::read(&bytes, &TableLimits { max_columns: u32::MAX, ..LIMITS }).unwrap();
    assert_eq!(table.rows.len(), 1);
}

#[test]
fn duplicate_field_names_are_rejected() {
    let schema = Arc::new(parse_message_type("message test { REQUIRED INT64 x; REQUIRED INT64 x; }").unwrap());
    let mut writer = SerializedFileWriter::new(Vec::new(), schema, Default::default()).unwrap();
    let mut group = writer.next_row_group().unwrap();
    for v in [10i64, 20] {
        let mut col = group.next_column().unwrap().unwrap();
        col.typed::<Int64Type>().write_batch(&[v], None, None).unwrap();
        col.close().unwrap();
    }
    group.close().unwrap();
    let bytes = writer.into_inner().unwrap();
    assert!(read(&bytes).unwrap_err().to_string().contains("unique"));
}

#[test]
fn int96_timestamps_are_rejected_rather_than_truncated() {
    use parquet::data_type::{Int96, Int96Type};
    let schema = Arc::new(parse_message_type("message test { REQUIRED INT96 ts; }").unwrap());
    let mut writer = SerializedFileWriter::new(Vec::new(), schema, Default::default()).unwrap();
    let mut group = writer.next_row_group().unwrap();
    let mut col = group.next_column().unwrap().unwrap();
    // 1,500,123 ns after 1970-01-01T00:00:00 (Julian day 2,440,588), which milliseconds cannot hold.
    let mut ts = Int96::new();
    ts.set_data(1_500_123, 0, 2_440_588);
    col.typed::<Int96Type>().write_batch(&[ts], None, None).unwrap();
    col.close().unwrap();
    group.close().unwrap();
    let bytes = writer.into_inner().unwrap();
    assert!(read(&bytes).unwrap_err().to_string().contains("INT96"));
}

#[test]
fn unannotated_binary_is_rejected_even_when_every_value_is_null() {
    let file = |definitions: &[i16], values: &[ByteArray]| {
        let schema = Arc::new(parse_message_type("message test { OPTIONAL BYTE_ARRAY b; }").unwrap());
        let mut writer = SerializedFileWriter::new(Vec::new(), schema, Default::default()).unwrap();
        let mut group = writer.next_row_group().unwrap();
        let mut col = group.next_column().unwrap().unwrap();
        col.typed::<ByteArrayType>().write_batch(values, Some(definitions), None).unwrap();
        col.close().unwrap();
        group.close().unwrap();
        writer.into_inner().unwrap()
    };
    for bytes in [file(&[1], &[ByteArray::from(vec![0xff, 0])]), file(&[0], &[]), file(&[], &[])] {
        assert!(read(&bytes).unwrap_err().to_string().contains("Binary"));
    }
}

#[test]
fn a_row_group_count_mismatch_fails_before_any_row_callback() {
    let schema = Arc::new(parse_message_type("message test { REQUIRED INT64 x; }").unwrap());
    let mut writer = SerializedFileWriter::new(Vec::new(), schema, Default::default()).unwrap();
    for v in [1i64, 2] {
        let mut group = writer.next_row_group().unwrap();
        let mut col = group.next_column().unwrap().unwrap();
        col.typed::<Int64Type>().write_batch(&[v], None, None).unwrap();
        col.close().unwrap();
        group.close().unwrap();
    }
    let bytes = writer.into_inner().unwrap();
    let footer_start = limits::footer(&bytes, &LIMITS).unwrap();
    // FileMetaData field 3, num_rows, is compact-Thrift header 0x16 followed by zigzag 2.
    let at: Vec<_> = (footer_start..bytes.len() - 9).filter(|&i| bytes[i..i + 2] == [0x16, 4]).collect();
    assert_eq!(at.len(), 1);
    for (declared, zigzag) in [(3, 6), (1, 2)] {
        let mut patched = bytes.clone();
        patched[at[0] + 1] = zigzag;
        let reader = SerializedFileReader::new(Bytes::from(patched.clone())).unwrap();
        assert_eq!(reader.metadata().file_metadata().num_rows(), declared);
        let mut calls = 0;
        let error = super::read_rows(&patched, &LIMITS, |_| {
            calls += 1;
            Ok(())
        })
        .unwrap_err();
        assert!(error.to_string().contains("row-group rows"), "{error}");
        assert_eq!(calls, 0);
    }
}

#[test]
fn the_writing_pass_checks_the_values_it_writes() {
    use std::cell::Cell as Calls;
    let long = "x".repeat(MAX_TEXT_BYTES + 1);
    let calls = Calls::new(0);
    let error = write("test", &["a"], 1, |_, _| {
        calls.set(calls.get() + 1);
        Ok(Cell::Text(if calls.get() == 1 { "short" } else { long.as_str() }))
    })
    .unwrap_err();
    assert!(error.to_string().contains("1 MiB"), "{error}");
    let calls = Calls::new(0);
    let error = write("test", &["a"], 1, |_, _| {
        calls.set(calls.get() + 1);
        Ok(Cell::<&str>::Number(if calls.get() == 1 { 1.0 } else { f64::INFINITY }))
    })
    .unwrap_err();
    assert!(error.to_string().contains("Non-finite"), "{error}");
    let calls = Calls::new(0);
    let error = write("test", &["a"], 1, |_, _| {
        calls.set(calls.get() + 1);
        Ok(if calls.get() == 1 { Cell::Number(1.0) } else { Cell::Text("1") })
    })
    .unwrap_err();
    assert!(error.to_string().contains("changed type"), "{error}");
}
