//! One export for every operation: `isthmus_call_json` takes
//! `{"op": "…", …camelCase args}` and answers with the envelope.
//!
//! A request is parsed whole into an [`Op`] before anything runs, so a
//! malformed one is `INVALID_ARGUMENT` whatever state the service is in, and
//! [`run`] matches every op with no catch-all: a new op is a new variant,
//! and the compiler points at the arm it needs. A new operation is an op
//! here (and in the Dart client, its implementation and its fake), never a
//! new export.

use std::ffi::c_char;

use isthmus_input::status::{PermissionKind, SettingsPane};
use isthmus_input::{Preset, PresetTarget, Settings};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::error::CtlError;
use crate::handle::IsthmusHandle;
use crate::host::{Host, Shutdown};
use crate::response::{Reply, parse_c_string, respond};

/// A request, parsed and validated: one variant per row of the ops table in
/// docs/architecture.md, carrying that op's arguments.
#[derive(Debug)]
enum Op {
    Start,
    Status,
    SettingsGet,
    /// Boxed: the settings dwarf every other op.
    SettingsSet(Box<Settings>),
    PresetApply {
        preset: Preset,
        target: PresetTarget,
    },
    Devices,
    RunningApps,
    PermissionRequest(PermissionKind),
    OpenSettingsPane(SettingsPane),
    Restore,
    LoginItemGet,
    LoginItemSet(bool),
    Shutdown,
}

impl Op {
    /// Reads a request's text. Anything but a JSON object with a known `op`
    /// and that op's arguments is `INVALID_ARGUMENT`.
    fn parse(text: &str) -> Result<Op, CtlError> {
        let request: Value = serde_json::from_str(text).map_err(|error| {
            CtlError::invalid_argument(format!("the request is not JSON: {error}"))
        })?;
        let name = request
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| CtlError::invalid_argument("the request has no `op`"))?;
        let op = match name {
            "start" => Op::Start,
            "status" => Op::Status,
            "settings_get" => Op::SettingsGet,
            "settings_set" => Op::SettingsSet(Box::new(arg(&request, name, "settings")?)),
            "preset_apply" => Op::PresetApply {
                preset: arg(&request, name, "preset")?,
                // `all` when left out: the one-click switch is the common case.
                target: optional_arg(&request, "target")?.unwrap_or(PresetTarget::All),
            },
            "devices" => Op::Devices,
            "running_apps" => Op::RunningApps,
            "permission_request" => Op::PermissionRequest(arg(&request, name, "kind")?),
            "open_settings_pane" => Op::OpenSettingsPane(arg(&request, name, "pane")?),
            "restore" => Op::Restore,
            "login_item_get" => Op::LoginItemGet,
            "login_item_set" => Op::LoginItemSet(arg(&request, name, "enabled")?),
            "shutdown" => Op::Shutdown,
            other => {
                return Err(CtlError::invalid_argument(format!("unknown op `{other}`")));
            }
        };
        Ok(op)
    }
}

/// A required argument of `op`.
fn arg<T: DeserializeOwned>(request: &Value, op: &str, name: &str) -> Result<T, CtlError> {
    optional_arg(request, name)?
        .ok_or_else(|| CtlError::invalid_argument(format!("`{op}` needs `{name}`")))
}

/// An argument that may be left out; present but malformed is an error.
fn optional_arg<T: DeserializeOwned>(request: &Value, name: &str) -> Result<Option<T>, CtlError> {
    match request.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => T::deserialize(value)
            .map(Some)
            .map_err(|error| CtlError::invalid_argument(format!("`{name}` is not valid: {error}"))),
    }
}

/// Runs `op` against `host`. Every op but `start` and `shutdown` needs the
/// service running.
fn run(host: &Host, op: Op) -> Result<Reply, CtlError> {
    match op {
        Op::Start => Reply::data(host.start()?.status()),
        Op::Status => Reply::data(host.running()?.status()),
        Op::SettingsGet => Reply::data(host.running()?.settings()),
        Op::SettingsSet(settings) => Reply::data(
            host.running()?
                .update_settings(*settings)
                .map_err(CtlError::from_service)?,
        ),
        Op::PresetApply { preset, target } => Reply::data(
            host.running()?
                .apply_preset(preset, target)
                .map_err(CtlError::from_service)?,
        ),
        Op::Devices => Reply::data(host.running()?.devices()),
        Op::RunningApps => Reply::data(host.running()?.running_apps()),
        Op::PermissionRequest(kind) => Reply::data(host.running()?.request_permission(kind)),
        Op::OpenSettingsPane(pane) => {
            host.running()?
                .open_settings_pane(pane)
                .map_err(CtlError::from_service)?;
            Ok(Reply::Message("System Settings is open".into()))
        }
        Op::Restore => Reply::data(host.running()?.restore().map_err(CtlError::from_service)?),
        Op::LoginItemGet => Reply::data(json!({"state": host.running()?.login_item()})),
        Op::LoginItemSet(enabled) => {
            let state = host
                .running()?
                .set_login_item(enabled)
                .map_err(CtlError::from_service)?;
            Reply::data(json!({ "state": state }))
        }
        Op::Shutdown => Ok(Reply::Message(
            match host.shutdown() {
                Shutdown::Stopped => "Isthmus stopped and put everything back",
                Shutdown::NeverStarted => "Isthmus was not running",
                Shutdown::AlreadyStopped => "Isthmus had already stopped",
            }
            .into(),
        )),
    }
}

