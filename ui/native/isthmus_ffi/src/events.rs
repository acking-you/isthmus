//! Telling the window that something changed.
//!
//! A change is an invalidation hint, not state: `{"kind": …, "seq": n}`,
//! and the window re-reads through the normal ops. A dropped or coalesced
//! hint therefore costs nothing, and the service's threads never serialize
//! state for a window that may not be listening. `seq` rises by one per hint
//! sent, so the window can drop one that arrives behind a slower re-read.
//!
//! A hint travels as a message to a Dart `ReceivePort`, posted with the
//! VM's own `Dart_PostCObject`, never as a call into an isolate's function.
//! An isolate's FFI callbacks die with it, and Flutter's hot restart kills
//! the root isolate without running Dart's `dispose`, so a callback left in
//! the slot would abort the whole process the next time a hint fired, with
//! the keyboard still remapped. `Dart_PostCObject` lives as long as the VM,
//! and posting to the port of an isolate that is gone just returns false.

use std::ffi::{CString, c_char};
use std::sync::atomic::{AtomicU64, Ordering};

use isthmus_input::Change;
use serde::Serialize;

use crate::callback::CallbackSlot;

/// `Dart_PostCObject` from `dart_native_api.h`, as Dart hands it over from
/// `NativeApi.postCObject`: callable from any thread while the VM runs, and
/// it copies the message before returning.
pub type PostCObject = unsafe extern "C" fn(port: i64, message: *mut DartCObject) -> bool;

/// `Dart_CObject_kString` in `dart_native_api.h`'s `Dart_CObject_Type`.
const DART_COBJECT_STRING: i32 = 5;

/// `ILLEGAL_PORT` in `dart_api.h`: no port, which clears the slot.
const ILLEGAL_PORT: i64 = 0;

/// `Dart_CObject` from `dart_native_api.h`, of which only the string case
/// is ever built here. Laid out as the C struct, union at full size, so
/// the VM reads exactly the bytes it expects.
#[repr(C)]
pub struct DartCObject {
    kind: i32,
    value: DartCObjectValue,
}

/// The C struct's value union: `as_string` is the one member used, and
/// `_size` stands for the largest member (`as_external_typed_data`, five
/// words) so the union has the C union's size and alignment.
#[repr(C)]
union DartCObjectValue {
    as_string: *const c_char,
    _size: [u64; 5],
}

impl DartCObject {
    /// A `kString` object pointing at `text`, which must outlive every use
    /// of the object.
    fn string(text: &CString) -> Self {
        Self {
            kind: DART_COBJECT_STRING,
            value: DartCObjectValue {
                as_string: text.as_ptr(),
            },
        }
    }
}

/// Where hints go: the VM's post function and the window's port.
#[derive(Clone, Copy)]
struct ChangePort {
    post: PostCObject,
    port: i64,
}

/// The window's port, process-wide like the service, so the port a new
/// isolate registers replaces the one a hot restart left behind.
static CHANGE_PORT: CallbackSlot<ChangePort> = CallbackSlot::empty();
/// The next hint's `seq`.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// The payload.
#[derive(Debug, Clone, Copy, Serialize)]
struct ChangeHint {
    kind: Change,
    seq: u64,
}

/// Tells the window that `change` happened. The service's change sink: it
/// never blocks, and does nothing when no port is installed.
pub(crate) fn emit(change: Change) {
    emit_to(&CHANGE_PORT, &SEQ, change);
}

/// [`emit`] on a given slot and counter, so the tests need not share the
/// process's.
fn emit_to(slot: &CallbackSlot<ChangePort>, seq: &AtomicU64, change: Change) {
    slot.with(|target| {
        let hint = ChangeHint {
            kind: change,
            seq: seq.fetch_add(1, Ordering::Relaxed),
        };
        let Ok(json) = serde_json::to_string(&hint) else {
            return;
        };
        let Ok(text) = CString::new(json) else {
            return;
        };
        let mut message = DartCObject::string(&text);
        // SAFETY: `post` is `Dart_PostCObject`, as `isthmus_set_change_port`
        // requires, callable from any thread while the VM runs, and the app
        // clears the slot before it quits. `message` is a valid `kString`
        // object whose string, `text`, outlives the call; the VM copies it
        // before returning. A port whose isolate is gone makes it return
        // false, and the hint is simply dropped.
        unsafe { (target.post)(target.port, &mut message) };
    });
}

/// Installs the port hints go to, or removes it with a null `post` or port
/// 0. Removing waits for a post in flight, so once this returns no hint
/// reaches the old port; Dart closes it right after.
///
/// # Safety
/// `post` must be null or `Dart_PostCObject` (Dart's
/// `NativeApi.postCObject`), and `port` a native port of a `ReceivePort`
/// (`sendPort.nativePort`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isthmus_set_change_port(post: Option<PostCObject>, port: i64) {
    CHANGE_PORT.store(change_port(post, port));
}

