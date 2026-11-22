import Cocoa
import FlutterMacOS
import window_manager

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)
    // The drag-to-authorize card shown with System Settings' Accessibility
    // pane (PermissionHelper.swift).
    PermissionHelper.register(with: flutterViewController.engine.binaryMessenger)

    super.awakeFromNib()
  }

  /// Isthmus lives in the menu bar: the window starts hidden, and Dart shows
  /// it on first run, when a permission is missing, or from the menu.
  override public func order(_ place: NSWindow.OrderingMode, relativeTo otherWin: Int) {
    super.order(place, relativeTo: otherWin)
    hiddenWindowAtLaunch()
  }
}
