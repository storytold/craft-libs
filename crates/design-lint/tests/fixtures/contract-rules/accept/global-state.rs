use std::path::Path;

const COLUMNS: [u32; 2] = [1, 2];

#[derive(MemoProps)]
struct Props {
    path: String,
}

fn exists(path: &Path) -> bool {
    COLUMNS.len() > 1 && path.is_absolute()
}

#[cfg(test)]
mod tests {
    static SEEN: std::sync::Mutex<u32> = std::sync::Mutex::new(0);
}

#[cfg(all(test, feature = "screen-testing"))]
mod counted {
    thread_local! {
        static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
}
