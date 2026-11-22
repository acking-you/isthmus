//! The JSON envelope every answer has, and reading C string arguments.
//!
//! `{"success": true, "data": …}`, `{"success": true, "message": …}` or
//! `{"success": false, "message": …, "code": …}`. [`respond`] holds the one
//! shape: the null-handle check, the call, a caught panic, the envelope.
//! The caller frees the returned string with [`isthmus_free_string`].

use std::any::Any;
use std::borrow::Cow;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use serde::Serialize;
use serde_json::{Value, json};

use crate::error::CtlError;
use crate::handle::IsthmusHandle;

/// What an op answers with when it succeeds.
pub(crate) enum Reply {
    /// `{"success": true, "message": …}`, for an op that only did something.
    Message(Cow<'static, str>),
    /// `{"success": true, "data": …}`, for an op that returns a value.
    Data(Value),
}

impl Reply {
    /// `value` as a data reply. A value of this library's own types always
    /// serializes; if one ever does not, that is a bug, so `INTERNAL`.
    pub(crate) fn data(value: impl Serialize) -> Result<Reply, CtlError> {
        serde_json::to_value(value)
            .map(Reply::Data)
            .map_err(|error| CtlError::internal(format!("could not encode the answer: {error}")))
    }
}

/// Runs `call` with the handle, or answers that there is none, and turns
/// what it returns into the envelope. A panic is an answer (`INTERNAL`),
/// never an unwind into Dart, which would abort the whole app mid-remap.
pub(crate) fn respond(
    handle: Option<&IsthmusHandle>,
    call: impl FnOnce(&IsthmusHandle) -> Result<Reply, CtlError>,
) -> *mut c_char {
    let Some(handle) = handle else {
        return err_ctl(&CtlError::invalid_argument("handle is null"));
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| call(handle))).unwrap_or_else(|payload| {
        tracing::error!(panic = panic_text(payload.as_ref()), "an FFI call panicked");
        Err(CtlError::internal("the call panicked"))
    });
    match outcome {
        Ok(Reply::Message(message)) => to_c_string(json!({"success": true, "message": message})),
        Ok(Reply::Data(data)) => to_c_string(json!({"success": true, "data": data})),
        Err(error) => err_ctl(&error),
    }
}

/// Every failure leaves through here, so `success: false` always arrives
/// with a code and Dart never has to match on the sentence.
pub(crate) fn err_ctl(error: &CtlError) -> *mut c_char {
    to_c_string(json!({
        "success": false,
        "message": error.to_string(),
        "code": error.code(),
    }))
}

/// A panic's message, when it has one, for the log.
pub(crate) fn panic_text(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic without a message")
}

/// `value` as an owned C string; null if it does not serialize, which Dart
/// reads as a failure of its own.
fn to_c_string(value: Value) -> *mut c_char {
    match serde_json::to_string(&value)
        .ok()
        .and_then(|text| CString::new(text).ok())
    {
        Some(text) => text.into_raw(),
        None => ptr::null_mut(),
    }
}

/// A required UTF-8 argument.
///
/// # Safety
/// `ptr` must be null or point to a NUL-terminated string that stays valid
/// for the call.
pub(crate) unsafe fn parse_c_string(ptr: *const c_char, field: &str) -> Result<String, CtlError> {
    if ptr.is_null() {
        return Err(CtlError::invalid_argument(format!("{field} is null")));
    }
    // SAFETY: non-null, and NUL-terminated and live as the caller guarantees.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| CtlError::invalid_argument(format!("{field} is not valid UTF-8")))
}

/// Frees a string this library returned: an envelope or a change payload.
///
/// # Safety
/// `s` must be null or a pointer this library returned, freed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isthmus_free_string(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: it came from `CString::into_raw` in this library and is
        // freed once, as the caller guarantees.
        unsafe { drop(CString::from_raw(s)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::handle::IsthmusHandle;
    use crate::test_support::{fake_host, take_reply};

    /// Success has nothing to classify, and must not grow a code that Dart
    /// would read as a failure.
    #[test]
    fn success_responses_carry_no_code() {
        let host = fake_host();
        let handle = IsthmusHandle::new(host);
        let message = take_reply(respond(Some(&handle), |_| {
            Ok(Reply::Message("done".into()))
        }));
        assert_eq!(message["success"], true);
        assert_eq!(message["message"], "done");
        assert!(message.get("code").is_none(), "{message}");
        assert!(message.get("data").is_none(), "{message}");

        let data = take_reply(respond(Some(&handle), |_| Reply::data([1, 2])));
        assert_eq!(data["success"], true);
        assert_eq!(data["data"], json!([1, 2]));
        assert!(data.get("code").is_none(), "{data}");
        assert!(data.get("message").is_none(), "{data}");
    }

    /// Every failure says what kind it is, including the ones raised at the
    /// boundary before any state is touched.
    #[test]
    fn every_failure_carries_a_code() {
        let failed = take_reply(err_ctl(&CtlError::new(ErrorCode::Io, "disk full")));
        assert_eq!(failed["success"], false);
        assert_eq!(failed["message"], "disk full");
        assert_eq!(failed["code"], "IO");

        let null_handle = take_reply(respond(None, |_| panic!("ran without a handle")));
        assert_eq!(null_handle["success"], false);
        assert_eq!(null_handle["code"], "INVALID_ARGUMENT");

        // SAFETY: null is allowed.
        let null_arg = unsafe { parse_c_string(ptr::null(), "request") }.unwrap_err();
        assert_eq!(null_arg.code(), ErrorCode::InvalidArgument);

        let bad_utf8 = CString::new(vec![0xff_u8, 0xfe]).unwrap();
        // SAFETY: a live NUL-terminated string.
        let not_text = unsafe { parse_c_string(bad_utf8.as_ptr(), "request") }.unwrap_err();
        assert_eq!(not_text.code(), ErrorCode::InvalidArgument);
    }

    /// A panic inside a call comes back as an `INTERNAL` answer instead of
    /// unwinding into C, which would abort the app.
    #[test]
    fn a_panic_is_an_internal_failure() {
        let host = fake_host();
        let handle = IsthmusHandle::new(host);
        let reply = take_reply(respond(Some(&handle), |_| panic!("boom")));
        assert_eq!(reply["success"], false);
        assert_eq!(reply["code"], "INTERNAL");
    }
}
