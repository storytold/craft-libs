//! Flat Parquet tables: one primitive column per field, read row by row into typed values and written from a cell callback.
//! No Arrow runtime, database, Python, or system service is required.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use bytes::Bytes;
use parquet::{
    basic::{ConvertedType, LogicalType, Repetition, TimeUnit, Type as Physical},
    data_type::{BoolType, ByteArray, ByteArrayType, DoubleType},
    errors::ParquetError,
    file::{reader::FileReader, serialized_reader::SerializedFileReader, writer::SerializedFileWriter},
    record::Field,
    schema::types::Type,
};
use std::{collections::HashSet, sync::Arc};

mod limits;
#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Format(String),
    #[error("{0}")]
    Parquet(#[from] ParquetError),
}
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(message: &str) -> Error {
    Error::Format(message.into())
}

/// The longest text value or field name, in bytes, read or written.
pub const MAX_TEXT_BYTES: usize = 1 << 20;

/// Table size caps chosen by the caller.
///
/// `max_cells` counts one row of field names plus the data rows, times the columns, so it bounds
/// what a caller that keeps the names as a header row expands the table into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableLimits {
    /// Data rows, not counting a header row.
    pub max_rows: u32,
    pub max_columns: u32,
    pub max_cells: u64,
}

/// Recognizes Parquet even when it has an unrelated extension.
pub fn is_parquet(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PAR1") || bytes.starts_with(b"PARE")
}

/// One decoded value. Integers, decimals, dates and times keep the file's own units; giving them
/// a meaning such as a spreadsheet serial number or exact text is the caller's work.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// Signed integers of every width.
    Int(i64),
    /// Unsigned integers of every width.
    UInt(u64),
    /// Half, single and double precision floats; always finite.
    Float(f64),
    /// At most [`MAX_TEXT_BYTES`].
    Text(String),
    /// The exact decimal digits, at most [`MAX_TEXT_BYTES`].
    Decimal(String),
    /// Days since 1970-01-01.
    Date(i32),
    TimeMillis(i32),
    TimeMicros(i64),
    /// Milliseconds since 1970-01-01T00:00:00.
    TimestampMillis(i64),
    /// Microseconds since 1970-01-01T00:00:00.
    TimestampMicros(i64),
}

/// One column of a table being read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
}

/// What a row-by-row read found besides the rows themselves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadSummary {
    pub columns: Vec<Column>,
    pub rows: u64,
}

/// A whole table, collected by [`read`] and [`read_owned`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
}

/// Reads a borrowed buffer, calling `on_row` once per row after every check on the file's
/// metadata has passed. An error from `on_row` stops the read at once and comes back unchanged,
/// so a caller can cancel or refuse a row. Call [`read_owned_rows`] when the input is already
/// owned to avoid a copy.
pub fn read_rows(bytes: &[u8], limits: &TableLimits, on_row: impl FnMut(&[Value]) -> Result<()>) -> Result<ReadSummary> {
    let footer_start = limits::footer(bytes, limits)?;
    read_buffer(Bytes::copy_from_slice(bytes), footer_start, limits, on_row)
}

/// Reads an owned buffer, transferring its allocation to the decoder without copying its payload.
/// `on_row` works as in [`read_rows`].
pub fn read_owned_rows(bytes: Vec<u8>, limits: &TableLimits, on_row: impl FnMut(&[Value]) -> Result<()>) -> Result<ReadSummary> {
    let footer_start = limits::footer(&bytes, limits)?;
    read_buffer(Bytes::from(bytes), footer_start, limits, on_row)
}

/// Reads a borrowed buffer into a [`Table`]. The table holds a second copy of the data; callers
/// that store rows elsewhere should use [`read_rows`]. The text and decimal values the table keeps
/// share the 512 MiB decoded-data budget, so a dictionary entry repeated across many rows cannot
/// grow the table past it.
pub fn read(bytes: &[u8], limits: &TableLimits) -> Result<Table> {
    let footer_start = limits::footer(bytes, limits)?;
    collect(Bytes::copy_from_slice(bytes), footer_start, limits, limits::MAX_DECODED)
}

