//! The error crossing the FFI boundary: a code for Dart to branch on, a
//! sentence for a person.
//!
//! There is deliberately no `From<String>` and no blanket `From` for the
//! service's error: every site picks a code, and the service's errors go
//! through [`CtlError::from_service`], whose exhaustive match makes a new
//! variant of [`isthmus_input::Error`] a compile error here until someone
//! decides its code. The cause chain goes to the log, not into the
//! envelope: the sentence is what the window shows, the chain is what a
//! developer reads.

use std::fmt;

use isthmus_awake::Error as AwakeError;
use isthmus_input::Error as ServiceError;
use isthmus_link::Error as LinkError;
use serde::Serialize;

/// What kind of failure this is. Dart treats a code it does not know as
/// `INTERNAL`, so adding one is not a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// The request was rejected before anything was attempted: not JSON, no
    /// or an unknown `op`, a missing or malformed argument, a null handle.
    InvalidArgument,
    /// The op needs the service, and it has not started in this process, or
    /// it has already shut down.
    NotRunning,
    /// Another Isthmus process holds the instance lock.
    AnotherInstance,
    /// macOS has not granted a permission the op needs.
    PermissionMissing,
    /// The op cannot work on this system.
    Unsupported,
    /// A local file could not be read or written, or holds something that
    /// cannot be read back (the settings file, the data directory).
    Io,
    /// Another pairing is in progress; try again when it ends.
    Busy,
    /// The network or the other device cannot be reached right now.
    Unavailable,
    /// Anything with no more specific answer, a caught panic included.
    Internal,
}

/// An error with a machine-readable code and a human sentence.
#[derive(Debug, Clone)]
pub struct CtlError {
    code: ErrorCode,
    message: String,
}

impl CtlError {
    /// The general constructor, for a site whose code is chosen at run time.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The code the envelope carries beside the sentence.
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// A request the library refused before doing anything.
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    /// An op that needs the service while it is not running.
    pub fn not_running(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotRunning, message)
    }

    /// A failure with no more specific answer.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    /// The service's failure as the window sees it: its code, and its own
    /// sentence without the cause chain, which is logged here instead.
    pub fn from_service(error: ServiceError) -> Self {
        let code = service_code(&error);
        tracing::warn!(?code, error = %chain(&error), "an Isthmus operation failed");
        Self::new(code, error.to_string())
    }

    /// File sharing's failure as the window sees it, like
    /// [`Self::from_service`].
    pub fn from_link(error: LinkError) -> Self {
        let code = link_code(&error);
        tracing::warn!(?code, error = %chain(&error), "a file sharing operation failed");
        Self::new(code, error.to_string())
    }

    /// A keep-awake failure, with the code [`awake_code`] gives it.
    pub fn from_awake(error: AwakeError) -> Self {
        let code = awake_code(&error);
        tracing::warn!(?code, error = %chain(&error), "a keep-awake operation failed");
        Self::new(code, error.to_string())
    }
}

/// The code for each of keep-awake's failures: its file is `IO`, a system
/// without the feature `UNSUPPORTED`, a quit `NOT_RUNNING`, and a system
/// that refused the request `INTERNAL`, with the system's reason in the
/// sentence.
pub fn awake_code(error: &AwakeError) -> ErrorCode {
    match error {
        AwakeError::ReadSettings { .. }
        | AwakeError::ParseSettings { .. }
        | AwakeError::WriteSettings { .. } => ErrorCode::Io,
        AwakeError::Unsupported => ErrorCode::Unsupported,
        AwakeError::Refused { .. } => ErrorCode::Internal,
        AwakeError::ShutDown => ErrorCode::NotRunning,
    }
}

/// The code for each of file sharing's failures. A request naming a device,
/// a transfer or a code that does not fit is `INVALID_ARGUMENT`; the network
/// not answering is `UNAVAILABLE`, which the window shows as "try again".
pub fn link_code(error: &LinkError) -> ErrorCode {
    match error {
        LinkError::ReadState { .. }
        | LinkError::ParseState { .. }
        | LinkError::WriteState { .. }
        | LinkError::Lock { .. }
        | LinkError::Source { .. }
        | LinkError::Reveal { .. } => ErrorCode::Io,
        LinkError::Key { .. } | LinkError::Runtime { .. } => ErrorCode::Internal,
        LinkError::AnotherInstance => ErrorCode::AnotherInstance,
        LinkError::Listen { .. } | LinkError::Discovery { .. } | LinkError::PeerOffline { .. } => {
            ErrorCode::Unavailable
        }
        LinkError::UnknownPeer { .. }
        | LinkError::NotPaired { .. }
        | LinkError::InvalidCode
        | LinkError::NoPairing
        | LinkError::NothingToSend
        | LinkError::TooManyFiles { .. }
        | LinkError::UnknownTransfer { .. }
        | LinkError::InvalidName { .. } => ErrorCode::InvalidArgument,
        LinkError::PairingBusy => ErrorCode::Busy,
        LinkError::ShutDown => ErrorCode::NotRunning,
    }
}

