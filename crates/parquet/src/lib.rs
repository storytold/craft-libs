//! Flat Parquet tables: one primitive column per field, read row by row into typed values and written from a cell callback.
//! No Arrow runtime, database, Python, or system service is required.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use bytes::Bytes;
use parquet::{
    basic::{ConvertedType, LogicalType, Repetition, TimeUnit, Type as Physical},
    bloom_filter::Sbbf,
    column::page::PageReader,
    data_type::{BoolType, ByteArray, ByteArrayType, Decimal, DoubleType},
    errors::ParquetError,
    file::{
        metadata::{ColumnChunkMetaData, RowGroupMetaData},
        reader::{FileReader, RowGroupReader},
        serialized_reader::SerializedFileReader,
        writer::SerializedFileWriter,
    },
    record::{Field, reader::RowIter},
    schema::types::{SchemaDescriptor, Type},
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

/// The most digits a decimal field's precision or scale may declare, Arrow Decimal256's limit.
///
/// The record reader writes out every scale digit of every value, so a larger scale costs CPU time
/// per cell that the file's size does not bound.
pub const MAX_DECIMAL_DIGITS: i32 = 76;

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
/// owned to avoid a copy. The stored bytes of byte-backed decimal values, sign padding included,
/// share the 512 MiB decoded-data budget, so a dictionary entry repeated across many rows cannot
/// make the read check and format more than that.
pub fn read_rows(bytes: &[u8], limits: &TableLimits, on_row: impl FnMut(&[Value]) -> Result<()>) -> Result<ReadSummary> {
    let footer_start = limits::footer(bytes, limits)?;
    read_buffer(Bytes::copy_from_slice(bytes), footer_start, limits, limits::MAX_DECODED, on_row)
}

/// Reads an owned buffer, transferring its allocation to the decoder without copying its payload.
/// `on_row` works as in [`read_rows`].
pub fn read_owned_rows(bytes: Vec<u8>, limits: &TableLimits, on_row: impl FnMut(&[Value]) -> Result<()>) -> Result<ReadSummary> {
    let footer_start = limits::footer(&bytes, limits)?;
    read_buffer(Bytes::from(bytes), footer_start, limits, limits::MAX_DECODED, on_row)
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
    let summary = read_buffer(bytes, footer_start, limits, limits::MAX_DECODED, |row| {
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

/// Reads every row, stopping with an error once the stored bytes of byte-backed decimal values pass
/// `decimal_budget`.
fn read_buffer(
    bytes: Bytes,
    footer_start: usize,
    limits: &TableLimits,
    decimal_budget: usize,
    mut on_row: impl FnMut(&[Value]) -> Result<()>,
) -> Result<ReadSummary> {
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
        let decimal = field.get_basic_info().converted_type() == ConvertedType::DECIMAL
            || matches!(field.get_basic_info().logical_type_ref(), Some(LogicalType::Decimal(_)));
        if decimal && (field.get_precision() > MAX_DECIMAL_DIGITS || field.get_scale() > MAX_DECIMAL_DIGITS) {
            return Err(Error::Format(format!(
                "Parquet decimal field {:?} declares precision {} and scale {}, past the {MAX_DECIMAL_DIGITS}-digit limit",
                field.name(),
                field.get_precision(),
                field.get_scale()
            )));
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
    // The row iterator follows the row-group counts, so a mismatch has to fail before `on_row` runs.
    let declared = metadata.row_groups().iter().try_fold(0i64, |sum, group| sum.checked_add(group.num_rows()));
    if declared != Some(info.num_rows()) {
        return Err(invalid("Parquet row-group rows do not match the file row count"));
    }
    // Check every advertised page allocation before the decoder sees it.
    limits::pages(&bytes, metadata, footer_start, limits)?;
    let mut count = 0u64;
    // The page budget counts a dictionary entry once, but every row that refers to it hands the
    // length check and the formatter the whole entry again.
    let mut decimal_bytes_read = 0usize;
    // The record reader's UTF-8 error lists every byte of an invalid value, about ten bytes of
    // message per byte of input, so text columns are read as unannotated bytes and checked here.
    let mut text = Vec::with_capacity(cols);
    let mut plain = Vec::with_capacity(cols);
    for field in fields {
        let basic = field.get_basic_info();
        let is_text = field.get_physical_type() == Physical::BYTE_ARRAY
            && (matches!(basic.logical_type_ref(), Some(LogicalType::String | LogicalType::Enum | LogicalType::Json))
                || matches!(basic.converted_type(), ConvertedType::UTF8 | ConvertedType::ENUM | ConvertedType::JSON));
        text.push(is_text);
        plain.push(if is_text {
            Arc::new(Type::primitive_type_builder(field.name(), Physical::BYTE_ARRAY).with_repetition(basic.repetition()).build()?)
        } else {
            field.clone()
        });
    }
    let schema = Arc::new(SchemaDescriptor::new(Arc::new(Type::group_type_builder(info.schema().name()).with_fields(plain).build()?)));
    let mut values = Vec::with_capacity(cols);
    for group in 0..reader.num_row_groups() {
        let inner = reader.get_row_group(group)?;
        let group = PlainText { metadata: plain_text_metadata(inner.metadata(), &schema)?, inner };
        for row in RowIter::from_row_group(None, &group)? {
            if count >= rows {
                return Err(invalid("Parquet contains more rows than its metadata declares"));
            }
            let row = row?;
            values.clear();
            for ((name, field), is_text) in row.into_columns().into_iter().zip(&text) {
                if *is_text {
                    values.push(text_value(&name, field)?);
                    continue;
                }
                if let Field::Decimal(decimal @ Decimal::Bytes { .. }) = &field {
                    decimal_bytes_read = decimal_bytes_read.saturating_add(decimal.data().len());
                    if decimal_bytes_read > decimal_budget {
                        return Err(invalid("Parquet decimal data exceeds the 512 MiB decoded-data budget; select a smaller table first"));
                    }
                    decimal_length(&name, decimal)?;
                }
                values.push(value(field)?);
            }
            on_row(&values)?;
            count += 1;
        }
    }
    if count != rows {
        return Err(invalid("Parquet row count does not match its metadata"));
    }
    Ok(ReadSummary { columns: fields.iter().map(|f| Column { name: f.name().into() }).collect(), rows })
}

/// A row group whose column descriptors come from `metadata`, which drops the text annotations, so
/// the record reader hands text values over as bytes. Pages still come from `inner`.
struct PlainText<'a> {
    inner: Box<dyn RowGroupReader + 'a>,
    metadata: RowGroupMetaData,
}
impl RowGroupReader for PlainText<'_> {
    fn metadata(&self) -> &RowGroupMetaData {
        &self.metadata
    }
    fn num_columns(&self) -> usize {
        self.metadata.num_columns()
    }
    fn get_column_page_reader(&self, i: usize) -> parquet::errors::Result<Box<dyn PageReader>> {
        self.inner.get_column_page_reader(i)
    }
    fn get_column_bloom_filter(&self, _: usize) -> Option<&Sbbf> {
        None
    }
    fn get_row_iter(&self, projection: Option<Type>) -> parquet::errors::Result<RowIter<'_>> {
        RowIter::from_row_group(projection, self)
    }
}
/// Copies the row count and each column's value count and codec under `schema`'s descriptors. The
/// record reader reads nothing else from a row group's metadata; `inner` keeps the page offsets.
fn plain_text_metadata(group: &RowGroupMetaData, schema: &Arc<SchemaDescriptor>) -> Result<RowGroupMetaData> {
    if group.num_columns() != schema.num_columns() {
        return Err(invalid("Parquet row-group columns do not match the schema"));
    }
    let columns = group
        .columns()
        .iter()
        .enumerate()
        .map(|(i, column)| {
            ColumnChunkMetaData::builder(schema.column(i)).set_num_values(column.num_values()).set_compression(column.compression()).build()
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(RowGroupMetaData::builder(schema.clone()).set_num_rows(group.num_rows()).set_column_metadata(columns).build()?)
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
        // `read_buffer` has already bounded the value's significant bytes with `decimal_length`.
        Field::Decimal(decimal @ Decimal::Bytes { .. }) => Value::Decimal(text(Field::Decimal(trimmed_decimal(&decimal)).to_string())?),
        Field::Decimal(_) => Value::Decimal(text(field.to_string())?),
        _ => return Err(invalid("Binary and nested Parquet values are not supported")),
    })
}
/// Refuses a byte-backed decimal value longer than its declared precision needs, before the record
/// reader turns it into digits. INT32 and INT64 decimals hold at most 8 bytes and skip this check. Formatting costs time and memory that grow faster than the value's
/// length, and a page may hold a value of tens of MiB. Redundant sign-extension bytes do not count,
/// so a wide FIXED_LEN_BYTE_ARRAY column still reads.
fn decimal_length(name: &str, decimal: &Decimal) -> Result<()> {
    let length = significant_bytes(decimal.data()).len();
    let allowed = decimal_bytes(decimal.precision());
    if length > allowed {
        return Err(Error::Format(format!(
            "A value in Parquet decimal field {name:?} holds {length} significant bytes, more than the {allowed} that precision {} allows",
            decimal.precision()
        )));
    }
    Ok(())
}

/// Drops leading bytes that only repeat the sign of a big-endian two's complement number.
fn significant_bytes(mut bytes: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = bytes {
        match rest.first() {
            Some(next) if (*first == 0x00 && next & 0x80 == 0) || (*first == 0xFF && next & 0x80 != 0) => bytes = rest,
            _ => break,
        }
    }
    bytes
}

/// Copies a byte-backed decimal with only its significant bytes, so the record reader's conversion
/// copies at most the bytes that `decimal_length` allowed rather than the redundant sign padding too.
fn trimmed_decimal(decimal: &Decimal) -> Decimal {
    Decimal::from_bytes(ByteArray::from(significant_bytes(decimal.data()).to_vec()), decimal.precision(), decimal.scale())
}

/// The fewest two's complement bytes that hold every unscaled value of `precision` digits: the
/// smallest `n` with `floor(log10(2^(8n - 1) - 1)) >= precision`, as the Parquet format defines
/// for FIXED_LEN_BYTE_ARRAY decimals.
fn decimal_bytes(precision: i32) -> usize {
    ((f64::from(precision.max(1)) * std::f64::consts::LOG2_10 + 1.0) / 8.0).ceil() as usize
}
fn finite(number: f64) -> Result<Value> {
    if !number.is_finite() {
        return Err(invalid("Non-finite Parquet numbers are not supported"));
    }
    Ok(Value::Float(number))
}
/// Converts one value of a text column read as raw bytes, checking its length before copying it.
fn text_value(name: &str, field: Field) -> Result<Value> {
    let bytes = match field {
        Field::Null => return Ok(Value::Null),
        Field::Bytes(bytes) => bytes,
        _ => return Err(invalid("Binary and nested Parquet values are not supported")),
    };
    if bytes.len() > MAX_TEXT_BYTES {
        return Err(invalid("A Parquet text cell exceeds the 1 MiB text-cell budget"));
    }
    String::from_utf8(bytes.data().to_vec())
        .map(Value::Text)
        .map_err(|_| Error::Format(format!("A value in Parquet text field {name:?} is not valid UTF-8")))
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
/// write. Both passes check the value, and an error it returns stops the write. A column of
/// blanks is written as text.
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
            let next = match checked(cell(row, col)?, &mut text_bytes)? {
                Some(kind) => kind,
                None => continue,
            };
            if kind.is_some_and(|k| std::mem::discriminant(&k) != std::mem::discriminant(&next)) {
                return Err(invalid("Parquet columns must have a consistent type (text, number or Boolean); blanks are allowed"));
            }
            kind = Some(next);
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
    // The callback runs again for the values written, so they go through the same checks.
    let mut written_text = 0usize;
    // Bound temporary vectors and dictionary state; never collect an entire column into memory.
    for first in (0..rows).step_by(4096) {
        let last = first.saturating_add(4095).min(rows - 1);
        let mut group = writer.next_row_group()?;
        for (col, kind) in kinds.iter().enumerate() {
            let mut column = group.next_column()?.ok_or_else(|| invalid("Missing Parquet writer column"))?;
            let values = (first..=last)
                .map(|row| {
                    let value = cell(row, col)?;
                    checked_as(&value, *kind, &mut written_text)?;
                    Ok(value)
                })
                .collect::<Result<Vec<_>>>()?;
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

/// Checks one cell to write and returns its kind, or `None` for a blank. Text counts toward the
/// 1 MiB cell budget and, through `text_bytes`, the 512 MiB export budget.
fn checked<S: AsRef<str>>(value: Cell<S>, text_bytes: &mut usize) -> Result<Option<Kind>> {
    Ok(Some(match value {
        Cell::Empty => return Ok(None),
        Cell::Text(s) => {
            let s = s.as_ref();
            if s.len() > MAX_TEXT_BYTES {
                return Err(invalid("A text cell exceeds the 1 MiB export budget"));
            }
            *text_bytes = text_bytes.saturating_add(s.len());
            if *text_bytes > limits::MAX_DECODED {
                return Err(invalid("Parquet export exceeds the 512 MiB text budget"));
            }
            Kind::Text
        }
        Cell::Bool(_) => Kind::Bool,
        Cell::Number(v) if v.is_finite() => Kind::Number,
        Cell::Number(_) => return Err(invalid("Non-finite numbers cannot be written to Parquet")),
    }))
}

/// Checks a cell on the writing pass, where it must also keep its column's kind.
fn checked_as<S: AsRef<str>>(value: &Cell<S>, kind: Kind, text_bytes: &mut usize) -> Result<()> {
    let borrowed = match value {
        Cell::Empty => Cell::Empty,
        Cell::Text(s) => Cell::Text(s.as_ref()),
        Cell::Number(n) => Cell::Number(*n),
        Cell::Bool(b) => Cell::Bool(*b),
    };
    match checked(borrowed, text_bytes)? {
        Some(found) if std::mem::discriminant(&found) != std::mem::discriminant(&kind) => {
            Err(invalid("A Parquet cell changed type between the checking and writing passes"))
        }
        _ => Ok(()),
    }
}