/// Runs one operation and answers with the envelope, which the caller frees
/// with `isthmus_free_string`. Blocking: Dart calls it on a worker isolate.
///
/// # Safety
/// `handle` must be null or a live handle from `isthmus_create`; `request`
/// null or a NUL-terminated string, both valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isthmus_call_json(
    handle: *mut IsthmusHandle,
    request: *const c_char,
) -> *mut c_char {
    // SAFETY: null or live for the call, as the caller guarantees.
    respond(unsafe { handle.as_ref() }, |handle| {
        // SAFETY: null or NUL-terminated and live, as the caller guarantees.
        let text = unsafe { parse_c_string(request, "request") }?;
        run(handle.host, Op::parse(&text)?)
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use isthmus_input::settings::{KeyboardStyle, ProfileStyle};

    use super::*;
    use crate::handle::{isthmus_create, isthmus_destroy};
    use crate::test_support::{FakeService, host_with, take_reply};

    /// Calls the export as Dart does, with a handle on `handle`'s host.
    fn call(handle: *mut IsthmusHandle, request: &str) -> Value {
        let request = CString::new(request).unwrap();
        // SAFETY: a live handle and a NUL-terminated request.
        take_reply(unsafe { isthmus_call_json(handle, request.as_ptr()) })
    }

    /// A handle on a host whose service is `fake`; freed by the caller with
    /// `isthmus_destroy`.
    fn fake_handle(fake: Arc<FakeService>) -> *mut IsthmusHandle {
        let (host, _) = host_with(fake);
        Box::into_raw(Box::new(IsthmusHandle::new(host)))
    }

    fn destroy(handle: *mut IsthmusHandle) {
        // SAFETY: from `Box::into_raw`, no call in flight.
        unsafe { isthmus_destroy(handle) };
    }

    /// Malformed requests are refused before anything runs, on the
    /// process's real handle, whose service the tests never start.
    #[test]
    fn malformed_requests_are_invalid_arguments() {
        let handle = isthmus_create();
        for request in [
            "not json",
            "[1, 2]",
            r#"{"noop": 1}"#,
            r#"{"op": 5}"#,
            r#"{"op": "explode"}"#,
            r#"{"op": "preset_apply"}"#,
            r#"{"op": "preset_apply", "preset": "linux"}"#,
            r#"{"op": "preset_apply", "preset": "windows", "target": "trackpad"}"#,
            r#"{"op": "settings_set"}"#,
            r#"{"op": "settings_set", "settings": 7}"#,
            r#"{"op": "permission_request", "kind": "camera"}"#,
            r#"{"op": "open_settings_pane", "pane": "displays"}"#,
            r#"{"op": "login_item_set", "enabled": "yes"}"#,
        ] {
            let reply = call(handle, request);
            assert_eq!(reply["success"], false, "{request}");
            assert_eq!(reply["code"], "INVALID_ARGUMENT", "{request}: {reply}");
        }
        // SAFETY: a live handle; null is allowed for the request.
        let reply = take_reply(unsafe { isthmus_call_json(handle, std::ptr::null()) });
        assert_eq!(reply["code"], "INVALID_ARGUMENT");
        destroy(handle);
    }

    /// A call without a handle never reaches the service.
    #[test]
    fn a_null_handle_is_an_invalid_argument() {
        let reply = call(std::ptr::null_mut(), r#"{"op": "status"}"#);
        assert_eq!(reply["success"], false);
        assert_eq!(reply["code"], "INVALID_ARGUMENT");
    }

    /// Before `start`, every op that needs the service says it is not
    /// running, on the process's real handle, whose service the tests never
    /// start.
    #[test]
    fn ops_before_start_are_not_running() {
        let handle = isthmus_create();
        for request in [
            r#"{"op": "status"}"#,
            r#"{"op": "settings_get"}"#,
            r#"{"op": "settings_set", "settings": {}}"#,
            r#"{"op": "preset_apply", "preset": "windows", "target": "all"}"#,
            r#"{"op": "devices"}"#,
            r#"{"op": "running_apps"}"#,
            r#"{"op": "permission_request", "kind": "accessibility"}"#,
            r#"{"op": "open_settings_pane", "pane": "loginItems"}"#,
            r#"{"op": "restore"}"#,
            r#"{"op": "login_item_get"}"#,
            r#"{"op": "login_item_set", "enabled": true}"#,
        ] {
            let reply = call(handle, request);
            assert_eq!(reply["success"], false, "{request}");
            assert_eq!(reply["code"], "NOT_RUNNING", "{request}: {reply}");
        }
        destroy(handle);
    }

    /// The op table end to end against a fake: each op reaches the service
    /// with its arguments and answers with the documented shape.
    #[test]
    fn every_op_reaches_the_service_with_its_arguments() {
        let fake = Arc::new(FakeService::default());
        let handle = fake_handle(fake.clone());

        let status = call(handle, r#"{"op": "start"}"#);
        assert_eq!(status["success"], true, "{status}");
        assert_eq!(status["data"]["version"], "fake");
        assert_eq!(call(handle, r#"{"op": "start"}"#)["data"], status["data"]);
        assert_eq!(call(handle, r#"{"op": "status"}"#)["data"], status["data"]);

        let saved = call(
            handle,
            r#"{"op": "settings_set", "settings": {"enabled": true, "keyboard": {"defaultStyle": "mac"}}}"#,
        );
        assert_eq!(saved["data"]["enabled"], true, "{saved}");
        assert_eq!(
            fake.settings.lock().keyboard.default_style,
            KeyboardStyle::Mac
        );
        assert_eq!(
            call(handle, r#"{"op": "settings_get"}"#)["data"],
            saved["data"]
        );

        let preset = call(
            handle,
            r#"{"op": "preset_apply", "preset": "windows", "target": "keyboard"}"#,
        );
        assert_eq!(
            preset["data"]["keyboard"]["defaultStyle"], "windows",
            "{preset}"
        );
        call(handle, r#"{"op": "preset_apply", "preset": "mac"}"#);
        let settings = fake.settings.lock().clone();
        assert_eq!(
            settings.keyboard.style(),
            ProfileStyle::Mac,
            "no target means all"
        );
        assert_eq!(
            settings.mouse.style(),
            ProfileStyle::Mac,
            "no target means all"
        );

        assert_eq!(call(handle, r#"{"op": "devices"}"#)["data"], json!([]));
        assert_eq!(
            call(handle, r#"{"op": "running_apps"}"#)["data"][0]["bundleId"],
            "com.apple.finder"
        );
        let asked = call(
            handle,
            r#"{"op": "permission_request", "kind": "accessibility"}"#,
        );
        assert_eq!(asked["data"]["version"], "fake");
        assert_eq!(
            *fake.permissions.lock(),
            vec![PermissionKind::Accessibility]
        );

        let opened = call(
            handle,
            r#"{"op": "open_settings_pane", "pane": "loginItems"}"#,
        );
        assert_eq!(opened["success"], true);
        assert!(opened["message"].is_string() && opened.get("data").is_none());
        assert_eq!(*fake.panes.lock(), vec![SettingsPane::LoginItems]);

        assert_eq!(
            call(handle, r#"{"op": "restore"}"#)["data"]["enabled"],
            false
        );

        assert_eq!(
            call(handle, r#"{"op": "login_item_get"}"#)["data"],
            json!({"state": "disabled"})
        );
        assert_eq!(
            call(handle, r#"{"op": "login_item_set", "enabled": true}"#)["data"],
            json!({"state": "enabled"})
        );

        let stopped = call(handle, r#"{"op": "shutdown"}"#);
        assert_eq!(stopped["success"], true);
        assert!(stopped["message"].is_string());
        assert_eq!(fake.shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(call(handle, r#"{"op": "status"}"#)["code"], "NOT_RUNNING");
        destroy(handle);
    }

    /// The service's own failures keep their code across the boundary.
    #[test]
    fn a_service_failure_keeps_its_code() {
        let fake = Arc::new(FakeService::default());
        fake.fail_login_item.store(true, Ordering::SeqCst);
        let handle = fake_handle(fake);
        call(handle, r#"{"op": "start"}"#);
        let reply = call(handle, r#"{"op": "login_item_set", "enabled": true}"#);
        assert_eq!(reply["success"], false);
        assert_eq!(reply["code"], "UNSUPPORTED", "{reply}");
        destroy(handle);
    }

    /// A panic in the service is an `INTERNAL` answer through the real
    /// export, and the service keeps answering afterwards.
    #[test]
    fn a_panicking_service_is_an_internal_failure() {
        let fake = Arc::new(FakeService::default());
        let handle = fake_handle(fake.clone());
        call(handle, r#"{"op": "start"}"#);
        fake.panic_on_status.store(true, Ordering::SeqCst);
        let reply = call(handle, r#"{"op": "status"}"#);
        assert_eq!(reply["success"], false);
        assert_eq!(reply["code"], "INTERNAL");
        fake.panic_on_status.store(false, Ordering::SeqCst);
        assert_eq!(call(handle, r#"{"op": "status"}"#)["success"], true);
        destroy(handle);
    }
}
