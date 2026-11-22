//! The C ABI the Flutter app loads as `libisthmus_ffi.dylib`.
//!
//! Six symbols, and every new feature is an op of the one JSON export, not a
//! new symbol: `isthmus_create` and `isthmus_destroy` (a handle, a view of
//! the one service in this process), `isthmus_call_json` (every operation,
//! answered with a JSON envelope), `isthmus_free_string` (for every string
//! this library returns), `isthmus_set_change_port` (invalidation hints to
//! a Dart `ReceivePort`), and `isthmus_shutdown`, which Swift's quit hook
//! calls without a handle to put the keyboard and mouse back before the
//! process exits.

mod call;
mod callback;
mod error;
mod events;
mod handle;
mod host;
mod logging;
mod response;
#[cfg(test)]
mod test_support;

// The C surface the app looks up. `no_mangle` exports each symbol either way;
// listed here, every one is in one place.
pub use call::isthmus_call_json;
pub use events::{DartCObject, PostCObject, isthmus_set_change_port};
pub use handle::{IsthmusHandle, isthmus_create, isthmus_destroy};
pub use host::isthmus_shutdown;
pub use response::isthmus_free_string;
