//! A slot holding what Dart installs for Rust to call: today the change
//! hints' port and the VM's post function.
//!
//! A call holds the slot's read lock until it returns, so clearing the slot
//! waits for the calls in flight: once Dart's clear returns, nothing more
//! goes to what it cleared, and Dart closes its port right after. A call
//! only posts a message to an isolate, so it never blocks on Dart while it
//! holds the lock.

use parking_lot::RwLock;

/// A nullable, thread-safe slot for a `Copy` value of type `F`, such as a C
/// function pointer and its argument.
pub(crate) struct CallbackSlot<F: Copy>(RwLock<Option<F>>);

impl<F: Copy> CallbackSlot<F> {
    pub(crate) const fn empty() -> Self {
        Self(RwLock::new(None))
    }

    /// Installs `callback`, or clears the slot with `None`, once no call
    /// through the slot is in flight.
    pub(crate) fn store(&self, callback: Option<F>) {
        *self.0.write() = callback;
    }

    /// Runs `invoke` with the installed callback, holding the slot until it
    /// returns; `None` if nothing is listening.
    pub(crate) fn with<R>(&self, invoke: impl FnOnce(F) -> R) -> Option<R> {
        let installed = self.0.read();
        (*installed).map(invoke)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use super::*;

    extern "C" fn plus_one(value: i32) -> i32 {
        value + 1
    }

    /// An empty slot calls nothing; a stored callback is called; `None`
    /// clears it again.
    #[test]
    fn a_slot_calls_only_what_is_installed() {
        let slot: CallbackSlot<extern "C" fn(i32) -> i32> = CallbackSlot::empty();
        assert_eq!(slot.with(|callback| callback(1)), None);
        slot.store(Some(plus_one));
        assert_eq!(slot.with(|callback| callback(41)), Some(42));
        slot.store(None);
        assert_eq!(slot.with(|callback| callback(1)), None);
    }

    /// What lets Dart close its port once it has cleared the slot: clearing
    /// waits for a call already in flight.
    #[test]
    fn clearing_waits_for_a_call_in_flight() {
        let slot: Arc<CallbackSlot<extern "C" fn(i32) -> i32>> = Arc::new(CallbackSlot::empty());
        slot.store(Some(plus_one));
        let (entered, call_entered) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let caller = std::thread::spawn({
            let slot = slot.clone();
            move || {
                slot.with(|callback| {
                    entered.send(()).unwrap();
                    released.recv().unwrap();
                    callback(1)
                })
            }
        });
        call_entered.recv().unwrap();
        let (cleared, slot_cleared) = mpsc::channel();
        let clearer = std::thread::spawn({
            let slot = slot.clone();
            move || {
                slot.store(None);
                cleared.send(()).unwrap();
            }
        });
        assert!(
            slot_cleared
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "the slot was cleared while a call was in flight"
        );
        release.send(()).unwrap();
        slot_cleared.recv().unwrap();
        assert_eq!(caller.join().unwrap(), Some(2));
        clearer.join().unwrap();
        assert_eq!(slot.with(|callback| callback(1)), None);
    }
}
