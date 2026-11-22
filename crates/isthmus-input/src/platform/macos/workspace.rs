//! The frontmost app, wake and session switches, from `NSWorkspace`'s own
//! notification center on the AppKit main thread.
//!
//! The observers are registered with `exec_async` on the main queue, never
//! `exec_sync`: the caller may be a thread the main thread is waiting on
//! (Dart calls in on a worker), and a synchronous hop would deadlock. Their
//! tokens live in a main-thread-local list and are removed on the main
//! thread too. Each observer only reads plain values and hands them to the
//! service's sink, which must not block the main thread.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification, NSWorkspaceDidWakeNotification,
    NSWorkspaceSessionDidBecomeActiveNotification, NSWorkspaceSessionDidResignActiveNotification,
};
use objc2_foundation::{NSNotification, NSNotificationName};
use tracing::debug;

/// A running app as the observers report it, before classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunningApp {
    pub bundle_id: String,
    pub name: String,
    pub pid: i32,
}

/// What the observers report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceEvent {
    /// An app came to the front (also sent once at registration, for the
    /// app in front then). An app without a bundle id has an empty one.
    Activated(RunningApp),
    DidWake,
    /// Another user's session took the console: HID state is machine-wide,
    /// so everything goes back while it is away.
    SessionResigned,
    SessionActivated,
}

/// Receives workspace events on the main thread. It must not block.
pub(crate) type WorkspaceSink = Arc<dyn Fn(WorkspaceEvent) + Send + Sync>;

type Token = Retained<ProtocolObject<dyn NSObjectProtocol>>;

thread_local! {
    /// The observer tokens, only ever touched on the main thread.
    static TOKENS: RefCell<Vec<Token>> = const { RefCell::new(Vec::new()) };
}

/// The installed observers; dropping this does not remove them, [`Self::remove`] does.
pub(crate) struct Observers {
    removed: Arc<AtomicBool>,
}

impl Observers {
    /// Registers the observers on the main thread, asynchronously, and
    /// reports the app in front right after.
    pub(crate) fn install(sink: WorkspaceSink) -> Observers {
        let removed = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&removed);
        DispatchQueue::main().exec_async(move || {
            if !flag.load(Ordering::Acquire) {
                install_on_main(&sink);
            }
        });
        Observers { removed }
    }

    /// Removes the observers: at once when called on the main thread (the
    /// app's quit hook runs there, and the queue may not turn again),
    /// otherwise on the main queue's next turn. Registration that has not
    /// run yet is cancelled.
    pub(crate) fn remove(&self) {
        if self.removed.swap(true, Ordering::AcqRel) {
            return;
        }
        if MainThreadMarker::new().is_some() {
            remove_on_main();
        } else {
            DispatchQueue::main().exec_async(remove_on_main);
        }
    }
}

fn install_on_main(sink: &WorkspaceSink) {
    let workspace = NSWorkspace::sharedWorkspace();
    let center = workspace.notificationCenter();
    let observe = |name: &NSNotificationName, handler: Box<dyn Fn(&NSNotification)>| {
        let block = RcBlock::new(move |notification: NonNull<NSNotification>| {
            // SAFETY: the center passes a valid notification for the call.
            handler(unsafe { notification.as_ref() });
        });
        // SAFETY: a NULL queue runs the block on the posting thread, which
        // is the main thread for workspace notifications; the block only
        // reads the notification and calls the sink.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        TOKENS.with(|tokens| tokens.borrow_mut().push(token));
    };
    // SAFETY: AppKit's notification name constants, set before main runs.
    let (activated, woke, resigned, became_active) = unsafe {
        (
            NSWorkspaceDidActivateApplicationNotification,
            NSWorkspaceDidWakeNotification,
            NSWorkspaceSessionDidResignActiveNotification,
            NSWorkspaceSessionDidBecomeActiveNotification,
        )
    };
    let on = Arc::clone(sink);
    observe(
        activated,
        Box::new(move |notification| {
            if let Some(app) = activated_app(notification) {
                on(WorkspaceEvent::Activated(app));
            }
        }),
    );
    let on = Arc::clone(sink);
    observe(woke, Box::new(move |_| on(WorkspaceEvent::DidWake)));
    let on = Arc::clone(sink);
    observe(
        resigned,
        Box::new(move |_| on(WorkspaceEvent::SessionResigned)),
    );
    let on = Arc::clone(sink);
    observe(
        became_active,
        Box::new(move |_| on(WorkspaceEvent::SessionActivated)),
    );
    if let Some(front) = workspace.frontmostApplication() {
        sink(WorkspaceEvent::Activated(running_app(&front)));
    }
    debug!("workspace observers installed");
}

fn remove_on_main() {
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let tokens = TOKENS.with(|tokens| std::mem::take(&mut *tokens.borrow_mut()));
    for token in tokens {
        // SAFETY: each token came from this center's addObserver call.
        unsafe { center.removeObserver(token.as_ref()) };
    }
}

/// The app a `DidActivateApplication` notification names, else the one
/// AppKit says is in front.
fn activated_app(notification: &NSNotification) -> Option<RunningApp> {
    let named = notification.userInfo().and_then(|info| {
        // SAFETY: an AppKit constant key.
        let key = unsafe { NSWorkspaceApplicationKey };
        info.objectForKey(key)?
            .downcast::<NSRunningApplication>()
            .ok()
    });
    let app = named.or_else(|| NSWorkspace::sharedWorkspace().frontmostApplication())?;
    Some(running_app(&app))
}

fn running_app(app: &NSRunningApplication) -> RunningApp {
    RunningApp {
        bundle_id: app
            .bundleIdentifier()
            .map(|s| s.to_string())
            .unwrap_or_default(),
        name: app
            .localizedName()
            .map(|s| s.to_string())
            .unwrap_or_default(),
        pid: app.processIdentifier(),
    }
}

/// Running apps that have a bundle id and a Dock presence (Regular
/// activation policy): the ones a user would write an app rule for.
pub(crate) fn running_apps() -> Vec<RunningApp> {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .to_vec()
        .into_iter()
        .filter(|app| app.activationPolicy() == NSApplicationActivationPolicy::Regular)
        .map(|app| running_app(&app))
        .filter(|app| !app.bundle_id.is_empty())
        .collect()
}
