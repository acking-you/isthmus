//! The system clipboard, for sharing it with paired computers.
//!
//! A [`Watcher`] thread asks the system four times a second whether anything
//! was copied (macOS's `changeCount`, Windows' clipboard sequence number:
//! both a counter read without touching the contents), and only when the
//! counter moved does it read the text and hand it on. Text another
//! computer sent goes onto the clipboard through the same watcher, which
//! remembers it so that it is not sent straight back.
//!
//! macOS asks the user before an app reads the clipboard by itself (its
//! "Paste from Other Apps" privacy setting), so on a Mac the watcher reads
//! what was copied only once the user has set Isthmus to always allow
//! ([`Access`]); until then it counts the copies it could not share, and
//! [`Watcher::request_access`] raises the system's prompt once, which also
//! lists Isthmus in System Settings. Writing needs no permission.
//!
//! What a password manager copies is not shared: macOS apps mark it
//! `org.nspasteboard.ConcealedType` or `TransientType`, Windows apps put
//! `ExcludeClipboardContentFromMonitorProcessing` beside it, and both mean
//! "do not record or sync this". Text only, up to [`MAX_TEXT`] bytes.
//!
//! The decisions ([`Decider`]) are plain logic tested on values; reading and
//! writing the clipboard is the platform's ([`Pasteboard`]), and no test
//! touches the real one.

mod platform;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;

/// The most text shared at once, in UTF-8 bytes: a long log, not a file.
pub const MAX_TEXT: usize = 4 * 1024 * 1024;

/// How often the watcher looks at the counter.
pub const POLL: Duration = Duration::from_millis(250);

/// What the clipboard holds, as far as sharing goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contents {
    /// Plain text anyone may see.
    Text(String),
    /// Something marked not to be recorded or synced (a password).
    Concealed,
    /// No text, or more than [`MAX_TEXT`].
    Other,
}

/// Whether this app may read the clipboard without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// It may: Windows always, a Mac once the user chose "always allow".
    Allowed,
    /// macOS would ask the user on each read.
    Ask,
    /// The user chose "always deny" on this Mac.
    Denied,
}

/// Why the clipboard could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The system clipboard, behind a trait so the tests touch no real one.
pub trait Pasteboard: Send + Sync {
    /// A number that changes whenever anything is copied; `None` where the
    /// system has none.
    fn change_count(&self) -> Option<u64>;

    /// Whether this app may read it without asking.
    fn access(&self) -> Access;

    /// What it holds now. On a Mac without [`Access::Allowed`] this raises
    /// the system's prompt.
    fn read(&self) -> Result<Contents, Error>;

    /// Puts `text` on it, replacing what was there.
    fn write(&self, text: &str) -> Result<(), Error>;
}

/// This build's operating system's clipboard.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPasteboard;

impl SystemPasteboard {
    /// Whether this system's clipboard can be shared at all.
    pub fn supported() -> bool {
        platform::SUPPORTED
    }
}

impl Pasteboard for SystemPasteboard {
    fn change_count(&self) -> Option<u64> {
        platform::change_count()
    }

    fn access(&self) -> Access {
        platform::access()
    }

    fn read(&self) -> Result<Contents, Error> {
        platform::read()
    }

    fn write(&self, text: &str) -> Result<(), Error> {
        platform::write(text)
    }
}

/// What the watcher knows between looks: the last counter it saw, and the
/// last text another computer put here, which must not go back.
#[derive(Debug, Default)]
pub struct Decider {
    seen: Option<u64>,
    received: Option<String>,
    sent: Option<String>,
}

impl Decider {
    /// Starts from the counter as it is, so what was copied before the app
    /// started is not shared.
    pub fn new(count: Option<u64>) -> Self {
        Self {
            seen: count,
            ..Self::default()
        }
    }

    /// Whether the counter moved since the last look; remembers it.
    pub fn moved(&mut self, count: Option<u64>) -> bool {
        if count.is_none() || count == self.seen {
            return false;
        }
        self.seen = count;
        true
    }

