# Isthmus docs

Each page owns one part of Isthmus: what it does, its limits and what a
change must keep. Start with [keyboard and mouse](keyboard-and-mouse.md);
contributor rules are in [AGENTS.md](../AGENTS.md). A page with a 中文 link
has a Chinese translation that changes with it.

## Use

| Page | Covers |
| --- | --- |
| [Keyboard and mouse](keyboard-and-mouse.md) · [中文](keyboard-and-mouse.zh-CN.md) | the three layers, keyboard styles and layouts, the HID profiles, app classes, every shortcut, Alt+Tab, the wheel and the pointer, settings and presets, the journal and putting things back, limits |
| [File sharing](file-sharing.md) · [中文](file-sharing.zh-CN.md) | finding other computers, pairing with a code, sending files, the cable-then-network paths, security, permissions and firewalls, the files it keeps, the protocol |
| [Updates and releases](updates.md) | how the app finds, checks and installs a new release, the update key, the Windows installer, and how a release is made |
| [Changelog](../CHANGELOG.md) | what each release changed |

## App

| Page | Covers |
| --- | --- |
| [Building the app](../ui/README.md) | the FFI boundary, the app's layers, the menu-bar Runner, building, running and checking |

## Project

| Page | Covers |
| --- | --- |
| [Architecture](architecture.md) | the code map: crates and modules, the layering, threads, the FFI and its ops, the app, the development loop |
| [Roadmap](roadmap.md) | what was planned and what is built: finding devices, pairing and file sharing (built), display alignment and keyboard and mouse across devices (planned) |
| [Rust engineering guide](rust-engineering-guide.md) | how Rust is written here: crates, types, errors, async, processes, wire formats, tests and lints |
