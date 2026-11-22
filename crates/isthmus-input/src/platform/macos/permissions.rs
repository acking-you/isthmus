//! Accessibility, which the event tap needs, read without ever prompting.
//!
//! Two signals, neither enough alone on macOS 26/27: `AXIsProcessTrusted`
//! answers from an in-process cache that can stick at a stale value, and
//! `IOHIDCheckAccess(PostEvent)` asks tccd (blocking IPC, so never on the
//! tap thread) and reads "unknown" both before the first request and after
//! the user deletes the app's entry. They are polled together on the
//! utility thread, and a revocation stops the tap at once: revoking
//! Accessibility under a running active tap can hang all input.

use objc2_application_services::AXIsProcessTrusted;
use std::ffi::{CStr, c_char, c_int};

use objc2_io_kit::{IOHIDAccessType, IOHIDCheckAccess, IOHIDRequestType};

use crate::status::{PermissionState, TapState};

/// What tccd says about posting events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostAccess {
    Granted,
    Denied,
    /// Never asked, or the entry was removed.
    Unknown,
}

/// One reading of both signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawAccess {
    /// `AXIsProcessTrusted()`, a hint.
    pub trusted: bool,
    pub post: PostAccess,
}

/// Reads both signals. Blocking IPC to tccd; never call it on the tap
/// thread. Never prompts.
pub(crate) fn probe() -> RawAccess {
    // SAFETY: a plain query with no arguments.
    let trusted = unsafe { AXIsProcessTrusted() };
    let post = match IOHIDCheckAccess(IOHIDRequestType::PostEvent) {
        IOHIDAccessType::Granted => PostAccess::Granted,
        IOHIDAccessType::Denied => PostAccess::Denied,
        _ => PostAccess::Unknown,
    };
    RawAccess { trusted, post }
}

/// The permission state from a reading and the one before it.
///
/// Either signal saying yes is a grant: on macOS 27 tccd's post-event
/// answer stays "denied" for an app granted Accessibility by dragging it
/// into the list (observed on the author's Mac: `trusted = true`,
/// `post = Denied`, and the tap installs), so its "denied" cannot veto the
/// AX cache. The one exception: once tccd has said "granted", its going
/// back to "denied" (switched off) or "unknown" (entry removed) is a
/// revocation, even while the stale AX cache still says trusted. Neither
/// saying yes is a denial when tccd says so, else unknown.
pub(crate) fn assess(previous: Option<RawAccess>, now: RawAccess) -> PermissionState {
    let revoked =
        now.post != PostAccess::Granted && previous.is_some_and(|p| p.post == PostAccess::Granted);
    match (now.post, now.trusted) {
        _ if revoked => PermissionState::Denied,
        (PostAccess::Granted, _) | (_, true) => PermissionState::Granted,
        (PostAccess::Denied, false) => PermissionState::Denied,
        (PostAccess::Unknown, false) => PermissionState::Unknown,
    }
}

/// The reading the next poll is judged against. Anything but "granted"
/// right after "granted" is a revocation; judged against that first
/// reading, the next one would defer to the AX cache, which still says
/// trusted for a while after a revocation, and read as a new grant that
/// restarts the tap. So the "granted" reading is carried forward while
/// tccd says anything else: the revocation holds until tccd says "granted"
/// again.
pub(crate) fn carried(previous: RawAccess, reading: RawAccess) -> RawAccess {
    if reading.post != PostAccess::Granted && previous.post == PostAccess::Granted {
        previous
    } else {
        reading
    }
}

/// What a permission change asks of the tap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TapAction {
    Nothing,
    /// Granted now: install whatever is missing.
    Retry,
    /// No longer granted while a tap runs: stop it before input hangs.
    Stop,
}

pub(crate) fn on_change(
    previous: PermissionState,
    now: PermissionState,
    tap_running: bool,
) -> TapAction {
    match (
        previous == PermissionState::Granted,
        now == PermissionState::Granted,
    ) {
        (false, true) => TapAction::Retry,
        (true, false) if tap_running => TapAction::Stop,
        _ => TapAction::Nothing,
    }
}

/// How far the tap's retry after a grant has got, for judging whether a
/// relaunch is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryStage {
    /// No grant has been followed by a retry yet.
    NotYet,
    /// This poll queued a retry on the tap thread, which has not run it:
    /// the tap states still show the refusal from before the grant.
    Queued,
    /// A retry has run, so the tap states show what it achieved.
    Done,
}

/// macOS applies a new grant to a running process only sometimes: when the
/// grant reads as given, the tap is wanted, a retry has happened, and the
/// keyboard tap still is not running, only a relaunch helps. `None` while
/// the retry is only queued: the states predate it, so judging them would
/// ask for a relaunch after every grant, even one the retry applies; the
/// previous judgment stands until the next poll.
pub(crate) fn relaunch_needed(
    state: PermissionState,
    wanted: bool,
    retry: RetryStage,
    keyboard: TapState,
) -> Option<bool> {
    match retry {
        RetryStage::NotYet => Some(false),
        RetryStage::Queued => None,
        RetryStage::Done => Some(
            state == PermissionState::Granted
                && wanted
                && matches!(keyboard, TapState::NeedsPermission | TapState::Failed),
        ),
    }
}

