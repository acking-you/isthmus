//! Windows keyboard and mouse habits on a Mac.
//!
//! Two layers do the work. Per keyboard, a HID-level mapping
//! ([`hidmap`]) puts the modifiers where Windows has them and makes Ctrl
//! act as Command, which reaches every app, password fields included. One
//! event tap ([`engine`]) then translates the shortcuts a single key mapping
//! cannot express: Home/End by line, Ctrl+arrows by word, Alt+Tab, Win+D,
//! and the guards that keep Ctrl+Q and Ctrl+Tab from becoming macOS's
//! Command+Q and Command+Tab. Mice get Windows wheel direction and distance
//! in the tap, and Windows pointer speed as HID properties ([`pointer`](mod@pointer)).
//!
//! Everything above the operating system is plain data and tested on any
//! host; macOS calls live only under `platform/macos`.

pub mod context;
pub mod device;
pub mod engine;
pub mod error;
pub mod hidmap;
pub mod journal;
pub mod keys;
pub mod platform;
pub mod pointer;
pub mod service;
pub mod settings;
pub mod status;

pub use error::Error;
pub use service::{ChangeSink, InputService, ServiceOptions};
pub use settings::{Preset, PresetTarget, Settings};
pub use status::{Change, Status};
