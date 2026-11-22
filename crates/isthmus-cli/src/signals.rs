//! SIGINT, SIGTERM and SIGHUP, turned into an ordinary blocking wait.
//!
//! `run` must put the keyboard and mouse back before the process exits,
//! and all three signals end the process at once by default. SIGHUP is the
//! one a user sends without meaning to: closing the terminal tab, quitting
//! the terminal or losing the ssh connection hangs up the tty, and the
//! foreground job gets SIGHUP. It is routed like the other two, except when
//! the process started with it ignored: `nohup isthmus run &` asks to
//! outlive the terminal, and taking SIGHUP back would end that run when the
//! terminal closes.
//!
//! A signal handler may do almost nothing safely, so this one only writes
//! the signal's number into a pipe (`write` is async-signal-safe), and a
//! thread blocked on the pipe's other end does the real work without
//! restrictions. Unlike masking the signals and calling `sigwait`, a
//! self-pipe does not care which thread the kernel delivers to: AppKit and
//! GCD start threads of their own that a mask set in `main` would not
//! reach.

/// Delivers SIGINT, SIGTERM and SIGHUP to [`Signals::wait`] instead of
/// ending the process.
pub(crate) struct Signals {
    #[cfg(unix)]
    read: std::io::PipeReader,
}

#[cfg(unix)]
mod unix {
    use std::ffi::c_int;
    use std::io::{self, Read};
    use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd};
    use std::sync::atomic::{AtomicI32, Ordering};

    use super::Signals;

    /// The pipe's write end, for the handler; -1 until installed. Never
    /// closed once set: a handler may run at any moment until exit.
    static WRITE_END: AtomicI32 = AtomicI32::new(-1);

    /// The handler: one byte into the pipe, errno left as it was.
    extern "C" fn on_signal(signal: c_int) {
        let fd = WRITE_END.load(Ordering::Relaxed);
        if fd < 0 {
            return;
        }
        let byte = u8::try_from(signal).unwrap_or(u8::MAX);
        // SAFETY: reading errno's thread-local slot is async-signal-safe.
        let saved = unsafe { *errno_location() };
        // SAFETY: `write` is async-signal-safe; `fd` is the pipe's write end,
        // open for the rest of the process and non-blocking, so a full pipe
        // drops the byte instead of hanging the handler; `byte` outlives the
        // call. A dropped byte is harmless: one is already waiting.
        unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
        // SAFETY: as above; the interrupted code may be about to read errno.
        unsafe { *errno_location() = saved };
    }

    /// This thread's errno.
    ///
    /// # Safety
    /// The pointer is valid only on the calling thread.
    #[cfg(target_vendor = "apple")]
    unsafe fn errno_location() -> *mut c_int {
        // SAFETY: always valid for the calling thread.
        unsafe { libc::__error() }
    }

    /// This thread's errno.
    ///
    /// # Safety
    /// The pointer is valid only on the calling thread.
    #[cfg(not(target_vendor = "apple"))]
    unsafe fn errno_location() -> *mut c_int {
        // SAFETY: always valid for the calling thread.
        unsafe { libc::__errno_location() }
    }

    /// The signals that stop `run` and `inspect`.
    const STOPPING: [c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

    impl Signals {
        /// Routes SIGINT, SIGTERM and SIGHUP to the pipe, SIGHUP only if the
        /// process did not start with it ignored. Call once per process.
        pub(crate) fn install() -> io::Result<Signals> {
            let signals = Signals::open_pipe()?;
            for signal in STOPPING {
                if takes_over(signal, inherited_handler(signal)?) {
                    install_handler(signal)?;
                }
            }
            Ok(signals)
        }

        /// The pipe and its write end published for the handler, without
        /// touching the signal dispositions.
        fn open_pipe() -> io::Result<Signals> {
            let (read, write) = std::io::pipe()?;
            let write = OwnedFd::from(write);
            set_nonblocking(&write)?;
            WRITE_END.store(write.into_raw_fd(), Ordering::SeqCst);
            Ok(Signals { read })
        }

        /// Blocks until one of the routed signals arrives, and says which.
        pub(crate) fn wait(&mut self) -> io::Result<c_int> {
            let mut byte = [0_u8; 1];
            // `read_exact` retries a read interrupted by the signal itself.
            self.read.read_exact(&mut byte)?;
            Ok(c_int::from(byte[0]))
        }
    }

    fn set_nonblocking(fd: &OwnedFd) -> io::Result<()> {
        let fd = fd.as_raw_fd();
        // SAFETY: `fd` is an open descriptor this process owns.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: as above; only O_NONBLOCK is added.
        if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Whether `install` replaces `inherited`, the disposition `signal` had
    /// when the process started. SIGINT and SIGTERM always: they are how a
    /// user and launchd ask Isthmus to stop. SIGHUP unless it was ignored,
    /// which is what `nohup` (or a parent that wants the run to outlive its
    /// terminal) sets up; a SIGHUP left at its default would end the process
    /// without putting anything back.
    fn takes_over(signal: c_int, inherited: libc::sighandler_t) -> bool {
        signal != libc::SIGHUP || inherited != libc::SIG_IGN
    }

    /// The handler `signal` has now, without changing it.
    fn inherited_handler(signal: c_int) -> io::Result<libc::sighandler_t> {
        // SAFETY: an all-zero `sigaction` is a valid value for the kernel to
        // overwrite.
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        // SAFETY: a null new action only reads the disposition; `current`
        // is a valid `sigaction` owned by this frame.
        if unsafe { libc::sigaction(signal, std::ptr::null(), &mut current) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(current.sa_sigaction)
    }

    /// Points `signal` at [`on_signal`].
    fn install_handler(signal: c_int) -> io::Result<()> {
        // SAFETY: an all-zero `sigaction` is a valid value (no flags, empty
        // mask, default handler); the fields that matter are set below.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = on_signal as extern "C" fn(c_int) as libc::sighandler_t;
        // Restart interrupted system calls, so the rest of the process does
        // not see EINTR because of a Ctrl-C it does not handle.
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: `sa_mask` is a valid `sigset_t` owned by `action`.
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        // SAFETY: `action` is fully initialized and its handler only does
        // async-signal-safe work; the old action is not needed.
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// What the handler writes is what the waiting thread reads, and the
        /// handler leaves errno alone. The handler is called directly: the
        /// test process's own signal dispositions stay as they are.
        #[test]
        fn a_signal_reaches_the_waiting_thread() {
            let mut signals = Signals::open_pipe().unwrap();
            // SAFETY: this thread's errno slot.
            unsafe { *errno_location() = libc::EDOM };
            on_signal(libc::SIGTERM);
            // SAFETY: as above.
            assert_eq!(unsafe { *errno_location() }, libc::EDOM);
            assert_eq!(signals.wait().unwrap(), libc::SIGTERM);
        }

        /// Closing the terminal stops `run` cleanly instead of killing it with
        /// the keyboard still remapped, while `nohup` keeps its promise that
        /// the run outlives the terminal. Ctrl-C and SIGTERM stop it either
        /// way.
        #[test]
        fn a_hangup_stops_the_run_unless_nohup_ignored_it() {
            assert!(STOPPING.contains(&libc::SIGHUP));
            assert!(takes_over(libc::SIGHUP, libc::SIG_DFL));
            assert!(!takes_over(libc::SIGHUP, libc::SIG_IGN));
            for signal in [libc::SIGINT, libc::SIGTERM] {
                assert!(takes_over(signal, libc::SIG_DFL));
                assert!(takes_over(signal, libc::SIG_IGN));
            }
            // Reading a disposition changes nothing, so the test may do it.
            assert!(inherited_handler(libc::SIGHUP).is_ok());
        }
    }
}

#[cfg(not(unix))]
impl Signals {
    /// No POSIX signals here: `run` and `inspect` are macOS commands.
    pub(crate) fn install() -> std::io::Result<Signals> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }

    pub(crate) fn wait(&mut self) -> std::io::Result<i32> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}
