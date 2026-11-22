//! macOS: the general pasteboard. `changeCount` moves with every copy, and
//! `NSPasteboard` may be used from any thread.

use objc2::runtime::NSObjectProtocol;
use objc2::sel;
use objc2_app_kit::{NSPasteboard, NSPasteboardAccessBehavior, NSPasteboardTypeString};
use objc2_foundation::NSString;

use crate::{Access, Contents, Error, MAX_TEXT};

pub(crate) const SUPPORTED: bool = true;

/// The markers password managers and other careful apps put beside what
/// must not be recorded or synced (nspasteboard.org).
const DO_NOT_SHARE: [&str; 2] = [
    "org.nspasteboard.ConcealedType",
    "org.nspasteboard.TransientType",
];

pub(crate) fn change_count() -> Option<u64> {
    let count = NSPasteboard::generalPasteboard().changeCount();
    u64::try_from(count).ok()
}

/// The user's "Paste from Other Apps" choice for this app. Reading it
/// raises no prompt. `Default` means the app has never asked, which macOS
/// treats as asking. A macOS before the setting (before 15.4) has no such
/// prompt, and no such method to call.
pub(crate) fn access() -> Access {
    let board = NSPasteboard::generalPasteboard();
    if !board.respondsToSelector(sel!(accessBehavior)) {
        return Access::Allowed;
    }
    match board.accessBehavior() {
        NSPasteboardAccessBehavior::AlwaysAllow => Access::Allowed,
        NSPasteboardAccessBehavior::AlwaysDeny => Access::Denied,
        _ => Access::Ask,
    }
}

pub(crate) fn read() -> Result<Contents, Error> {
    let board = NSPasteboard::generalPasteboard();
    if let Some(types) = board.types() {
        let concealed = (0..types.count()).any(|index| {
            let kind = types.objectAtIndex(index).to_string();
            DO_NOT_SHARE.contains(&kind.as_str())
        });
        if concealed {
            return Ok(Contents::Concealed);
        }
    }
    // SAFETY: a static NSString the framework provides.
    let string_type = unsafe { NSPasteboardTypeString };
    match board.stringForType(string_type) {
        Some(text) if text.len() <= MAX_TEXT => Ok(Contents::Text(text.to_string())),
        Some(_) | None => Ok(Contents::Other),
    }
}

pub(crate) fn write(text: &str) -> Result<(), Error> {
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    // SAFETY: a static NSString the framework provides.
    let string_type = unsafe { NSPasteboardTypeString };
    if board.setString_forType(&NSString::from_str(text), string_type) {
        Ok(())
    } else {
        Err(Error("macOS did not take the text".into()))
    }
}
