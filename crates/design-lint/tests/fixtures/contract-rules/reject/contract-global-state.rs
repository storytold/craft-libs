use std::cell::RefCell;
use std::fs;
use std::sync::{Mutex, OnceLock};

static COUNTER: Mutex<u32> = Mutex::new(0);
static mut RAW: u32 = 0;
static CACHE: OnceLock<String> = OnceLock::new();

thread_local! {
    static LOCAL: RefCell<u32> = const { RefCell::new(0) };
}

struct Props;

impl MemoProps for Props {}

impl PlainData for Props {}

fn project() -> Tracked {
    Tracked::__split_for_bind(&[])
}

fn read(path: &str) -> Vec<u8> {
    static INNER: Mutex<u32> = Mutex::new(0);
    std::fs::read(path).unwrap_or_default()
}
