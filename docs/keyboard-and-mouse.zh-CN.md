# 键盘与鼠标

Isthmus 让 Mac 的键盘和鼠标按 Windows 教会你双手的方式工作。本页负责这项功能：每项设置做什么、怎么做到、有哪些边界，以及改动必须保持什么。规则见 [AGENTS.md](../AGENTS.md)；代码地图见 [architecture.md](architecture.md)（英文）。本页是 [keyboard-and-mouse.md](keyboard-and-mouse.md) 的译文，两者一起改动。

## 三层

| 层 | 改变什么 | 在哪里 | 需要什么 |
| --- | --- | --- | --- |
| **HID 键位映射** | 每个物理键是哪个修饰键，按键盘 | 键盘 HID 服务上的 `UserKeyMapping`（[`hidmap.rs`](../crates/isthmus-input/src/hidmap.rs)） | 无；Windows 配置要求键盘 tap 在运行 |
| **事件 tap** | 单个按键表达不了的快捷键、滚轮和侧键 | 一个主动的 `kCGHIDEventTap`，跑在自己的线程上（[`engine/`](../crates/isthmus-input/src/engine)） | 辅助功能 |
| **指针属性** | 指针速度和加速，按鼠标 | HID 事件系统属性（[`pointer.rs`](../crates/isthmus-input/src/pointer.rs)） | 无 |

Ctrl 充当 Command 由 HID 映射完成，而不是 tap：映射后的键对每个应用都是真正的 Command，包括密码框，而 macOS 的安全事件输入（Secure Event Input）会挡住所有事件 tap。tap 再转换例外情况（Home/End、Ctrl+方向键、Alt+Tab、Win+D……），并守住那些不能变成 Command 组合的 Ctrl 组合（Ctrl+Q、Ctrl+Tab……）。

## 键盘

### 风格与布局

每个键盘有一种**风格**：Windows 或 Mac。Mac 风格的键盘从不被改动。Windows 风格的键盘得到的映射由它的**布局**（自动检测，或由用户设定）和最前面应用的**类别**决定：

| 配置（应用类别） | Windows 的 Ctrl 变成 | Win 变成 | Alt 变成 |
| --- | --- | --- | --- |
| Windows（标准应用） | 右 Command | 右 Control | 右 Option |
| NativeCtrl（终端、JetBrains、Emacs、Vim） | 右 Control | 右 Command | 左 Option |
| Positions（虚拟机、远程桌面） | 左 Control | 左 Command | 左 Option |

「Windows 的 Ctrl」是 Ctrl 位置上的键：两种布局上都是 Control。「Win」是 Windows 键位置上的键：PC 键盘上是 Command，Mac 键盘上是 Option。「Alt」是空格旁边的键：PC 键盘上是 Option，Mac 键盘上是 Command。具体数组：

| 布局 | 配置 | `UserKeyMapping`（源 → 目标，HID page 7） |
| --- | --- | --- |
| PC | Windows | E0→E7, E4→E7, E3→E4, E7→E4, E2→E6 |
| PC | NativeCtrl | E0→E4, E3→E7 |
| PC | Positions | （无） |
| Mac | Windows | E0→E7, E4→E7, E2→E4, E6→E4, E3→E6, E7→E6 |
| Mac | NativeCtrl | E0→E4, E2→E7, E6→E7, E3→E2, E7→E6 |
| Mac | Positions | E2→E3, E3→E2, E6→E7, E7→E6 |

（E0 LeftControl，E1 LeftShift，E2 LeftAlt/Option，E3 LeftGUI/Command，E4 RightControl，E6 RightAlt，E7 RightGUI；每个都写成 `0x7000000E0` 这样的 64 位 CFNumber。）

**为什么是右侧。** macOS 不告诉事件 tap 一个键来自哪个键盘，但每个事件都带着区分左右的设备相关标志位。Windows 风格键盘的修饰键从右侧到达，Mac 风格键盘的从左侧到达。所以当 MacBook 自带键盘是 Mac 风格、PC 键盘是 Windows 风格时，tap 只转换 PC 键盘的组合键，MacBook 保留 Command+Tab、Command+Left 和 Control+A。应用看到的是普通修饰键：macOS 的快捷键不区分左右。Positions 保留左侧，因为远端的 Windows 会把右 Alt 读成 AltGr。Mac 风格键盘自己右侧的键带着同样的标记：在标准应用里，它的右 Command 被读成 Windows 的 Ctrl，右 Option 被读成 Windows 的 Alt。这是已知的冲突。