/// Whether macOS would let this process install an active tap now, by the
/// two checks that never prompt: the AX trust cache and CoreGraphics' own
/// event-posting preflight. Either one is reason enough to try: the tap's
/// creation is the real answer, and after a grant the two do not always
/// turn true together. Asks tccd, so never on the tap thread.
pub(crate) fn preflight() -> bool {
    // SAFETY: a plain query with no arguments.
    let trusted = unsafe { AXIsProcessTrusted() };
    trusted || objc2_core_graphics::CGPreflightPostEventAccess()
}

/// tccd's change notification, `com.apple.tcc.access.changed`, which it
/// posts right after any permission is written: a grant is seen at once
/// instead of at the next poll. Checking it is a read of shared memory,
/// cheap enough for every turn of the utility loop.
pub(crate) struct TccWatch {
    token: c_int,
}

/// The notification tccd posts after every permission write.
const TCC_CHANGED: &CStr = c"com.apple.tcc.access.changed";

unsafe extern "C" {
    // libnotify, part of libSystem.
    fn notify_register_check(name: *const c_char, out_token: *mut c_int) -> u32;
    fn notify_check(token: c_int, check: *mut c_int) -> u32;
    fn notify_cancel(token: c_int) -> u32;
}

/// `NOTIFY_STATUS_OK`.
const NOTIFY_OK: u32 = 0;

impl TccWatch {
    /// Registers for the notification, or `None` when libnotify refuses
    /// (the polls then catch the grant on their own).
    pub(crate) fn new() -> Option<TccWatch> {
        let mut token: c_int = 0;
        // SAFETY: a NUL-terminated static name and a valid out pointer.
        let status = unsafe { notify_register_check(TCC_CHANGED.as_ptr(), &mut token) };
        if status != NOTIFY_OK {
            return None;
        }
        let watch = TccWatch { token };
        // The first check after registering always reports a change; take
        // it now so `changed` means "since registering".
        let _ = watch.changed();
        Some(watch)
    }

    /// Whether tccd posted a change since the last call.
    pub(crate) fn changed(&self) -> bool {
        let mut check: c_int = 0;
        // SAFETY: a token this watch registered and a valid out pointer.
        let status = unsafe { notify_check(self.token, &mut check) };
        status == NOTIFY_OK && check != 0
    }
}

impl Drop for TccWatch {
    fn drop(&mut self) {
        // SAFETY: the token this watch registered, cancelled once.
        let _ = unsafe { notify_cancel(self.token) };
    }
}

