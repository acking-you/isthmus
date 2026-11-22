#include "flutter_window.h"

#include <flutter/standard_method_codec.h>
#include <shellapi.h>
#include <strsafe.h>

#include <optional>

#include "flutter/generated_plugin_registrant.h"

namespace {

// The id tray_manager gives the app's notification-area icon on the main
// window (tray_manager_plugin.cpp, nid.uID). A balloon is shown on that
// icon rather than on a second one.
constexpr UINT kTrayIconId = 1;

std::wstring Utf16(const std::string& utf8) {
  if (utf8.empty()) return std::wstring();
  int length = ::MultiByteToWideChar(CP_UTF8, 0, utf8.data(),
                                     static_cast<int>(utf8.size()), nullptr, 0);
  std::wstring wide(length, L'\0');
  ::MultiByteToWideChar(CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()),
                        wide.data(), length);
  return wide;
}

std::string StringArgument(const flutter::EncodableMap& args, const char* key) {
  auto found = args.find(flutter::EncodableValue(key));
  if (found == args.end()) return std::string();
  const auto* value = std::get_if<std::string>(&found->second);
  return value ? *value : std::string();
}

}  // namespace

FlutterWindow::FlutterWindow(const flutter::DartProject& project)
    : project_(project) {}

FlutterWindow::~FlutterWindow() {}

bool FlutterWindow::OnCreate() {
  if (!Win32Window::OnCreate()) {
    return false;
  }

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());
  SetChildContent(flutter_controller_->view()->GetNativeWindow());

  // Notifications while the window is hidden: Dart's SystemNotice calls
  // "notify" with a title and a body. The handler runs on the platform
  // thread, as every channel call must; local_notifier, used before, called
  // Flutter from WinRT's toast threads, which froze the app once a toast
  // shown while it waited in the tray was dismissed.
  notifier_channel_ =
      std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
          flutter_controller_->engine()->messenger(), "isthmus/notifier",
          &flutter::StandardMethodCodec::GetInstance());
  notifier_channel_->SetMethodCallHandler(
      [this](const flutter::MethodCall<flutter::EncodableValue>& call,
             std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>>
                 result) {
        if (call.method_name() != "notify") {
          result->NotImplemented();
          return;
        }
        const auto* args = std::get_if<flutter::EncodableMap>(call.arguments());
        if (args == nullptr) {
          result->Error("bad-arguments", "notify takes a map");
          return;
        }
        bool shown =
            ShowBalloon(StringArgument(*args, "title"), StringArgument(*args, "body"));
        result->Success(flutter::EncodableValue(shown));
      });

  flutter_controller_->engine()->SetNextFrameCallback([&]() {
    this->Show();
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  return true;
}

bool FlutterWindow::ShowBalloon(const std::string& title,
                                const std::string& body) {
  // Windows 10 and 11 show a notification-area balloon as a notification,
  // under the app's name; NIIF_RESPECT_QUIET_TIME holds it back during the
  // first hour after sign-in and in Focus. Nothing else of the icon changes.
  NOTIFYICONDATAW nid = {};
  nid.cbSize = sizeof(nid);
  nid.hWnd = GetHandle();
  nid.uID = kTrayIconId;
  nid.uFlags = NIF_INFO;
  nid.dwInfoFlags = NIIF_NONE | NIIF_RESPECT_QUIET_TIME;
  StringCchCopyW(nid.szInfoTitle, ARRAYSIZE(nid.szInfoTitle),
                 Utf16(title).c_str());
  StringCchCopyW(nid.szInfo, ARRAYSIZE(nid.szInfo), Utf16(body).c_str());
  return ::Shell_NotifyIconW(NIM_MODIFY, &nid) != FALSE;
}

void FlutterWindow::OnDestroy() {
  notifier_channel_ = nullptr;
  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  switch (message) {
    case WM_FONTCHANGE:
      flutter_controller_->engine()->ReloadSystemFonts();
      break;
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}
