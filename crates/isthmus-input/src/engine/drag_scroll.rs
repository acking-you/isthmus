//! Scrolling by holding the middle button and moving the mouse, as Windows
//! apps autoscroll.
//!
//! On Windows, the middle button held over a page and the mouse moved away
//! from where it went down scrolls the page toward the pointer, faster the
//! further it is. macOS apps have no such mode, so the tap makes one:
//!
//! - **Hold.** The middle button's press is held back, not sent, and the
//!   place it went down becomes the anchor.
//! - **Click or scroll.** Released before the pointer moved more than
//!   [`DRAG_THRESHOLD`] points, it was a click: the press and the release
//!   are sent then, at the anchor, so a middle click still opens a link in
//!   a new tab or closes one. Moved further while held, it scrolls: every
//!   tick sends a pixel scroll whose speed grows with the distance from the
//!   anchor, outside a small dead zone, each axis on its own. The release
//!   ends it and is swallowed.
//!
//! Windows' other half, a click that starts a mode the next click ends, is
//! left out: on a link the same click must open it, and the tap cannot see
//! what is under the pointer. The click and the scroll are told apart by
//! movement alone.
//!
//! Everything here is plain arithmetic on positions and nanoseconds; the
//! tap reads the pointer and posts what this answers.

/// `kCGMouseEventButtonNumber` of the middle button (mouse button 3).
pub const MIDDLE_BUTTON: i64 = 2;

/// How far, in points, the pointer moves from the anchor before the press is
/// a scroll rather than a click: more than a hand's tremor, less than any
/// deliberate drag.
pub const DRAG_THRESHOLD: f64 = 6.0;

/// The distance, in points, around the anchor that scrolls nothing, so the
/// page can be held still without holding the mouse perfectly still.
pub const DEAD_ZONE: f64 = 10.0;

/// The fastest scroll, in pixels per second.
pub const MAX_SPEED: f64 = 6000.0;

/// How often the tap ticks while the middle button is held, in seconds.
pub const TICK_SECONDS: f64 = 1.0 / 60.0;

/// The longest gap between ticks that counts at full length: a tick that
/// comes late (the system was busy) does not jump the page.
const MAX_TICK_NS: u64 = 50_000_000;

/// The scroll speed, in pixels per second, of a pointer `offset` points
/// from the anchor along one axis: nothing inside the dead zone, then a
/// linear part for fine control and a square part for long distances,
/// capped. The sign follows the offset.
pub fn speed(offset: f64) -> f64 {
    let beyond = offset.abs() - DEAD_ZONE;
    if beyond <= 0.0 {
        return 0.0;
    }
    let speed = (3.5 * beyond + 0.015 * beyond * beyond).min(MAX_SPEED);
    speed.copysign(offset)
}

/// A press of the middle button, held while it may still be a click.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Session {
    /// Where the button went down, in global points (y down).
    anchor: [f64; 2],
    /// Whether the pointer has moved past the threshold: a scroll.
    scrolling: bool,
    /// The last tick.
    last_ns: u64,
    /// The fractions of a pixel not sent yet, per axis.
    carry: [f64; 2],
}

/// What a middle-button release becomes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Release {
    /// Not one this engine held: send it.
    Pass,
    /// It was a click: send a press and a release at this anchor.
    Click { at: [f64; 2] },
    /// It ended a scroll: swallow it.
    Drop,
}

/// The middle-button scroll in progress, if any.
#[derive(Debug, Default)]
pub struct DragScroll {
    session: Option<Session>,
}

impl DragScroll {
    /// The middle button went down at `at`: hold it.
    pub fn press(&mut self, at: [f64; 2], now_ns: u64) {
        self.session = Some(Session {
            anchor: at,
            scrolling: false,
            last_ns: now_ns,
            carry: [0.0, 0.0],
        });
    }

    /// The middle button came up.
    pub fn release(&mut self) -> Release {
        match self.session.take() {
            None => Release::Pass,
            Some(session) if session.scrolling => Release::Drop,
            Some(session) => Release::Click { at: session.anchor },
        }
    }

    /// Whether a press is held, so the tap must tick.
    pub fn active(&self) -> bool {
        self.session.is_some()
    }

    /// Ends a session without a click: another button went down, or the tap
    /// was disabled. The release that follows passes, unmatched, which apps
    /// ignore.
    pub fn cancel(&mut self) {
        self.session = None;
    }