/// Whether a refused keyboard tap is worth trying again now: Isthmus wants
/// it, it is not running, and a no-prompt check says macOS may allow it.
/// A tap is only ever tried after such a check, since creating one can
/// raise the system prompt on macOS 27.
pub(crate) fn should_retry(wanted: bool, keyboard: TapState, may_be_allowed: bool) -> bool {
    wanted && may_be_allowed && matches!(keyboard, TapState::NeedsPermission | TapState::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(trusted: bool, post: PostAccess) -> RawAccess {
        RawAccess { trusted, post }
    }

    /// Either signal saying yes is a grant (tccd's "denied" cannot veto the
    /// AX cache), but the stale AX cache cannot keep a removed entry
    /// granted.
    #[test]
    fn tccd_wins_and_a_removed_entry_reads_as_revoked() {
        assert_eq!(
            assess(None, raw(false, PostAccess::Granted)),
            PermissionState::Granted
        );
        // Granted by drag on macOS 27: tccd keeps saying "denied" for
        // posting while the AX cache says trusted, and the tap installs.
        assert_eq!(
            assess(None, raw(true, PostAccess::Denied)),
            PermissionState::Granted
        );
        assert_eq!(
            assess(None, raw(false, PostAccess::Denied)),
            PermissionState::Denied
        );
        assert_eq!(
            assess(None, raw(false, PostAccess::Unknown)),
            PermissionState::Unknown
        );
        assert_eq!(
            assess(None, raw(true, PostAccess::Unknown)),
            PermissionState::Granted
        );
        let before = Some(raw(true, PostAccess::Granted));
        assert_eq!(
            assess(before, raw(true, PostAccess::Unknown)),
            PermissionState::Denied
        );
    }

    /// A revoked grant (entry removed, or switched off) stays revoked for as
    /// long as tccd says anything but "granted", however long the stale AX
    /// cache keeps saying trusted; tccd saying "granted" again ends it.
    #[test]
    fn a_removed_entry_stays_revoked_until_tccd_answers_again() {
        let mut access = raw(true, PostAccess::Granted);
        for _ in 0..3 {
            let reading = raw(true, PostAccess::Unknown);
            assert_eq!(assess(Some(access), reading), PermissionState::Denied);
            access = carried(access, reading);
        }
        let granted = raw(true, PostAccess::Granted);
        assert_eq!(assess(Some(access), granted), PermissionState::Granted);
        assert_eq!(carried(access, granted), granted);
        let denied = raw(true, PostAccess::Denied);
        assert_eq!(assess(Some(access), denied), PermissionState::Denied);
        assert_eq!(carried(access, denied), access);
        // Switched off after a grant tccd confirmed: revoked, and still so
        // at the next poll despite the AX cache.
        let mut access = raw(true, PostAccess::Granted);
        for _ in 0..3 {
            assert_eq!(assess(Some(access), denied), PermissionState::Denied);
            access = carried(access, denied);
        }
        // Never granted: an "unknown" is judged as itself, so the AX hint
        // still fills in for a tccd that has not been asked.
        let unknown = raw(true, PostAccess::Unknown);
        assert_eq!(carried(raw(false, PostAccess::Unknown), unknown), unknown);
    }

    /// The author's Mac after a drag-and-switch grant: tccd never says
    /// "granted" for posting, so the AX cache alone carries the grant, and
    /// its turning false is the revocation that stops the running tap.
    #[test]
    fn a_grant_tccd_never_confirms_follows_the_ax_cache() {
        let before = raw(false, PostAccess::Denied);
        let granted = raw(true, PostAccess::Denied);
        assert_eq!(assess(Some(before), granted), PermissionState::Granted);
        assert_eq!(carried(before, granted), granted);
        let revoked = raw(false, PostAccess::Denied);
        let now = assess(Some(granted), revoked);
        assert_eq!(now, PermissionState::Denied);
        assert_eq!(
            on_change(PermissionState::Granted, now, true),
            TapAction::Stop
        );
    }

    /// A grant retries the tap; losing it stops a running tap at once and
    /// does nothing when no tap runs.
    #[test]
    fn granting_retries_and_revoking_stops_a_running_tap() {
        use PermissionState::{Denied, Granted, Unknown};
        assert_eq!(on_change(Unknown, Granted, false), TapAction::Retry);
        assert_eq!(on_change(Granted, Denied, true), TapAction::Stop);
        assert_eq!(on_change(Granted, Unknown, false), TapAction::Nothing);
        assert_eq!(on_change(Denied, Unknown, true), TapAction::Nothing);
    }

    /// A refused keyboard tap is tried again only when it is wanted and a
    /// no-prompt check says it may now be allowed; a running tap, or one
    /// Isthmus does not want, is left alone.
    #[test]
    fn a_refused_tap_is_retried_only_when_wanted_and_maybe_allowed() {
        assert!(should_retry(true, TapState::NeedsPermission, true));
        assert!(should_retry(true, TapState::Failed, true));
        assert!(!should_retry(true, TapState::NeedsPermission, false));
        assert!(!should_retry(false, TapState::NeedsPermission, true));
        assert!(!should_retry(true, TapState::Running, true));
        assert!(!should_retry(true, TapState::Off, true));
    }

    /// The change watch registers with libnotify and reports nothing until
    /// tccd posts (registering itself is not a change). Read-only.
    #[test]
    fn the_tcc_watch_starts_quiet() {
        let watch = TccWatch::new().expect("libnotify registers a check token");
        assert!(!watch.changed());
    }

    /// A relaunch is asked for only after a retry with the grant in place
    /// and the keyboard tap still refused.
    #[test]
    fn a_relaunch_is_needed_only_when_a_granted_retry_still_fails() {
        use RetryStage::{Done, NotYet};
        let granted = PermissionState::Granted;
        let refused = TapState::NeedsPermission;
        assert_eq!(relaunch_needed(granted, true, Done, refused), Some(true));
        assert_eq!(relaunch_needed(granted, true, NotYet, refused), Some(false));
        assert_eq!(
            relaunch_needed(granted, true, Done, TapState::Running),
            Some(false)
        );
        assert_eq!(
            relaunch_needed(granted, false, Done, TapState::Failed),
            Some(false)
        );
        assert_eq!(
            relaunch_needed(PermissionState::Denied, true, Done, TapState::Failed),
            Some(false)
        );
    }

    /// The poll that queues a retry does not judge a relaunch from the tap
    /// states it read before the retry ran, which still say refused; the
    /// next poll, with the retry's outcome in, does.
    #[test]
    fn a_queued_retry_leaves_the_relaunch_judgment_to_the_next_poll() {
        let granted = PermissionState::Granted;
        let refused = TapState::NeedsPermission;
        assert_eq!(
            relaunch_needed(granted, true, RetryStage::Queued, refused),
            None
        );
        assert_eq!(
            relaunch_needed(granted, true, RetryStage::Done, TapState::Running),
            Some(false)
        );
    }
}
