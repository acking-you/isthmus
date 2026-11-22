import 'package:flutter/widgets.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

/// The four sizes an icon may take: 16 beside a label or inline, 18 in a
/// button or menu, 20 in the rail, 24 for an empty state or a tile.
abstract final class AppIconSize {
  static const double inline = 16, button = 18, nav = 20, tile = 24;
}

/// The app's one icon family: Lucide (ISC). Named for what each glyph means
/// here, so a redraw is one line rather than a search through every page.
///
/// Weight follows size. Every constant is Lucide's 400 stroke, for 16-20 px,
/// where the 300 stroke thins to grey; the `…Large` ones are the 300 stroke,
/// for 24 px and up. They are separate constants rather than one weight
/// chosen at runtime, because a non-const [IconData] defeats the icon font's
/// tree shaking.
abstract final class AppIcons {
  // Destinations.
  static const IconData overview = LucideIcons.layoutDashboard400;
  static const IconData keyboard = LucideIcons.keyboard400;
  static const IconData mouse = LucideIcons.mouse400;
  static const IconData devices = LucideIcons.monitorSmartphone400;
  static const IconData settings = LucideIcons.settings400;

  // Devices.
  static const IconData trackpad = LucideIcons.touchpad400;
  static const IconData laptop = LucideIcons.laptop400;

  /// A KVM switch between the device and this Mac.
  static const IconData kvm = LucideIcons.arrowLeftRight400;

  // Actions.
  static const IconData applyWindows = LucideIcons.wandSparkles400;
  static const IconData restore = LucideIcons.rotateCcw400;
  static const IconData add = LucideIcons.plus400;
  static const IconData delete = LucideIcons.trash2400;
  static const IconData close = LucideIcons.x400;
  static const IconData external = LucideIcons.externalLink400;
  static const IconData power = LucideIcons.power400;

  // State.
  static const IconData check = LucideIcons.check400;
  static const IconData ok = LucideIcons.circleCheck400;
  static const IconData info = LucideIcons.info400;
  static const IconData warning = LucideIcons.triangleAlert400;
  static const IconData error = LucideIcons.circleAlert400;
  static const IconData permission = LucideIcons.shieldCheck400;
  static const IconData secureInput = LucideIcons.lockKeyhole400;

  // What the keyboard page groups shortcuts by.
  static const IconData shortcuts = LucideIcons.command400;
  static const IconData editing = LucideIcons.pencilLine400;
  static const IconData navigation = LucideIcons.textCursor400;
  static const IconData windows = LucideIcons.appWindow400;
  static const IconData startMenu = LucideIcons.search400;
  static const IconData system = LucideIcons.gauge400;
  static const IconData inputSource = LucideIcons.languages400;
  static const IconData appFixes = LucideIcons.layers400;
  static const IconData terminal = LucideIcons.squareTerminal400;
  static const IconData guard = LucideIcons.shieldAlert400;

  // What the mouse page tunes.
  static const IconData wheel = LucideIcons.arrowDownUp400;
  static const IconData pointer = LucideIcons.mousePointer2400;
  static const IconData zoom = LucideIcons.zoomIn400;

  // File sharing.
  /// A Mac peer.
  static const IconData mac = LucideIcons.laptop400;

  /// A PC peer.
  static const IconData pc = LucideIcons.monitor400;

  /// The direct cable path.
  static const IconData cable = LucideIcons.cable400;

  /// The local network path.
  static const IconData network = LucideIcons.wifi400;
  static const IconData send = LucideIcons.send400;
  static const IconData outgoing = LucideIcons.arrowUpRight400;
  static const IconData incoming = LucideIcons.arrowDownLeft400;
  static const IconData file = LucideIcons.file400;
  static const IconData folder = LucideIcons.folder400;
  static const IconData folderOpen = LucideIcons.folderOpen400;
  static const IconData drop = LucideIcons.filePlus400;
  static const IconData pair = LucideIcons.link2400;
  static const IconData unpair = LucideIcons.unlink400;
  static const IconData rename = LucideIcons.pencil400;
  static const IconData more = LucideIcons.ellipsis400;
  static const IconData searching = LucideIcons.radar400;

  // Settings.
  static const IconData language = LucideIcons.globe400;
  static const IconData loginItem = LucideIcons.rocket400;
  static const IconData awake = LucideIcons.coffee400;
  static const IconData update = LucideIcons.circleArrowUp400;
  static const IconData download = LucideIcons.download400;
  static const IconData comingNext = LucideIcons.radar400;

  // Disclosure.
  static const IconData chevronDown = LucideIcons.chevronDown400;
  static const IconData chevronUp = LucideIcons.chevronUp400;
  static const IconData chevronRight = LucideIcons.chevronRight400Dir;

  // The 300 stroke, for 24 px and up.
  static const IconData keyboardLarge = LucideIcons.keyboard300;
  static const IconData mouseLarge = LucideIcons.mouse300;
  static const IconData permissionLarge = LucideIcons.shieldCheck300;
}
