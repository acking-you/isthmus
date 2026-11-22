//! Core Foundation glue for the system half: the values HID properties
//! hold, the private symbols Isthmus looks up at run time, and a run-loop
//! handle other threads may poke.
//!
//! Every conversion here is total: a value of an unexpected type reads as
//! `None`, never as a guess, because a HID property of the wrong CF type is
//! at best ignored by the event system and at worst misread inside it.

use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;

use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFRunLoop, CFString, CFType,
    kCFRunLoopDefaultMode,
};

use crate::journal::PropertyValue;

/// The source key of one `UserKeyMapping` entry.
pub(crate) const MAPPING_SRC: &str = "HIDKeyboardModifierMappingSrc";
/// The destination key of one `UserKeyMapping` entry.
pub(crate) const MAPPING_DST: &str = "HIDKeyboardModifierMappingDst";

/// A CF string for `value`.
pub(crate) fn string(value: &str) -> CFRetained<CFString> {
    CFString::from_str(value)
}

/// A number property as an integer. HID publishes flags as either CFNumber
/// or CFBoolean (`Built-In` is both, depending on the driver), so both read.
pub(crate) fn as_i64(value: &CFType) -> Option<i64> {
    if let Some(number) = value.downcast_ref::<CFNumber>() {
        return number.as_i64();
    }
    value
        .downcast_ref::<CFBoolean>()
        .map(|flag| i64::from(flag.as_bool()))
}

/// A string property, `None` for any other type.
pub(crate) fn as_string(value: &CFType) -> Option<String> {
    value.downcast_ref::<CFString>().map(ToString::to_string)
}

/// A number for a HID property: 32-bit when it fits, because the HID filters
/// read numbers such as `HIDMouseAcceleration` and `HIDFKeyMode` as SInt32,
/// and 64-bit otherwise.
pub(crate) fn number(value: i64) -> CFRetained<CFNumber> {
    match i32::try_from(value) {
        Ok(small) => CFNumber::new_i32(small),
        Err(_) => CFNumber::new_i64(value),
    }
}

/// A `UserKeyMapping` array. Usages are 64-bit (`page << 32 | usage`), so
/// each is a 64-bit CFNumber: a 32-bit one would truncate `0x7000000E3`.
pub(crate) fn key_mapping(pairs: &[(u64, u64)]) -> CFRetained<CFArray> {
    let (src_key, dst_key) = (string(MAPPING_SRC), string(MAPPING_DST));
    let entries: Vec<CFRetained<CFDictionary<CFString, CFNumber>>> = pairs
        .iter()
        .map(|&(src, dst)| {
            let (src, dst) = (
                CFNumber::new_i64(src.cast_signed()),
                CFNumber::new_i64(dst.cast_signed()),
            );
            CFDictionary::from_slices(&[&*src_key, &*dst_key], &[&*src, &*dst])
        })
        .collect();
    let typed = CFArray::from_retained_objects(&entries);
    // SAFETY: an array of dictionaries is an array of CF objects; the opaque
    // form only forgets the element type.
    unsafe { CFRetained::cast_unchecked(typed) }
}

/// Reads a `UserKeyMapping` (or `HIDKeyboardModifierMappingPairs`) array.
/// `None` when the value is not an array of source/destination dictionaries.
pub(crate) fn read_key_mapping(value: &CFType) -> Option<Vec<(u64, u64)>> {
    let array = value.downcast_ref::<CFArray>()?;
    // SAFETY: every element of a CF array that IOKit returns is a CF object;
    // reading them as `CFType` claims nothing more.
    let typed: &CFArray<CFType> = unsafe { &*std::ptr::from_ref(array).cast() };
    let (src_key, dst_key) = (string(MAPPING_SRC), string(MAPPING_DST));
    typed
        .iter()
        .map(|item| {
            let entry = item.downcast_ref::<CFDictionary>()?;
            let src = dictionary_value(entry, &src_key).and_then(|v| as_i64(&v))?;
            let dst = dictionary_value(entry, &dst_key).and_then(|v| as_i64(&v))?;
            Some((src.cast_unsigned(), dst.cast_unsigned()))
        })
        .collect()
}

/// `dictionary`'s value for the string key `key`.
pub(crate) fn dictionary_value(
    dictionary: &CFDictionary,
    key: &CFString,
) -> Option<CFRetained<CFType>> {
    // SAFETY: only read through, with a CF string key; every key and value of
    // a CF property dictionary is a CF object.
    let typed: &CFDictionary<CFString, CFType> = unsafe { &*std::ptr::from_ref(dictionary).cast() };
    typed.get(key)
}

