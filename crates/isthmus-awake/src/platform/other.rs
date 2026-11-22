//! Any other system: no power request, so the switch is not offered.

use crate::Held;

pub(crate) const SUPPORTED: bool = false;

/// Never constructed.
pub(crate) enum Never {}

impl Held for Never {}

pub(crate) fn hold(_reason: &str) -> Result<Never, String> {
    Err("not available on this system".into())
}
