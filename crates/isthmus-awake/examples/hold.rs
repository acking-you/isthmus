//! Holds the keep-awake request for a few seconds, to check by hand that
//! the system lists it: `pmset -g assertions` on a Mac, `powercfg
//! /requests` (as administrator) on Windows. The choice is kept in a
//! scratch folder, never the user's data directory.
//!
//! `cargo run -p isthmus-awake --example hold -- 30`

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use isthmus_awake::KeepAwake;

fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|text| text.parse().ok())
        .unwrap_or(10);
    let scratch = std::env::temp_dir().join(format!("isthmus-awake-hold-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let keep = KeepAwake::start(&scratch).unwrap();
    let status = keep.set(true).unwrap();
    println!("{status:?}; held for {seconds} s");
    std::thread::sleep(Duration::from_secs(seconds));
    keep.shutdown();
    println!("released: {:?}", keep.status());
    std::fs::remove_dir_all(&scratch).unwrap();
}