/// Reads an owned buffer into a [`Table`] without copying the input, under the same text budget
/// as [`read`].
pub fn read_owned(bytes: Vec<u8>, limits: &TableLimits) -> Result<Table> {
    let footer_start = limits::footer(&bytes, limits)?;
    collect(Bytes::from(bytes), footer_start, limits, limits::MAX_DECODED)
}

/// Collects every row, stopping with an error once the kept text and decimal values pass
/// `text_budget` bytes.
fn collect(bytes: Bytes, footer_start: usize, limits: &TableLimits, text_budget: usize) -> Result<Table> {
    let mut rows = Vec::new();
    let mut text_bytes = 0usize;
    let summary = read_buffer(bytes, footer_start, limits, |row| {
        for value in row {
            if let Value::Text(s) | Value::Decimal(s) = value {
                text_bytes = text_bytes.saturating_add(s.len());
            }
        }
        if text_bytes > text_budget {
            return Err(invalid("Parquet text exceeds the 512 MiB decoded-data budget; select a smaller table first"));
        }
        rows.push(row.to_vec());
        Ok(())
    })?;
    Ok(Table { columns: summary.columns, rows })
}

fn read_buffer(bytes: Bytes, footer_start: usize, limits: &TableLimits, mut on_row: impl FnMut(&[Value]) -> Result<()>) -> Result<ReadSummary> {
    let reader = SerializedFileReader::new(bytes.clone())?;
    let metadata = reader.metadata();
    let info = metadata.file_metadata();
    let fields = info.schema().get_fields();
    let cols = fields.len();
    let rows = u64::try_from(info.num_rows()).map_err(|_| invalid("Negative Parquet row count"))?;
    if cols == 0 || cols > limits.max_columns as usize || rows > u64::from(limits.max_rows) {
        return Err(Error::Format(format!(
            "Parquet table exceeds the table limits ({} data rows plus a header, {} columns)",
            thousands(u64::from(limits.max_rows)),
            thousands(u64::from(limits.max_columns))
        )));
    }
    if (rows + 1).saturating_mul(cols as u64) > limits.max_cells {
        return Err(Error::Format(format!(
            "Parquet table exceeds the {}-cell import budget; select a smaller table first",
            thousands(limits.max_cells)
        )));
    }
    if fields.iter().any(|f| !f.is_primitive() || !f.get_basic_info().has_repetition() || f.get_basic_info().repetition() == Repetition::REPEATED) {
        return Err(invalid("Nested, list and repeated Parquet fields are not supported; flatten the table first"));
    }
    let mut names = HashSet::new();
    for field in fields {
        if field.name().len() > MAX_TEXT_BYTES {
            return Err(invalid("A Parquet field name exceeds the 1 MiB text budget"));
        }
        // The record reader looks columns up by name, so a repeated name would read the last
        // column of that name in every position.
        if !names.insert(field.name()) {
            return Err(invalid("Parquet field names must be unique"));
        }
        // INT96 timestamps carry nanoseconds that the record reader cuts to milliseconds.
        if field.get_physical_type() == Physical::INT96 {
            return Err(invalid("INT96 Parquet timestamps are not supported; convert to microseconds first"));
        }
        if let Some(logical) = field.get_basic_info().logical_type_ref() {
            match logical {
                LogicalType::Time(t) | LogicalType::Timestamp(t) if t.unit == TimeUnit::NANOS => {
                    return Err(invalid("Nanosecond Parquet time/timestamps are not supported; convert to microseconds first"));
                }
                LogicalType::String
                | LogicalType::Enum
                | LogicalType::Decimal(_)
                | LogicalType::Date
                | LogicalType::Time(_)
                | LogicalType::Timestamp(_)
                | LogicalType::Integer(_)
                | LogicalType::Json
                | LogicalType::Float16 => {}
                _ => return Err(invalid("This Parquet logical field type is not supported")),
            }
        }
        // Unannotated binary has no value type, and a column of nulls never reaches `value`.
        let binary = matches!(field.get_physical_type(), Physical::BYTE_ARRAY | Physical::FIXED_LEN_BYTE_ARRAY);
        let annotated = matches!(
            field.get_basic_info().logical_type_ref(),
            Some(LogicalType::String | LogicalType::Enum | LogicalType::Json | LogicalType::Decimal(_) | LogicalType::Float16)
        ) || matches!(
            field.get_basic_info().converted_type(),
            ConvertedType::UTF8 | ConvertedType::ENUM | ConvertedType::JSON | ConvertedType::DECIMAL
        );
        if binary && !annotated {
            return Err(invalid("Binary and nested Parquet values are not supported"));
        }
    }
    // Check every advertised page allocation before the decoder sees it.
    limits::pages(&bytes, metadata, footer_start, limits)?;
    let mut count = 0u64;
    let mut values = Vec::with_capacity(cols);
    for row in reader.get_row_iter(None)? {
        if count >= rows {
            return Err(invalid("Parquet contains more rows than its metadata declares"));
        }
        let row = row?;
        values.clear();
        for (_, field) in row.into_columns() {
            values.push(value(field)?);
        }
        on_row(&values)?;
        count += 1;
    }
    if count != rows {
        return Err(invalid("Parquet row count does not match its metadata"));
    }
    Ok(ReadSummary { columns: fields.iter().map(|f| Column { name: f.name().into() }).collect(), rows })
}