/// The code for each of the service's failures.
///
/// `ParseSettings` is `IO`, not `INTERNAL`: it means the settings file on
/// disk holds something unreadable (edited by hand, or written by a broken
/// tool), which the user can fix or remove, not a bug in this library.
/// `Encode` and `System` are `INTERNAL`: the library could not serialize its
/// own data, or macOS refused a call with a status the window cannot act on.
pub fn service_code(error: &ServiceError) -> ErrorCode {
    match error {
        ServiceError::ReadSettings { .. }
        | ServiceError::ParseSettings { .. }
        | ServiceError::WriteFile { .. }
        | ServiceError::DataDir { .. } => ErrorCode::Io,
        ServiceError::Encode { .. } | ServiceError::System { .. } => ErrorCode::Internal,
        ServiceError::AnotherInstance => ErrorCode::AnotherInstance,
        ServiceError::NotRunning => ErrorCode::NotRunning,
        ServiceError::Unsupported { .. } => ErrorCode::Unsupported,
        ServiceError::PermissionMissing { .. } => ErrorCode::PermissionMissing,
    }
}

/// `error` and every cause below it, joined with `: `, for the log.
pub(crate) fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

impl fmt::Display for CtlError {
    /// Just the sentence; the code travels beside it in the envelope, so
    /// repeating it here would double it in every toast.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CtlError {}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn io_error() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied")
    }

    fn json_error() -> serde_json::Error {
        serde_json::from_str::<serde_json::Value>("{").unwrap_err()
    }

    /// Each of the service's failures reaches Dart with the code the
    /// architecture's table gives it, so the window can branch on it.
    #[test]
    fn every_service_failure_maps_to_its_documented_code() {
        let path = PathBuf::from("/nowhere/settings.json");
        let cases = [
            (
                ServiceError::ReadSettings {
                    path: path.clone(),
                    source: io_error(),
                },
                ErrorCode::Io,
            ),
            (
                ServiceError::ParseSettings {
                    path: path.clone(),
                    source: json_error(),
                },
                ErrorCode::Io,
            ),
            (
                ServiceError::WriteFile {
                    path,
                    source: isthmus_core::Error::NoHome,
                },
                ErrorCode::Io,
            ),
            (
                ServiceError::DataDir {
                    source: isthmus_core::Error::NoHome,
                },
                ErrorCode::Io,
            ),
            (
                ServiceError::Encode {
                    what: "settings",
                    source: json_error(),
                },
                ErrorCode::Internal,
            ),
            (ServiceError::AnotherInstance, ErrorCode::AnotherInstance),
            (ServiceError::NotRunning, ErrorCode::NotRunning),
            (
                ServiceError::Unsupported { what: "pointer" },
                ErrorCode::Unsupported,
            ),
            (
                ServiceError::PermissionMissing {
                    what: "Accessibility",
                },
                ErrorCode::PermissionMissing,
            ),
            (
                ServiceError::System {
                    what: "the event tap",
                    status: -1,
                },
                ErrorCode::Internal,
            ),
        ];
        for (error, code) in cases {
            let text = error.to_string();
            let ctl = CtlError::from_service(error);
            assert_eq!(ctl.code(), code, "{text}");
            assert_eq!(ctl.to_string(), text, "the sentence is the service's own");
        }
    }

    /// File sharing's failures reach Dart with the codes the window
    /// branches on: a bad request, a busy pairing, an absent peer.
    #[test]
    fn link_failures_map_to_their_codes() {
        let cases = [
            (LinkError::InvalidCode, ErrorCode::InvalidArgument),
            (
                LinkError::NotPaired { id: "x".into() },
                ErrorCode::InvalidArgument,
            ),
            (LinkError::PairingBusy, ErrorCode::Busy),
            (
                LinkError::PeerOffline { id: "x".into() },
                ErrorCode::Unavailable,
            ),
            (LinkError::AnotherInstance, ErrorCode::AnotherInstance),
            (
                LinkError::Source {
                    path: PathBuf::from("/x"),
                    source: io_error(),
                },
                ErrorCode::Io,
            ),
            (LinkError::ShutDown, ErrorCode::NotRunning),
        ];
        for (error, code) in cases {
            let text = error.to_string();
            let ctl = CtlError::from_link(error);
            assert_eq!(ctl.code(), code, "{text}");
            assert_eq!(ctl.to_string(), text);
        }
    }

    /// The log gets the whole chain, so a developer sees why a file could
    /// not be read even though the window shows one sentence.
    #[test]
    fn the_logged_chain_includes_every_cause() {
        let error = ServiceError::ReadSettings {
            path: PathBuf::from("/x/settings.json"),
            source: io_error(),
        };
        assert_eq!(chain(&error), format!("{error}: {}", io_error()));
    }
}
