//! Local import timing without printing any cell contents.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("usage: inspect file.parquet [--owned]")?;
    let owned = args.any(|arg| arg == "--owned");
    let bytes = std::fs::read(path)?;
    // Native-only timing; std clocks trap on wasm (see clippy.toml).
    #[allow(clippy::disallowed_methods)]
    let start = std::time::Instant::now();
    let input_bytes = bytes.len();
    let limits = craft_parquet::TableLimits { max_rows: 1_048_575, max_columns: 16_384, max_cells: 5_000_000 };
    let mut values = 0u64;
    let on_row = |row: &[craft_parquet::Value]| {
        values += row.iter().filter(|v| **v != craft_parquet::Value::Null).count() as u64;
        Ok(())
    };
    let summary = if owned { craft_parquet::read_owned_rows(bytes, &limits, on_row)? } else { craft_parquet::read_rows(&bytes, &limits, on_row)? };
    println!(
        "input_bytes={} rows={} columns={} values={} seconds={:.3}",
        input_bytes,
        summary.rows,
        summary.columns.len(),
        values,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