/// A property value as the event system wants it written. `Absent` has no
/// CF form: a property cannot be unset, so callers decide what absence
/// means for their key (an empty array for a key mapping).
pub(crate) enum CfValue {
    Number(CFRetained<CFNumber>),
    Array(CFRetained<CFArray>),
}

impl CfValue {
    /// The CF form of `value`, `None` for [`PropertyValue::Absent`].
    pub(crate) fn of(value: &PropertyValue) -> Option<CfValue> {
        match value {
            PropertyValue::Absent => None,
            PropertyValue::Number(n) => Some(CfValue::Number(number(*n))),
            PropertyValue::KeyMapping(pairs) => Some(CfValue::Array(key_mapping(pairs))),
        }
    }

    pub(crate) fn as_type(&self) -> &CFType {
        match self {
            CfValue::Number(n) => n,
            CfValue::Array(a) => a,
        }
    }
}

/// Reads a property the way Isthmus writes it: a key mapping for
/// `UserKeyMapping`, a number for everything else.
pub(crate) fn property_value(key: &str, value: &CFType) -> Option<PropertyValue> {
    if key == "UserKeyMapping" {
        read_key_mapping(value).map(PropertyValue::KeyMapping)
    } else {
        as_i64(value).map(PropertyValue::Number)
    }
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// `RTLD_DEFAULT`: search every image loaded.
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

/// The address of function `name`, where this system exports it. Private
/// HID symbols are looked up this way so that Isthmus loads, and degrades,
/// on a system that dropped them.
pub(crate) fn function(name: &CStr) -> Option<NonNull<c_void>> {
    // SAFETY: a lookup; nothing is called.
    NonNull::new(unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) })
}

/// A run loop that other threads may poke. Only the three calls CF
/// documents as thread-safe are exposed: perform a block, wake, stop.
pub(crate) struct RunLoopHandle(CFRetained<CFRunLoop>);

// SAFETY: `CFRunLoopPerformBlock`, `CFRunLoopWakeUp` and `CFRunLoopStop` are
// thread-safe, retain and release are atomic, and nothing else is reachable
// through this type.
unsafe impl Send for RunLoopHandle {}
// SAFETY: as above; every exposed call takes `&self` and is thread-safe.
unsafe impl Sync for RunLoopHandle {}

impl RunLoopHandle {
    /// The calling thread's run loop.
    pub(crate) fn current() -> Option<RunLoopHandle> {
        CFRunLoop::current().map(RunLoopHandle)
    }

    /// Runs `block` on the run loop's thread at its next turn in the default
    /// mode, and wakes it so the turn comes now.
    pub(crate) fn perform(&self, block: &block2::DynBlock<dyn Fn()>) {
        // SAFETY: an extern static CF constant, set before main runs.
        let mode = unsafe { kCFRunLoopDefaultMode };
        let mode: Option<&CFType> = mode.map(|m| &**m);
        // SAFETY: thread-safe by CF's documentation; CF copies the block.
        unsafe { self.0.perform_block(mode, Some(block)) };
        self.0.wake_up();
    }

    /// Makes the run loop's `CFRunLoopRun` return.
    pub(crate) fn stop(&self) {
        self.0.stop();
        self.0.wake_up();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key mapping survives the trip through CF unchanged, including
    /// usages above 32 bits (Apple's Fn usage), which a 32-bit CFNumber
    /// would truncate.
    #[test]
    fn key_mappings_round_trip_through_cf_with_64_bit_usages() {
        let pairs = vec![
            (0x7_0000_00E0, 0x7_0000_00E7),
            (0xFF_0000_0003, 0x7_0000_00E0),
        ];
        let array = key_mapping(&pairs);
        assert_eq!(read_key_mapping(&array), Some(pairs));
        assert_eq!(read_key_mapping(&key_mapping(&[])), Some(Vec::new()));
    }

    /// A value of the wrong CF type is unreadable rather than coerced: a
    /// number is not a mapping and a string is not a number.
    #[test]
    fn values_of_the_wrong_type_read_as_nothing() {
        let n = number(1);
        assert_eq!(read_key_mapping(&n), None);
        assert_eq!(as_i64(&string("1")), None);
        assert_eq!(as_string(&n), None);
        assert_eq!(property_value("UserKeyMapping", &n), None);
    }

    /// Numbers that fit are written as SInt32, as the HID filters read them;
    /// both widths read back the same, and so do booleans.
    #[test]
    fn numbers_read_back_whatever_their_width() {
        assert_eq!(as_i64(&number(-65_536)), Some(-65_536));
        assert_eq!(as_i64(&number(1 << 40)), Some(1 << 40));
        assert_eq!(as_i64(CFBoolean::new(true)), Some(1));
        assert_eq!(
            property_value("HIDMouseAcceleration", &number(52_429)),
            Some(PropertyValue::Number(52_429))
        );
    }
}