**配置跟随应用。** 最前面应用的类别一变，HID 线程就重写每个 Windows 风格键盘的映射。映射变化时 macOS 会释放按住的键，所以不会有键卡住。没有运行中的键盘 tap 时不应用 Windows 配置（改用 Positions），因为那样就没有东西能阻止 Ctrl+Q 退出应用。

**布局检测。** 没有哪个 HID 属性说明修饰键在哪里。内置键盘和 Apple 键盘是 Mac；其他名字却带 Apple 厂商 id 的（Keychron 等仿品在 Mac 模式下用它）是 Mac，置信度中等；名字里有「Mac」的是 Mac；KVM 切换器模拟的键盘（制造商「TESmart」，厂商 0x1A86 的桥接芯片）是 PC，置信度低；其余都是 PC。键盘页面显示这个猜测，并允许用户纠正。

**带 Mac/Windows 开关的键盘。** Keychron 等借用 Apple 厂商 id 的键盘，以及以模式开关著称的品牌（Lofree、NuPhy、Akko、罗技多设备键盘等），由固件交换空格键旁边的键：Mac 模式下是 Command，Windows 模式下是 Alt。拨动开关不会告诉 macOS 任何信息（在 Keychron K8 上实测：id 不变、服务不变、不重新连接），所以对这些键盘来说布局就是模式，键盘页面显示为「键盘模式：Mac / Windows」。Isthmus 按一下键就能识别：暂时取消这把键盘的映射（`layout_probe_begin`，最多 20 秒），请用户按空格键左边紧挨着的键，读出它发出的是什么（Command 是 Mac 模式，Option/Alt 是 Windows 模式）。拨动开关之后，菜单栏里每把这样的键盘都有一个「Windows 模式」勾选项。

**功能键。** 报告 Apple 厂商 id 的键盘，不按住 Fn 时 F1–F12 发出媒体键。Windows 风格会在它们的服务上设置 `HIDFKeyMode`，让 F2、F5 和 F11 像在 Windows 上一样是功能键。

### 应用类别

| 类别 | 哪些应用（内置列表见 [`context.rs`](../crates/isthmus-input/src/context.rs)） | Ctrl | 快捷键组 |
| --- | --- | --- | --- |
| Standard（标准） | 其余所有应用 | 充当 Command | 全部 |
| Terminal（终端） | Terminal、iTerm2、Warp、Ghostty、kitty、WezTerm、Alacritty、Hyper、Tabby…… | 仍是 Ctrl（Ctrl+C 中断） | 窗口、开始菜单、系统、输入法、终端 |
| NativeCtrl | JetBrains IDE、Android Studio、Emacs、Vim、X11 | 仍是 Ctrl | 窗口、开始菜单、系统、输入法 |
| Passthrough（直通） | Parallels、VMware、UTM、VirtualBox、Windows App、Jump、RustDesk、TeamViewer、AnyDesk、Parsec、Moonlight、ToDesk、向日葵、CrossOver…… | 只做位置映射 | 无 |

用户自己的规则（bundle id 或 `前缀*`，加一个类别）优先于内置列表。Apple 的屏幕共享属于标准类别：对面是一台 Mac。

### 快捷键表

表中的「Ctrl」「Win」「Alt」指 Windows 的键，按当前配置从左右标志位解码得到（[`hidmap::decode`](../crates/isthmus-input/src/hidmap.rs)）。带有来自 Mac 风格键盘的修饰键的事件从不转换。从上到下，第一条匹配的规则生效。每次改写都保留事件中修饰键以外的标志位；改了键码的，作为一个新的按键事件发出；一个键的自动重复和抬起，得到它按下时得到的那种转换。

**守卫**（在标准应用里始终开启；Ctrl 是以 Command 到达的，这些规则让它不当作 Command）：

