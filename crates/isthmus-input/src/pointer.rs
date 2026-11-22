//! Pointer speed and acceleration, as HID event-system properties.
//!
//! macOS 14 and later can move the pointer linearly: with
//! `HIDUseLinearScalingMouseAcceleration = 1`, `HIDMouseAcceleration` stops
//! being a curve index and becomes a plain gain, `points = counts × gain`,
//! with the mouse's DPI ignored. That is exactly Windows with "Enhance
//! pointer precision" off, where `pixels = counts × multiplier` and the
//! multiplier comes from the 1–20 speed. So the Windows mode is a gain from
//! Windows' own table. When the linear flag is not available, acceleration
//! is switched off with the value -1 (one count, one point).

use serde::{Deserialize, Serialize};

use crate::settings::{MouseProfile, POINTER_SPEED_RANGE, PointerMode};

/// The properties to write on one pointer service.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum PointerPlan {
    /// Put back what the system had (see [`crate::journal`]).
    System,
    /// `HIDUseLinearScalingMouseAcceleration = 1`, then the gain into the
    /// service's acceleration key, as 16.16 fixed point.
    Linear { gain: f64 },
    /// No linear flag on this service: acceleration -1, gain fixed at 1.
    Raw,
}

/// Windows' pointer multiplier for speed 1–20 (Control Panel's
/// `MouseSensitivity`; the 11 notches of the slider are 1, 2, 4, ..., 20).
/// Speeds outside the range are clamped.
pub fn windows_multiplier(speed: u8) -> f64 {
    let clamped = speed.clamp(*POINTER_SPEED_RANGE.start(), *POINTER_SPEED_RANGE.end());
    // The range starts at 1, so the index is in bounds; `get` keeps a bad
    // table edit from panicking on the HID thread.
    WINDOWS_MULTIPLIERS
        .get(usize::from(clamped - 1))
        .copied()
        .unwrap_or(1.0)
}

/// Windows' multipliers with "Enhance pointer precision" off, for speeds 1
/// to 20: eighths below the default (with 1/32 and 1/16 at the slow end),
/// 1.0 at 10, then quarters. Microsoft does not publish the table; this is
/// the one every measurement agrees on.
const WINDOWS_MULTIPLIERS: [f64; 20] = [
    1.0 / 32.0,
    1.0 / 16.0,
    1.0 / 8.0,
    1.0 / 4.0,
    3.0 / 8.0,
    1.0 / 2.0,
    5.0 / 8.0,
    3.0 / 4.0,
    7.0 / 8.0,
    1.0,
    1.25,
    1.5,
    1.75,
    2.0,
    2.25,
    2.5,
    2.75,
    3.0,
    3.25,
    3.5,
];

const _: () = assert!(
    WINDOWS_MULTIPLIERS.len() == *POINTER_SPEED_RANGE.end() as usize
        && *POINTER_SPEED_RANGE.start() == 1,
    "one multiplier per Windows speed, indexed from speed 1"
);

/// The plan for one pointer service. `supports_linear` is whether the
/// service publishes `HIDUseLinearScalingMouseAcceleration` and uses
/// `HIDMouseAcceleration` as its acceleration type. `display_scale` is the
/// main display's panel pixels per point ([`display_scale`]): Windows moves
/// in pixels and macOS in points, so the gain is divided by it.
pub fn plan(profile: &MouseProfile, supports_linear: bool, display_scale: f64) -> PointerPlan {
    match (profile.pointer, supports_linear) {
        (PointerMode::System, _) => PointerPlan::System,
        (PointerMode::Windows, true) => PointerPlan::Linear {
            gain: (windows_multiplier(profile.pointer_speed) / sane_scale(display_scale))
                .clamp(*GAIN_RANGE.start(), *GAIN_RANGE.end()),
        },
        (PointerMode::Windows, false) => PointerPlan::Raw,
    }
}

/// Panel pixels per point from a native pixel width and a width in points:
/// 2560 pixels shown as 2048 points is 1.25. Anything that is not a sensible
/// positive ratio reads as 1.0.
pub fn display_scale(panel_pixels: f64, points: f64) -> f64 {
    if panel_pixels.is_finite() && points.is_finite() && panel_pixels > 0.0 && points > 0.0 {
        sane_scale(panel_pixels / points)
    } else {
        1.0
    }
}

/// The scales Isthmus believes: from 0.5 (a panel shown larger than
/// native) to 4 (more pixels per point than any Mac display uses).
fn sane_scale(scale: f64) -> f64 {
    if scale.is_finite() {
        scale.clamp(0.5, 4.0)
    } else {
        1.0
    }
}

/// A gain as the 16.16 fixed-point value the HID filter reads. Never zero:
/// the filter silently replaces a zero gain with 0.1.
///
/// A gain that rounds to zero becomes the smallest positive step instead. A
/// negative value passes through with its sign: the filter reads -1.0
/// (-65536) as "acceleration off", which [`PointerPlan::Raw`] writes, and
/// turning that into a tiny positive gain would all but freeze the pointer.
/// Out-of-range values saturate at the `i32` limits.
pub fn to_fixed(gain: f64) -> i32 {
    // `as` saturates and maps NaN to 0, which the check below then lifts.
    let fixed = (gain * FIXED_ONE).round() as i32;
    if fixed == 0 { 1 } else { fixed }
}

