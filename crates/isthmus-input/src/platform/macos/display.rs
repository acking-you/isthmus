//! The main display's scale between panel pixels and points (system half).
//!
//! Windows moves the pointer in physical pixels; macOS moves it in points.
//! On a panel shown "looks like" a smaller size (a 2560×1440 monitor at
//! 2048×1152 points, say), one point is several panel pixels, so the same
//! gain covers more of the screen than Windows does at the same speed. The
//! Windows pointer plan divides its gain by this factor, so a mouse shared
//! through a KVM crosses the same share of the same monitor on both systems.

use objc2_core_foundation::{CFBoolean, CFDictionary, CFString};
use objc2_core_graphics::{
    CGDisplayBounds, CGDisplayCopyAllDisplayModes, CGDisplayMode, CGMainDisplayID,
    kCGDisplayShowDuplicateLowResolutionModes,
};

/// `kDisplayModeNativeFlag` in a mode's IO flags: the panel's own
/// resolution.
const NATIVE_MODE_FLAG: u32 = 0x0200_0000;

/// Panel pixels per point on the main display, or 1.0 when macOS will not
/// say (no display, no native mode listed). Read-only; cheap enough to call
/// on every reconcile.
pub(crate) fn main_display_scale() -> f64 {
    let display = CGMainDisplayID();
    let points = CGDisplayBounds(display).size.width;
    let Some(panel) = native_pixel_width(display) else {
        return 1.0;
    };
    crate::pointer::display_scale(panel as f64, points)
}

/// The widest native mode's pixel width. Duplicate low-resolution modes are
/// asked for too, since a HiDPI setup lists the native mode only among them.
fn native_pixel_width(display: u32) -> Option<usize> {
    // SAFETY: a static CFString constant exported by CoreGraphics.
    let key = unsafe { kCGDisplayShowDuplicateLowResolutionModes };
    let options = CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[CFBoolean::new(true)]);
    // SAFETY: the options dictionary maps a CFString key to a CFBoolean, the
    // types the call documents.
    let modes = unsafe { CGDisplayCopyAllDisplayModes(display, Some(options.as_opaque())) }?;
    let count = modes.count();
    let mut widest = None;
    for index in 0..count {
        // SAFETY: `index` is below the array's count.
        let item = unsafe { modes.value_at_index(index) };
        if item.is_null() {
            continue;
        }
        // SAFETY: CGDisplayCopyAllDisplayModes returns an array of
        // CGDisplayMode, and the array keeps the element alive while we
        // borrow it here.
        let mode: &CGDisplayMode = unsafe { &*item.cast::<CGDisplayMode>() };
        if CGDisplayMode::io_flags(Some(mode)) & NATIVE_MODE_FLAG == 0 {
            continue;
        }
        let width = CGDisplayMode::pixel_width(Some(mode));
        widest = Some(widest.map_or(width, |w: usize| w.max(width)));
    }
    widest
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads this Mac's main display (read-only) and prints its scale; run
    /// by hand with `--ignored` to check the factor on a real monitor.
    #[test]
    #[ignore = "reads the real display"]
    fn prints_the_main_display_scale() {
        let scale = main_display_scale();
        println!("main display scale: {scale}");
        assert!((0.5..=4.0).contains(&scale));
    }
}