fn value(field: Field) -> Result<Value> {
    Ok(match field {
        Field::Null => Value::Null,
        Field::Bool(v) => Value::Bool(v),
        Field::Str(s) => Value::Text(text(s)?),
        Field::Byte(v) => Value::Int(i64::from(v)),
        Field::Short(v) => Value::Int(i64::from(v)),
        Field::Int(v) => Value::Int(i64::from(v)),
        Field::Long(v) => Value::Int(v),
        Field::UByte(v) => Value::UInt(u64::from(v)),
        Field::UShort(v) => Value::UInt(u64::from(v)),
        Field::UInt(v) => Value::UInt(u64::from(v)),
        Field::ULong(v) => Value::UInt(v),
        Field::Float16(v) => finite(v.to_f64())?,
        Field::Float(v) => finite(f64::from(v))?,
        Field::Double(v) => finite(v)?,
        Field::Date(v) => Value::Date(v),
        Field::TimeMillis(v) => Value::TimeMillis(v),
        Field::TimeMicros(v) => Value::TimeMicros(v),
        Field::TimestampMillis(v) => Value::TimestampMillis(v),
        Field::TimestampMicros(v) => Value::TimestampMicros(v),
        // Keep exact decimal text rather than silently rounding arbitrary precision values.
        Field::Decimal(_) => Value::Decimal(text(field.to_string())?),
        _ => return Err(invalid("Binary and nested Parquet values are not supported")),
    })
}
fn finite(number: f64) -> Result<Value> {
    if !number.is_finite() {
        return Err(invalid("Non-finite Parquet numbers are not supported"));
    }
    Ok(Value::Float(number))
}
fn text(s: String) -> Result<String> {
    if s.len() > MAX_TEXT_BYTES {
        return Err(invalid("A Parquet text cell exceeds the 1 MiB text-cell budget"));
    }
    Ok(s)
}
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// One value to write. Text may be any string type the caller already holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell<S> {
    Empty,
    Text(S),
    Number(f64),
    Bool(bool),
}

#[derive(Clone, Copy)]
enum Kind {
    Text,
    Number,
    Bool,
}

