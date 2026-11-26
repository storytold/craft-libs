#[derive(Component, Props)]
pub struct Chip {
    #[common]
    common: Common,
    enabled: bool,
    id_salt: u64,
    tooltip: Option<String>,
}
