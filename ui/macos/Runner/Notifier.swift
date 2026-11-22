import Cocoa
import FlutterMacOS
import UserNotifications

// System notifications for what happens while Isthmus waits in the menu
// bar: a clipboard copy that crossed to or from a paired device. Dart calls
// "notify" on the "isthmus/notifier" channel with an id, a title and a
// body; a notification with the id of one still shown replaces it, so a
// run of copies leaves one notification, not a pile.
//
// macOS asks the user once, the first time one is posted.

/// The channel and the notification center's delegate.
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
  static let shared = Notifier()

  /// Wires the method channel to the Flutter engine.
  static func register(with messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(
      name: "isthmus/notifier", binaryMessenger: messenger)
    UNUserNotificationCenter.current().delegate = shared
    channel.setMethodCallHandler { call, result in
      switch call.method {
      case "notify":
        let args = call.arguments as? [String: Any] ?? [:]
        shared.notify(
          id: args["id"] as? String ?? UUID().uuidString,
          title: args["title"] as? String ?? "",
          body: args["body"] as? String ?? "")
        result(nil)
      default:
        result(FlutterMethodNotImplemented)
      }
    }
  }

  /// Posts a notification now, asking for permission the first time;
  /// refused, it posts nothing.
  func notify(id: String, title: String, body: String) {
    let center = UNUserNotificationCenter.current()
    center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
      guard granted else { return }
      let content = UNMutableNotificationContent()
      content.title = title
      content.body = body
      center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil))
    }
  }

  /// Shown even while Isthmus's window is in front, as a banner.
  func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    willPresent notification: UNNotification,
    withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    completionHandler([.banner])
  }
}