/// Writes a table of `rows` data rows under the field `names`, Snappy-compressed, in row groups
/// of 4,096 rows. `cell(row, column)` gives each value and is called twice per cell: once to check
/// that every column holds one type (text, number or Boolean, with blanks allowed) and once to
/// write. An error it returns stops the write. A column of blanks is written as text.
pub fn write<S: AsRef<str>>(root: &str, names: &[&str], rows: u32, cell: impl Fn(u32, usize) -> Result<Cell<S>>) -> Result<Vec<u8>> {
    if names.is_empty() {
        return Err(invalid("A Parquet table needs at least one column"));
    }
    let mut seen = HashSet::new();
    let mut kinds = Vec::new();
    let mut fields = Vec::new();
    let mut text_bytes = 0usize;
    for (col, name) in names.iter().enumerate() {
        if name.len() > MAX_TEXT_BYTES {
            return Err(invalid("A Parquet field name exceeds the 1 MiB text budget"));
        }
        if name.is_empty() || !seen.insert(*name) {
            return Err(invalid("Parquet column headers must be nonempty and unique"));
        }
        let mut kind = None;
        for row in 0..rows {
            let next = match cell(row, col)? {
                Cell::Empty => continue,
                Cell::Text(s) => {
                    let s = s.as_ref();
                    if s.len() > MAX_TEXT_BYTES {
                        return Err(invalid("A text cell exceeds the 1 MiB export budget"));
                    }
                    text_bytes = text_bytes.saturating_add(s.len());
                    Kind::Text
                }
                Cell::Bool(_) => Kind::Bool,
                Cell::Number(v) if v.is_finite() => Kind::Number,
                Cell::Number(_) => return Err(invalid("Non-finite numbers cannot be written to Parquet")),
            };
            if kind.is_some_and(|k| std::mem::discriminant(&k) != std::mem::discriminant(&next)) {
                return Err(invalid("Parquet columns must have a consistent type (text, number or Boolean); blanks are allowed"));
            }
            kind = Some(next);
        }
        if text_bytes > limits::MAX_DECODED {
            return Err(invalid("Parquet export exceeds the 512 MiB text budget"));
        }
        let kind = kind.unwrap_or(Kind::Text);
        let physical = match kind {
            Kind::Text => Physical::BYTE_ARRAY,
            Kind::Number => Physical::DOUBLE,
            Kind::Bool => Physical::BOOLEAN,
        };
        let mut builder = Type::primitive_type_builder(name, physical).with_repetition(Repetition::OPTIONAL);
        if matches!(kind, Kind::Text) {
            builder = builder.with_converted_type(ConvertedType::UTF8);
        }
        fields.push(Arc::new(builder.build()?));
        kinds.push(kind);
    }
    let schema = Arc::new(Type::group_type_builder(root).with_fields(fields).build()?);
    let properties = parquet::file::properties::WriterProperties::builder().set_compression(parquet::basic::Compression::SNAPPY).build();
    let mut writer = SerializedFileWriter::new(Vec::new(), schema, Arc::new(properties))?;
    // Bound temporary vectors and dictionary state; never collect an entire column into memory.
    for first in (0..rows).step_by(4096) {
        let last = first.saturating_add(4095).min(rows - 1);
        let mut group = writer.next_row_group()?;
        for (col, kind) in kinds.iter().enumerate() {
            let mut column = group.next_column()?.ok_or_else(|| invalid("Missing Parquet writer column"))?;
            let values = (first..=last).map(|row| cell(row, col)).collect::<Result<Vec<_>>>()?;
            let definitions: Vec<i16> = values.iter().map(|v| i16::from(!matches!(v, Cell::Empty))).collect();
            match kind {
                Kind::Text => {
                    let data: Vec<ByteArray> =
                        values.iter().filter_map(|v| if let Cell::Text(s) = v { Some(ByteArray::from(s.as_ref())) } else { None }).collect();
                    column.typed::<ByteArrayType>().write_batch(&data, Some(&definitions), None)?;
                }
                Kind::Number => {
                    let data: Vec<f64> = values.iter().filter_map(|v| if let Cell::Number(n) = v { Some(*n) } else { None }).collect();
                    column.typed::<DoubleType>().write_batch(&data, Some(&definitions), None)?;
                }
                Kind::Bool => {
                    let data: Vec<bool> = values.iter().filter_map(|v| if let Cell::Bool(b) = v { Some(*b) } else { None }).collect();
                    column.typed::<BoolType>().write_batch(&data, Some(&definitions), None)?;
                }
            }
            column.close()?;
        }
        group.close()?;
    }
    Ok(writer.into_inner()?)
}