/// 1.0 in 16.16 fixed point.
const FIXED_ONE: f64 = 65536.0;

/// The gain range Isthmus writes.
pub const GAIN_RANGE: std::ops::RangeInclusive<f64> = 0.03..=8.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole table: eighths up to the default 10 (1.0), quarters above,
    /// and the slider's 11 notches land on 1/32, 1/16, 1/4, ... 3.5.
    #[test]
    fn windows_multipliers_match_the_control_panel_table() {
        let table: Vec<f64> = (1..=20).map(windows_multiplier).collect();
        assert_eq!(
            table,
            [
                0.03125, 0.0625, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0, 1.25, 1.5, 1.75,
                2.0, 2.25, 2.5, 2.75, 3.0, 3.25, 3.5,
            ]
        );
        let notches: Vec<f64> = [1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20]
            .into_iter()
            .map(windows_multiplier)
            .collect();
        assert_eq!(
            notches,
            [
                0.03125, 0.0625, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5
            ]
        );
    }

    /// A speed below 1 or above 20 (a hand-edited settings file) is the
    /// nearest end of the scale, never a panic.
    #[test]
    fn speeds_outside_the_scale_are_clamped() {
        assert_eq!(windows_multiplier(0), windows_multiplier(1));
        assert_eq!(windows_multiplier(21), 3.5);
        assert_eq!(windows_multiplier(u8::MAX), 3.5);
    }

    /// System leaves the pointer alone whatever the service supports;
    /// Windows is the table's gain where linear scaling exists and plain
    /// unaccelerated motion where it does not.
    #[test]
    fn the_plan_follows_the_mode_and_what_the_service_supports() {
        let windows = MouseProfile {
            pointer_speed: 14,
            ..MouseProfile::WINDOWS
        };
        assert_eq!(plan(&windows, true, 1.0), PointerPlan::Linear { gain: 2.0 });
        assert_eq!(plan(&windows, false, 1.0), PointerPlan::Raw);
        assert_eq!(plan(&MouseProfile::MAC, true, 1.0), PointerPlan::System);
        assert_eq!(plan(&MouseProfile::MAC, false, 1.0), PointerPlan::System);
        assert_eq!(
            plan(&MouseProfile::WINDOWS, true, 1.0),
            PointerPlan::Linear { gain: 1.0 }
        );
    }

    /// Every gain the plan can produce lies in the range Isthmus writes.
    #[test]
    fn planned_gains_stay_in_the_written_range() {
        for speed in 0..=u8::MAX {
            let profile = MouseProfile {
                pointer_speed: speed,
                ..MouseProfile::WINDOWS
            };
            let PointerPlan::Linear { gain } = plan(&profile, true, 1.25) else {
                panic!("Windows with linear support plans a gain");
            };
            assert!(GAIN_RANGE.contains(&gain), "speed {speed}: {gain}");
        }
    }

    /// A 2560×1440 panel shown as 2048×1152 points (this author's KVM
    /// monitor) has 1.25 pixels per point, so Windows' default speed becomes
    /// a gain of 0.8 and the pointer crosses the screen as it does on the
    /// Windows PC; nonsense measurements change nothing.
    #[test]
    fn the_gain_is_divided_by_the_display_scale() {
        let scale = display_scale(2560.0, 2048.0);
        assert_eq!(scale, 1.25);
        assert_eq!(
            plan(&MouseProfile::WINDOWS, true, scale),
            PointerPlan::Linear { gain: 0.8 }
        );
        assert_eq!(display_scale(0.0, 2048.0), 1.0);
        assert_eq!(display_scale(f64::NAN, 2048.0), 1.0);
        assert_eq!(display_scale(2560.0, -1.0), 1.0);
        assert_eq!(display_scale(40_000.0, 100.0), 4.0);
    }

    /// Gains become 16.16 fixed point, rounded: 1.0 is 65536 and the
    /// slowest Windows speed is 2048.
    #[test]
    fn gains_become_rounded_sixteen_sixteen_fixed_point() {
        assert_eq!(to_fixed(1.0), 65_536);
        assert_eq!(to_fixed(0.03125), 2_048);
        assert_eq!(to_fixed(0.8), 52_429);
        assert_eq!(to_fixed(3.5), 229_376);
    }

    /// Zero is never written, since the filter would read it as 0.1; the
    /// "acceleration off" value -1.0 keeps its sign.
    #[test]
    fn a_zero_gain_is_never_written_and_minus_one_stays_acceleration_off() {
        assert_eq!(to_fixed(0.0), 1);
        assert_eq!(to_fixed(1e-9), 1);
        assert_eq!(to_fixed(-1e-9), 1);
        assert_eq!(to_fixed(f64::NAN), 1);
        assert_eq!(to_fixed(-1.0), -65_536);
    }
}
