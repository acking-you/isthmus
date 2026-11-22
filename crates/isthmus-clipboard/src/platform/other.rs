//! Any other system: nothing to share.

use crate::{Access, Contents, Error};

pub(crate) const SUPPORTED: bool = false;

pub(crate) fn change_count() -> Option<u64> {
    None
}

pub(crate) fn access() -> Access {
    Access::Denied
}

pub(crate) fn read() -> Result<Contents, Error> {
    Ok(Contents::Other)
}

pub(crate) fn write(_text: &str) -> Result<(), Error> {
    Err(Error("the clipboard is not shared on this system".into()))
}