/// The slot's new value: nothing unless Dart gave both a function and a
/// real port, so either half alone clears it.
fn change_port(post: Option<PostCObject>, port: i64) -> Option<ChangePort> {
    post.filter(|_| port != ILLEGAL_PORT)
        .map(|post| ChangePort { post, port })
}

#[cfg(test)]
mod tests {
    use std::ffi::CStr;

    use parking_lot::Mutex;
    use serde_json::Value;

    use super::*;

    /// Every hint the fake VM was handed, with its port. Each test uses
    /// ports of its own, as the tests run in parallel.
    static POSTED: Mutex<Vec<(i64, Value)>> = Mutex::new(Vec::new());

    /// Ports the fake VM treats as closed, as after a hot restart.
    const DEAD_PORT: i64 = 900;

    /// What `Dart_PostCObject` does with a string message: copy it out, and
    /// refuse a closed port.
    extern "C" fn fake_post(port: i64, message: *mut DartCObject) -> bool {
        // SAFETY: `emit_to` passes a valid object for the call.
        let message = unsafe { &*message };
        assert_eq!(message.kind, DART_COBJECT_STRING);
        if port == DEAD_PORT {
            return false;
        }
        // SAFETY: a `kString` object; its string is NUL-terminated and
        // alive for the call.
        let text = unsafe { CStr::from_ptr(message.value.as_string) };
        let value = serde_json::from_str(text.to_str().unwrap()).unwrap();
        POSTED.lock().push((port, value));
        true
    }

    fn posted_to(port: i64) -> Vec<Value> {
        POSTED
            .lock()
            .iter()
            .filter(|(to, _)| *to == port)
            .map(|(_, value)| value.clone())
            .collect()
    }

    /// The struct the VM reads has `dart_native_api.h`'s layout on a
    /// 64-bit Mac: the type tag, then an 8-aligned union of five words.
    #[test]
    fn the_message_has_the_c_layout() {
        assert_eq!(std::mem::offset_of!(DartCObject, value), 8);
        assert_eq!(std::mem::size_of::<DartCObject>(), 48);
        assert_eq!(std::mem::align_of::<DartCObject>(), 8);
    }

    /// The window gets the kind it should re-read and a rising sequence
    /// number on its port, and nothing while no port is installed.
    #[test]
    fn hints_carry_the_kind_and_a_rising_sequence() {
        let slot = CallbackSlot::<ChangePort>::empty();
        let seq = AtomicU64::new(0);
        emit_to(&slot, &seq, Change::Status);
        assert_eq!(seq.load(Ordering::Relaxed), 0, "no listener, no hint");

        slot.store(Some(ChangePort {
            post: fake_post,
            port: 101,
        }));
        emit_to(&slot, &seq, Change::Settings);
        emit_to(&slot, &seq, Change::FrontApp);
        emit_to(&slot, &seq, Change::Devices);
        slot.store(None);
        emit_to(&slot, &seq, Change::Status);

        let expected: Vec<Value> = vec![
            serde_json::json!({"kind": "settings", "seq": 0}),
            serde_json::json!({"kind": "frontApp", "seq": 1}),
            serde_json::json!({"kind": "devices", "seq": 2}),
        ];
        assert_eq!(posted_to(101), expected);
    }

    /// After a hot restart the slot still names the dead isolate's port
    /// until the new isolate registers. A hint in that window is refused by
    /// the VM and dropped, not a call into freed code, and the new port gets
    /// every hint after it registers.
    #[test]
    fn a_hint_to_a_dead_isolate_is_dropped_and_the_next_port_gets_the_rest() {
        let slot = CallbackSlot::<ChangePort>::empty();
        let seq = AtomicU64::new(0);
        slot.store(Some(ChangePort {
            post: fake_post,
            port: DEAD_PORT,
        }));
        emit_to(&slot, &seq, Change::FrontApp);

        slot.store(Some(ChangePort {
            post: fake_post,
            port: 102,
        }));
        emit_to(&slot, &seq, Change::Status);
        assert_eq!(
            posted_to(102),
            vec![serde_json::json!({"kind": "status", "seq": 1})]
        );
    }

    /// Dart clears with a null function and port 0, and either alone also
    /// clears, so a half-made registration never posts to port 0 or calls
    /// through null.
    #[test]
    fn a_null_post_or_port_zero_clears_the_port() {
        assert_eq!(change_port(Some(fake_post), 103).map(|p| p.port), Some(103));
        assert!(change_port(Some(fake_post), ILLEGAL_PORT).is_none());
        assert!(change_port(None, 103).is_none());
        assert!(change_port(None, ILLEGAL_PORT).is_none());
    }
}