    /// What to share of `contents`, read after the counter moved: text that
    /// is neither concealed, nor what another computer just sent, nor what
    /// was shared last.
    pub fn share(&mut self, contents: Contents) -> Option<String> {
        let Contents::Text(text) = contents else {
            return None;
        };
        if text.is_empty()
            || self.received.as_deref() == Some(text.as_str())
            || self.sent.as_deref() == Some(text.as_str())
        {
            return None;
        }
        self.received = None;
        self.sent = Some(text.clone());
        Some(text)
    }

    /// Another computer's text went onto the clipboard, and the counter
    /// read right after it: neither goes back out.
    pub fn received(&mut self, text: &str, count: Option<u64>) {
        self.received = Some(text.to_owned());
        self.sent = None;
        if count.is_some() {
            self.seen = count;
        }
    }
}

/// Watches the clipboard on a thread of its own, and puts text from other
/// computers on it.
pub struct Watcher {
    pasteboard: Arc<dyn Pasteboard>,
    decider: Arc<Mutex<Decider>>,
    /// Copies made here that were not shared for want of permission.
    blocked: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Watcher {
    /// Starts watching `pasteboard`, calling `on_copy` with each new text
    /// copied on this computer, on the watcher's thread.
    pub fn start(
        pasteboard: Arc<dyn Pasteboard>,
        on_copy: impl Fn(String) + Send + 'static,
    ) -> std::io::Result<Self> {
        let decider = Arc::new(Mutex::new(Decider::new(pasteboard.change_count())));
        let blocked = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (pasteboard, decider, blocked, stop) = (
                pasteboard.clone(),
                decider.clone(),
                blocked.clone(),
                stop.clone(),
            );
            std::thread::Builder::new()
                .name("isthmus-clipboard".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(POLL);
                        let count = pasteboard.change_count();
                        if !decider.lock().moved(count) {
                            continue;
                        }
                        if pasteboard.access() != Access::Allowed {
                            // Reading now would put a prompt in front of
                            // every copy.
                            blocked.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let contents = match pasteboard.read() {
                            Ok(contents) => contents,
                            Err(error) => {
                                tracing::debug!(%error, "the clipboard could not be read");
                                continue;
                            }
                        };
                        let shared = decider.lock().share(contents);
                        if let Some(text) = shared {
                            on_copy(text);
                        }
                    }
                })?
        };
        Ok(Self {
            pasteboard,
            decider,
            blocked,
            stop,
            thread: Mutex::new(Some(thread)),
        })
    }

    /// Puts `text`, from another computer, on this clipboard without
    /// sharing it again.
    pub fn put(&self, text: &str) -> Result<(), Error> {
        // Held across the write, so the watcher cannot read the new counter
        // before it is marked as this write's.
        let mut decider = self.decider.lock();
        self.pasteboard.write(text)?;
        decider.received(text, self.pasteboard.change_count());
        Ok(())
    }

    /// Whether this app may read the clipboard without asking.
    pub fn access(&self) -> Access {
        self.pasteboard.access()
    }

    /// How many copies were not shared for want of permission.
    pub fn blocked(&self) -> u64 {
        self.blocked.load(Ordering::Relaxed)
    }

    /// Reads the clipboard once, on a thread of its own, so that macOS
    /// shows its prompt (and lists Isthmus in System Settings, where the
    /// user can choose "always allow"). What it reads is not shared.
    pub fn request_access(&self) {
        let pasteboard = self.pasteboard.clone();
        let _ = std::thread::Builder::new()
            .name("isthmus-clipboard-ask".into())
            .spawn(move || {
                let _ = pasteboard.read();
            });
    }

    /// Stops the thread and waits for it.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.lock().take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    /// What was on the clipboard before the app started is not shared;
    /// each new copy is, once.
    #[test]
    fn only_new_copies_are_shared_once_each() {
        let mut decider = Decider::new(Some(7));
        assert!(!decider.moved(Some(7)));
        assert!(decider.moved(Some(8)));
        assert_eq!(decider.share(Contents::Text("a".into())), Some("a".into()));
        assert!(!decider.moved(Some(8)), "the same copy is not news");
        // Copying the same text again (the counter moves) shares nothing.
        assert!(decider.moved(Some(9)));
        assert_eq!(decider.share(Contents::Text("a".into())), None);
        assert!(decider.moved(Some(10)));
        assert_eq!(decider.share(Contents::Text("b".into())), Some("b".into()));
    }

    /// Text another computer sent is not sent back, even if the system
    /// moves the counter again for it; copying something else afterwards
    /// is shared, and so is copying the received text anew after that.
    #[test]
    fn received_text_does_not_echo_back() {
        let mut decider = Decider::new(Some(1));
        decider.received("from the PC", Some(2));
        assert!(!decider.moved(Some(2)));
        assert!(decider.moved(Some(3)));
        assert_eq!(decider.share(Contents::Text("from the PC".into())), None);
        assert!(decider.moved(Some(4)));
        assert_eq!(
            decider.share(Contents::Text("mine".into())),
            Some("mine".into())
        );
        assert!(decider.moved(Some(5)));
        assert_eq!(
            decider.share(Contents::Text("from the PC".into())),
            Some("from the PC".into())
        );
    }

    /// A password manager's copy, an image or nothing at all is not shared.
    #[test]
    fn concealed_and_other_contents_are_not_shared() {
        let mut decider = Decider::new(None);
        assert_eq!(decider.share(Contents::Concealed), None);
        assert_eq!(decider.share(Contents::Other), None);
        assert_eq!(decider.share(Contents::Text(String::new())), None);
    }

    /// A clipboard in memory, with a counter like the system's.
    struct FakePasteboard {
        state: Mutex<(u64, Contents)>,
        access: Mutex<Access>,
    }

    impl FakePasteboard {
        fn copy(&self, contents: Contents) {
            let mut state = self.state.lock();
            state.0 += 1;
            state.1 = contents;
        }
    }

    impl Pasteboard for FakePasteboard {
        fn change_count(&self) -> Option<u64> {
            Some(self.state.lock().0)
        }

        fn access(&self) -> Access {
            *self.access.lock()
        }

        fn read(&self) -> Result<Contents, Error> {
            Ok(self.state.lock().1.clone())
        }

        fn write(&self, text: &str) -> Result<(), Error> {
            self.copy(Contents::Text(text.to_owned()));
            Ok(())
        }
    }

    /// The thread hands on what is copied here, and a put from another
    /// computer lands on the clipboard without coming back.
    #[test]
    fn the_watcher_shares_copies_and_puts_without_echo() {
        let board = Arc::new(FakePasteboard {
            state: Mutex::new((0, Contents::Other)),
            access: Mutex::new(Access::Allowed),
        });
        let (tx, copied) = mpsc::channel();
        let watcher = Watcher::start(board.clone(), move |text| {
            let _ = tx.send(text);
        })
        .unwrap();
        board.copy(Contents::Text("hello".into()));
        assert_eq!(
            copied.recv_timeout(Duration::from_secs(5)).unwrap(),
            "hello"
        );

        watcher.put("from afar").unwrap();
        assert_eq!(board.read().unwrap(), Contents::Text("from afar".into()));
        board.copy(Contents::Text("next".into()));
        assert_eq!(
            copied.recv_timeout(Duration::from_secs(5)).unwrap(),
            "next",
            "the put was not shared back first"
        );
        watcher.stop();
    }

    /// Without permission to read, a copy is counted, not read (no prompt
    /// per copy) and not shared; once allowed, the next copy is.
    #[test]
    fn copies_without_permission_are_counted_not_read() {
        let board = Arc::new(FakePasteboard {
            state: Mutex::new((0, Contents::Other)),
            access: Mutex::new(Access::Ask),
        });
        let (tx, copied) = mpsc::channel();
        let watcher = Watcher::start(board.clone(), move |text| {
            let _ = tx.send(text);
        })
        .unwrap();
        board.copy(Contents::Text("private".into()));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while watcher.blocked() == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(POLL);
        }
        assert_eq!(watcher.blocked(), 1);
        assert!(copied.try_recv().is_err());

        *board.access.lock() = Access::Allowed;
        board.copy(Contents::Text("allowed now".into()));
        assert_eq!(
            copied.recv_timeout(Duration::from_secs(5)).unwrap(),
            "allowed now"
        );
        watcher.stop();
    }
}
