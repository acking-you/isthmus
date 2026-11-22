//! Prints this computer's clipboard access and counter, read-only: whether
//! Isthmus may read it without asking, by hand.
//!
//! `cargo run -p isthmus-clipboard --example probe`

use isthmus_clipboard::{Pasteboard, SystemPasteboard};

fn main() {
    let board = SystemPasteboard;
    println!(
        "supported: {}, access: {:?}, counter: {:?}",
        SystemPasteboard::supported(),
        board.access(),
        board.change_count()
    );
}
