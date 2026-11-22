import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  /// Asks an already running Isthmus to show its window.
  static let showWindowNotification = Notification.Name("dev.acking.isthmus.show-window")

  private var sigterm: DispatchSourceSignal?

  override func applicationWillFinishLaunching(_ notification: Notification) {
    // Isthmus changes machine-wide HID state, so only one copy may run. A
    // second one hands over to the first and leaves at once, with exit(0)
    // rather than terminate: terminating would run the shutdown below and
    // undo the first copy's changes.
    let mine = ProcessInfo.processInfo.processIdentifier
    if let bundleID = Bundle.main.bundleIdentifier {
      let others = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID)
        .filter { $0.processIdentifier != mine }
      if let other = others.first {
        DistributedNotificationCenter.default().postNotificationName(
          Self.showWindowNotification, object: bundleID, userInfo: nil,
          deliverImmediately: true)
        other.activate()
        exit(0)
      }
    }
    DistributedNotificationCenter.default().addObserver(
      self, selector: #selector(showWindowRequested(_:)),
      name: Self.showWindowNotification, object: Bundle.main.bundleIdentifier)

    // The default SIGTERM ends the process without applicationWillTerminate,
    // which would leave the keyboard remapped. Route it through AppKit.
    signal(SIGTERM, SIG_IGN)
    let source = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
    source.setEventHandler { NSApp.terminate(nil) }
    source.resume()
    sigterm = source

    super.applicationWillFinishLaunching(notification)
  }

  /// A quit that logout, restart or shutdown sends goes ahead at once.
  ///
  /// FlutterAppDelegate first answers every quit with `.terminateCancel` and
  /// asks Dart, then terminates again once Dart agrees. loginwindow takes
  /// that cancel as Isthmus refusing, aborts the whole logout or restart and
  /// names Isthmus; Dart's later yes does not resume it. Such a quit event
  /// carries a reason (`kAEQuitReason`), and only it skips Dart: Dart's exit
  /// handler only lets go of the tray and its library handle and always
  /// agrees, and Rust's restore runs in `applicationWillTerminate` either
  /// way. Every other quit (Cmd-Q, the menu bar's Quit, a routed SIGTERM)
  /// still asks Dart first.
  override func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    if Self.systemIsEndingSession() {
      return .terminateNow
    }
    return super.applicationShouldTerminate(sender)
  }

  /// Every quit that AppKit sees ends here: Cmd-Q, the menu bar's Quit,
  /// logout and a routed SIGTERM. Rust puts back every HID property it
  /// changed and removes its event tap before the process goes.
  ///
  /// No call to super: FlutterAppDelegate does not implement this method,
  /// so `super` would raise "unrecognized selector". AppKit swallows that
  /// exception but stops delivering the notification, and the observers
  /// registered after this delegate, the Flutter engine's own shutdown among
  /// them, would never run.
  override func applicationWillTerminate(_ notification: Notification) {
    Self.shutdownRust()
  }

  /// Closing hides to the menu bar, so the app has to outlive its window.
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return false
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }

  /// Bring the window back when the app is opened again while hidden.
  override func applicationShouldHandleReopen(
    _ sender: NSApplication,
    hasVisibleWindows flag: Bool
  ) -> Bool {
    if !flag {
      showMainWindow()
    }
    return true
  }

  @objc private func showWindowRequested(_ notification: Notification) {
    showMainWindow()
  }

  private func showMainWindow() {
    NSApp.setActivationPolicy(.regular)
    for window in NSApp.windows {
      window.setIsVisible(true)
      window.makeKeyAndOrderFront(self)
    }
    NSApp.activate(ignoringOtherApps: true)
  }

  /// Whether the quit being handled is loginwindow's for a logout, restart
  /// or shutdown: a `quit` Apple event with a reason. AERegistry.h calls the
  /// reason a parameter while it is commonly read as an attribute, so both
  /// are checked; a quit without one (the Dock, a script) asks Dart.
  private static func systemIsEndingSession() -> Bool {
    guard let event = NSAppleEventManager.shared().currentAppleEvent,
      event.eventClass == AEEventClass(kCoreEventClass),
      event.eventID == AEEventID(kAEQuitApplication)
    else {
      return false
    }
    let keyword = AEKeyword(kAEQuitReason)
    guard
      let reason = event.attributeDescriptor(forKeyword: keyword)
        ?? event.paramDescriptor(forKeyword: keyword)
    else {
      return false
    }
    return reason.typeCodeValue != 0
  }

  /// Calls the library's shutdown through the copy already loaded, found by
  /// name so a build without the library still launches.
  private static func shutdownRust() {
    typealias Shutdown = @convention(c) () -> Void
    guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "isthmus_shutdown") else {
      return
    }
    unsafeBitCast(symbol, to: Shutdown.self)()
  }
}
