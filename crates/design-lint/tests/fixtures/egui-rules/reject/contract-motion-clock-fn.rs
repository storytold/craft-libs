fn f(cell: &std::sync::OnceLock<std::time::Instant>) -> std::time::Instant {
    *cell.get_or_init(std::time::Instant::now)
}