| Windows | 变成 | 原因 |
| --- | --- | --- |
| Ctrl+Tab、Ctrl+Shift+Tab | Control+Tab（+Shift） | Command+Tab 是应用切换器 |
| Ctrl+Space | 「上一个输入法」快捷键（Control+Space） | Command+Space 是聚焦搜索；Windows 的 Ctrl+Space 切换输入法 |
| Ctrl+Esc | 开启开始菜单组时打开聚焦搜索，否则 Control+Esc | macOS 26 起 Command+Esc 是游戏叠层 |
| Ctrl+H | 见编辑组；编辑组关闭时为 Control+H | Command+H 隐藏应用 |
| Ctrl+M、Ctrl+Q、Ctrl+Shift+Q、Ctrl+\`、Ctrl+Shift+\` | Control+同一键 | 最小化、退出、注销、窗口轮换 |
| Ctrl+Alt+任意键 | Control+Option+该键 | AltGr 组合；Command+Option+D/H/W/Space 是系统快捷键 |
| Ctrl+F1 … Ctrl+F20 | Control+功能键 | Command+F5 开关旁白 |
| Ctrl+Home / Ctrl+End（+Shift） | Command+Up / Command+Down（+Shift） | 文档开头和结尾 |
| Ctrl+Left / Ctrl+Right（+Shift） | Option+Left / Option+Right（+Shift） | 按词；浏览器里 Command+Left 是后退 |
| Ctrl+Up / Ctrl+Down（+Shift） | Option+Up / Option+Down（+Shift） | 按段落 |
| Ctrl+Backspace / Ctrl+Delete | Option+Backspace / Option+向前删除 | 按词 |

其他 Ctrl 组合都保持为 Command：Ctrl+C/V/X/Z/A/S/F/N/O/P/W/T/R/L/D/K/B/I/U、数字、`-` `=` `0` 缩放、`/`、Ctrl+Shift+T/N/P/Z、Ctrl+Return、Ctrl+点按（多选）。

**编辑**（`editing` 组）：

| Windows | 变成 | 说明 |
| --- | --- | --- |
| Ctrl+Y | Command+Shift+Z | Office：Command+Y |
| Ctrl+H | Command+Option+F（查找与替换） | 浏览器：Command+Y（历史记录）；Office：Control+H |
| Ctrl+F4 | Command+W | |
| F3 / Shift+F3 | Command+G / Command+Shift+G | VS Code：不变；访达：Command+F |
| Ctrl+PageDown / Ctrl+PageUp | Control+Tab / Control+Shift+Tab | VS Code：Command+Option+Right/Left |
| Ctrl+Insert / Shift+Insert | Command+C / Command+V | Insert 键以 Help 键到达 |

**导航**（`navigation` 组）：

| Windows | 变成 | 说明 |
| --- | --- | --- |
| Home / End（+Shift） | Command+Left / Command+Right（+Shift） | 浏览器除外，那里 Command+Left 是后退；macOS 的 Shift+Home 会一直选到文档开头 |

**窗口**（`windows` 组；终端和 NativeCtrl 应用里也生效）：

| Windows | 变成 |
| --- | --- |
| Alt+Tab、Alt+Shift+Tab | 应用切换器，按住 Alt 期间一直开着（见下文） |
| Alt+F4 | Command+W；浏览器和 VS Code：Command+Shift+W |
| Win+D | 显示桌面（快捷键 36） |
| Win+E | 在个人文件夹打开一个新的访达窗口 |
| Win+L | 锁定屏幕（Control+Command+Q） |
| Win+Tab | 调度中心（快捷键 32） |
| Win+Up / Win+Left / Win+Right | 填充 / 左侧拼贴 / 右侧拼贴（macOS 15 起的窗口拼贴快捷键） |
| Win+Down | Command+M（最小化） |
| Win+M | Command+Option+M（最小化该应用的所有窗口） |
| Win+Ctrl+Left / Win+Ctrl+Right | 上一个 / 下一个空间 |
| Win+I | 系统设置 |
| Win+R | 聚焦搜索 |
| Win+. 、Win+; | 表情与符号（Control+Command+Space） |

**开始菜单**（`startMenu` 组）：单独按下并在 300 ms 内松开 Win，中间没有其他按键、点按或滚轮，就打开聚焦搜索（快捷键 64）。

**系统**（`system` 组；终端和 NativeCtrl 应用里也生效）：

| Windows | 变成 |
| --- | --- |
| Ctrl+Shift+Esc | 活动监视器 |
| Ctrl+Alt+Delete（或 Backspace） | 强制退出（Command+Option+Esc） |
| PrintScreen、Win+Shift+S、Alt+PrintScreen | 区域截图到剪贴板（Control+Shift+Command+4） |
| Win+PrintScreen | 全屏截图到文件（Shift+Command+3） |

PC 键盘的 PrintScreen 以 F13 到达。

**输入法**（`inputSource` 组）：Win+Space 选择下一个输入法（快捷键 61）。Ctrl+Space 是一条守卫（见上文）。单独按下并在 500 毫秒内松开 Shift 会选择上一个输入法（快捷键 60），于是在输入法和 ABC 之间来回切换，和 Windows 输入法里单按 Shift 切换中英文一样；macOS 自己的方式是 Caps Lock。它有单独的设置（`keyboard.shiftSwitchesInput`，默认开启），在终端和 NativeCtrl 应用里同样生效，并且只在用户的输入法里有真正的输入法（拼音、微信输入法、Rime……）时才起作用（从 `com.apple.HIToolbox` 读取）：只有键盘布局时，单按 Shift 会在布局之间切换，而 Windows 从不这样。中间按了别的键、点了鼠标、滚了滚轮或按了别的修饰键都会取消；Shift 事件本身总是原样放行，所以 Shift+点击、大写字母和 IDE 的双击 Shift 都不受影响（双击 Shift 会切换两次，和 Windows 一样）。虚拟机里的 Shift 留给虚拟机里的输入法。如果输入法自己也开了 Shift 切换，会切换两次，请关掉其中一个。

**应用修正**（`appFixes` 组，只在标准应用里）：

| 应用 | Windows | 变成 |
| --- | --- | --- |
| 浏览器 | F5 / Shift+F5 / Ctrl+F5 | Command+R / Command+Shift+R / Command+Shift+R |
| | Alt+Left / Alt+Right | Command+[ / Command+] |
| | Alt+D、F6 | Command+L |
| | F11 | Control+Command+F（全屏） |
| | F12 | Command+Option+I |
| | Alt+Home | Command+Shift+H |
| VS Code 系列 | Ctrl+G、Ctrl+R、Ctrl+Shift+G、Ctrl+Q | Control+同一键（转到行、最近打开、源代码管理、视图选择器） |
| | Ctrl+Up / Ctrl+Down | Control+PageUp / Control+PageDown（按行滚动） |
| | Ctrl+Alt+Up / Ctrl+Alt+Down | Command+Option+Up / Down（添加光标） |
| | Ctrl+Shift+[ / ] | Command+Option+[ / ]（折叠） |
| | Alt+Left / Alt+Right | Control+- / Control+Shift+-（后退 / 前进） |
| Office | F12 | Command+Shift+S（另存为） |
| 访达 | F2 | Return（重命名） |
| | Alt+Up | Command+Up（上层文件夹） |
| | Alt+Left / Alt+Right | Command+[ / Command+] |
| | Alt+Return | Command+I（显示简介） |

**终端**（`terminal` 组，只在终端应用里；Ctrl 以 Control 到达）：

| Windows Terminal | 变成 |
| --- | --- |
| Ctrl+Shift+C、Ctrl+Insert | Command+C |
| Ctrl+Shift+V、Shift+Insert | Command+V |
| Ctrl+Shift+T / W / N / F | Command+T / W / N / F |
| Ctrl+= / Ctrl+- / Ctrl+0 | Command+= / - / 0 |
| Ctrl+Left / Ctrl+Right | Option+Left / Option+Right |
| Ctrl+Backspace | Control+W |

在终端和 NativeCtrl 应用里，Ctrl+Left/Right/Up/Down 否则会撞上 macOS 的空间和调度中心快捷键，所以不论快捷键组怎么设，它们都变成 Option+方向键。

### Alt+Tab

Command+Tab 的切换器只在按住 Command 时开着，所以把 Alt+Tab 一次性换成 Command+Tab 是不够的。按住 Alt 后第一次按 Tab 时，tap 发出 Command 按下（`flagsChanged`），并把这次 Tab 去掉 Option 后作为 Command+Tab 发出。会话期间，Tab、Shift、方向键、Esc、Q、H 和反引号键都带着 Command 发出。Alt 自己的抬起照常通过，随后 tap 发出 Command 抬起，切换器随之确认。Isthmus 从不自己发出 Alt 抬起，所以物理按键的状态始终真实：一个看门狗每 150 ms 读取一次，结束那些漏掉了松开事件的会话（安全输入、tap 被禁用）。

### 系统快捷键

显示桌面、调度中心、聚焦搜索、输入法切换、空间和窗口拼贴都是 macOS 的符号快捷键（symbolic hotkeys）。Isthmus 发出用户绑定的组合键（从 `com.apple.symbolichotkeys` 读取，否则用 macOS 默认值），对用户关掉的快捷键什么都不做。

## 鼠标

触控板和妙控鼠标从不被改动：只改逐格滚轮的事件（`kCGScrollWheelEventIsContinuous` 为 0，没有滚动阶段或惯性阶段）。每个事件都写明发出它的 HID 服务（字段 87），所以每只鼠标有自己的配置；未知的发送者使用默认配置。

| 设置 | Windows 预设 | 作用 |
| --- | --- | --- |
| 滚轮方向 | Windows | 滚轮向自己滚是向下翻，即使为触控板开了自然滚动。字段 137 表明 macOS 已经反转过事件时，tap 把两个轴都翻回来 |
| 线性滚轮、每格行数 | 开，3 | 每格滚动 N 行，没有加速：行增量 ±1，定点增量 ±N，像素增量 ±10·N（Chrome 和 Safari 滚动 40·N px，AppKit 10·N pt，VS Code 每格约 50 px） |
| 指针 | Windows，速度 10 | macOS 14 起的线性跟踪，增益取 Windows 1–20 档对应的值：即关闭「提高指针精确度」的 Windows。不支持线性时，关闭加速 |
| 侧键 | 开 | 4、5 号键发出后退 / 前进的轻扫。Chrome、Edge、Brave、Arc、Vivaldi、Chromium、VS Code、VSCodium、Cursor、Antigravity、JetBrains IDE 和 Android Studio 自己处理侧键，原样交给它们，直通类应用也一样；其余应用，包括 Safari、Firefox 和其他 Electron 应用，都得到轻扫。要让别的应用收到原始侧键，可以为这只鼠标关闭侧键，或给该应用一条直通规则（这也会停止它的键盘转换） |
| Ctrl+滚轮缩放 | 开 | 每格一次 Command+= / Command+-；Office 保留 Command+滚轮 |

增量按行、定点、像素的顺序写入：设置行增量会让 macOS 重新计算另外两个。

Windows 指针的增益会除以主显示器每个点对应的面板像素数，因为 Windows 按像素移动指针、macOS 按点移动：2560 像素的面板显示为 2048 点时这个值是 1.25，速度 10 就变成增益 0.8，于是通过 KVM 共用的鼠标在同一台显示器上划过的比例和在 Windows 上一样。

## 设置与一键切换

Isthmus 做什么，都设在一个文件里：`~/Library/Application Support/Isthmus/settings.json`（[`settings.rs`](../crates/isthmus-input/src/settings.rs)）：

| 部分 | 内容 |
| --- | --- |
| **总开关** | 关闭时恢复每一处系统改动并移除事件 tap（暂停）；开启时应用下面两部分。新安装时是关闭的。 |
| **键盘** | 自己的开关；没有单独条目的键盘用的风格；每个键盘的风格、布局（自动、Mac 或 PC）和功能键（标准，或按系统设置）；八个快捷键组；单按 Shift 是否切换输入法；用户自己的应用规则 |
| **鼠标** | 自己的开关；没有单独条目的鼠标用的配置；每只鼠标自己的配置：鼠标表里的各项设置，每格 1–12 行，速度 1–20 |

设备的条目在重新插拔、KVM 切换和重启后仍跟着它：它的 id 由传输方式、厂商、产品、种类和序列号组成，没有序列号时用 USB 端口（[`device.rs`](../crates/isthmus-input/src/device.rs)）。有单独条目的设备在断开时也会列出。

预设是一组值，不是模式。每个预设都能一键作用于键盘、鼠标或两者：

- **Windows** 让每个键盘都用 Windows 风格、标准功能键、开启所有快捷键组并打开单按 Shift 切换，让每只鼠标都用 Windows 配置，并打开总开关。
- **Mac** 让每个键盘都用 Mac 风格、功能键按系统设置并关闭单按 Shift 切换，让每只鼠标都用 Mac 配置：滚轮、指针和侧键保持 macOS 原样，关闭缩放。它不动总开关；两部分都是 Mac 时，Isthmus 什么都不改。

布局纠正和应用规则是关于用户硬件和应用的事实，不是风格，所以预设保留它们。鼠标预设会删掉每只鼠标的单独配置，让每只鼠标都跟随它。数值与两个预设都不符时，页面显示**自定义**。

之后的每个版本都会读这个文件：新字段带默认值，重写文件时保留不认识的键（在顶层、键盘和鼠标两部分、每个设备条目和每条应用规则里），本版本不认识的值按「什么都不改」的那个值读取。快捷键组和鼠标配置不保留不认识的键，所以之后的版本把新设置加在它们旁边，而不是里面。读不了的文件会报告出来，绝不被默认值替换。

## 系统改动与恢复

Isthmus 写入的每个 HID 属性都先记入日志 `~/Library/Application Support/Isthmus/journal.json`：哪个服务、哪个键、原始值（以第一次记下的原始值为准）以及 Isthmus 写入了什么。在退出（`applicationWillTerminate`、转交给 AppKit 的 SIGTERM）、暂停、切换到其他用户时，以及崩溃后的下次启动时，所有仍保持 Isthmus 写入状态的条目都会恢复；被别人改过的条目不动。上一次开机留下的日志直接丢弃：重启已经撤销了它。事件 tap 不需要日志：窗口服务器会丢掉已退出进程的 tap。`isthmus restore` 不启动其他任何东西就恢复日志；有正在运行的 Isthmus 持有锁时，它会拒绝。

如果 Isthmus 完全无法运行，拔下再插上一个设备会清除它在这个设备上设置的一切，重启 Mac 则清除全部。只清除键位映射可以在终端里执行：

```bash
hidutil property --set '{"UserKeyMapping":[]}'   # 所有键盘的映射，包括你自己的
```

## 需要了解的边界

- **没有辅助功能权限就没有 tap。** 没有这项权限的应用，macOS 不允许它用事件 tap 修改事件。这时键盘只有 Positions 映射（Win 和 Alt 回到 Windows 的位置，Ctrl 仍是 Control），鼠标只有指针速度。
- **安全输入会暂停 tap。** 密码框或终端的「安全键盘输入」开启安全输入时，tap 收不到按键事件。在密码框里 HID 映射仍然有效，所以 Ctrl+V 能粘贴，但 Home/End、Alt+Tab 和其他所有 tap 转换都停止，守卫也一样：这时 Ctrl+Q 就是 Command+Q。
- **安全键盘输入会让终端的复制粘贴失效。** 终端里 Ctrl 仍是 Control，所以开启它后，Ctrl+Shift+C 以 Control+Shift+C 到达终端，终端把它作为 ^C 发出，中断正在运行的命令；其他终端转换也都停止。Win+C 和 Win+V（Command）仍能复制粘贴。
- **KVM 切换等同于重新插拔。** 每次切换，HID 映射和指针属性随旧服务一起消失；新服务出现时 Isthmus 重新应用。
- **键码就是位置。** 规则按美式键盘的位置匹配，所以在德语键盘上，Ctrl+Z 是标着 Y 的那个键。
- **系统设置里的修饰键先生效。** macOS 先应用它们，再应用 Isthmus 的映射，所以两处都改过的键盘两者都会生效；概览页会提示这一点。
- **Karabiner-Elements 会独占键盘。** 被它接管的键盘收不到 Isthmus 的映射；概览页会提示。
- **需要 macOS 13 或更高版本。** 线性指针速度需要 macOS 14（更早的版本改为关闭加速），Win+方向键的窗口拼贴需要 macOS 15。
- **授权只需拖一下、点一下。** 「授权」直接打开系统设置的辅助功能面板（不弹系统自带的提示），并在设置窗口下方放一张小卡片，上面是 Isthmus 的图标：把它拖进列表，打开开关即可。卡片跟随设置窗口移动，检测到授权后自动消失。
- **授权后无需重启。** 键盘 tap 等待辅助功能权限时，Isthmus 监听 tccd 的变更通知，每秒读两次权限，只要不会弹窗的检查表明可能已允许，就立即重新安装 tap。如果授权已生效但 macOS 仍拒绝 tap，应用会自动重启一次；三分钟内再次被拒绝时，改为在概览页提示。
- **辅助功能授权跟着代码签名走。** ad-hoc 签名的开发构建每次重新构建 Rust 库都会失去授权；`tccutil reset Accessibility dev.acking.isthmus-local` 清掉失效的条目。
- **一台 Mac 只运行一个 Isthmus。** HID 状态属于整台机器；第二份副本（另一个构建，或 `isthmus run`）能运行，但什么都不改。