    /// A tick with the pointer at `pointer`: answers the whole pixels to
    /// scroll now, vertical then horizontal, in the scroll event's sense
    /// (positive scrolls toward the top or the left), or `None` while it is
    /// still a possible click or nothing is due.
    pub fn tick(&mut self, pointer: [f64; 2], now_ns: u64) -> Option<[i32; 2]> {
        let session = self.session.as_mut()?;
        let offset = [
            pointer[1] - session.anchor[1],
            pointer[0] - session.anchor[0],
        ];
        let elapsed = now_ns.saturating_sub(session.last_ns).min(MAX_TICK_NS);
        session.last_ns = now_ns;
        if !session.scrolling {
            if offset[0].hypot(offset[1]) <= DRAG_THRESHOLD {
                return None;
            }
            session.scrolling = true;
        }
        let seconds = elapsed as f64 / 1e9;
        let mut pixels = [0i32; 2];
        for axis in 0..2 {
            // The pointer below or right of the anchor shows what is below
            // or right: the page scrolls down or right, a negative delta.
            let exact = session.carry[axis] - speed(offset[axis]) * seconds;
            let whole = exact.trunc();
            session.carry[axis] = exact - whole;
            pixels[axis] = whole as i32;
        }
        (pixels != [0, 0]).then_some(pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    /// Nothing scrolls inside the dead zone; speed grows with distance on
    /// either side, keeps the sign, and stops at the cap.
    #[test]
    fn speed_grows_with_distance_outside_the_dead_zone() {
        assert_eq!(speed(0.0), 0.0);
        assert_eq!(speed(DEAD_ZONE), 0.0);
        assert_eq!(speed(-DEAD_ZONE + 1.0), 0.0);
        let near = speed(DEAD_ZONE + 20.0);
        let far = speed(DEAD_ZONE + 200.0);
        assert!(near > 0.0 && far > near * 5.0, "{near} {far}");
        assert_eq!(speed(-(DEAD_ZONE + 20.0)), -near);
        assert_eq!(speed(10_000.0), MAX_SPEED);
    }

    /// Released without moving past the threshold, the press was a click,
    /// sent at the anchor; nothing scrolled meanwhile.
    #[test]
    fn a_press_without_movement_is_a_click_at_the_anchor() {
        let mut drag = DragScroll::default();
        drag.press([100.0, 200.0], 0);
        assert!(drag.active());
        assert_eq!(drag.tick([104.0, 203.0], 16 * MS), None);
        assert_eq!(drag.release(), Release::Click { at: [100.0, 200.0] });
        assert!(!drag.active());
    }

    /// Moved below and right of the anchor, the page scrolls down and right
    /// (negative deltas), by about speed times time, and the release that
    /// ends it is swallowed.
    #[test]
    fn a_drag_scrolls_toward_the_pointer_until_release() {
        let mut drag = DragScroll::default();
        drag.press([100.0, 100.0], 0);
        let pointer = [100.0 + DEAD_ZONE + 40.0, 100.0 + DEAD_ZONE + 120.0];
        let mut total = [0i64; 2];
        for tick in 1..=60 {
            if let Some([vertical, horizontal]) = drag.tick(pointer, tick * 16 * MS) {
                total[0] += i64::from(vertical);
                total[1] += i64::from(horizontal);
            }
        }
        let seconds = 60.0 * 0.016;
        let expected_v = (speed(DEAD_ZONE + 120.0) * seconds) as i64;
        let expected_h = (speed(DEAD_ZONE + 40.0) * seconds) as i64;
        assert!(
            (total[0] + expected_v).abs() <= 1,
            "{total:?} vs -{expected_v}"
        );
        assert!(
            (total[1] + expected_h).abs() <= 1,
            "{total:?} vs -{expected_h}"
        );
        assert_eq!(drag.release(), Release::Drop);
    }

    /// Above or left of the anchor scrolls up or left; within the dead zone
    /// after the drag started, the page holds still.
    #[test]
    fn up_scrolls_up_and_the_dead_zone_holds_still() {
        let mut drag = DragScroll::default();
        drag.press([0.0, 0.0], 0);
        let up = drag.tick([0.0, -(DEAD_ZONE + 100.0)], 50 * MS).unwrap();
        assert!(up[0] > 0 && up[1] == 0, "{up:?}");
        assert_eq!(drag.tick([0.0, -DEAD_ZONE / 2.0], 100 * MS), None);
    }

    /// A late tick counts at most 50 ms, so a stall does not jump the page.
    #[test]
    fn a_late_tick_does_not_jump() {
        let mut drag = DragScroll::default();
        drag.press([0.0, 0.0], 0);
        let pointer = [0.0, DEAD_ZONE + 100.0];
        let late = drag.tick(pointer, 5_000 * MS).unwrap();
        let bound = (speed(DEAD_ZONE + 100.0) * 0.05).ceil() as i32;
        assert!(late[0].abs() <= bound, "{late:?} > {bound}");
    }

    /// A release with no press held passes, and a cancelled press neither
    /// clicks nor swallows its release.
    #[test]
    fn unmatched_or_cancelled_releases_pass() {
        let mut drag = DragScroll::default();
        assert_eq!(drag.release(), Release::Pass);
        drag.press([0.0, 0.0], 0);
        drag.cancel();
        assert_eq!(drag.release(), Release::Pass);
    }
}
