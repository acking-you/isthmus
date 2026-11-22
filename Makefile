#!/bin/bash

# Build the macOS FFI library and stage it for Flutter (host arch only).
build-isthmus-ffi-macos:
	bash ./scripts/build/isthmus-ffi-macos.sh

# The same, unoptimized, for a faster development loop.
build-isthmus-ffi-macos-debug:
	PROFILE=debug bash ./scripts/build/isthmus-ffi-macos.sh

# Build the release app with the FFI library staged.
# Restricted to the host arch so the .app matches the host-only dylib;
# without this Flutter emits a universal binary whose other slice has no dylib.
MACOS_HOST_ARCH := $(shell uname -m)
build-ui-macos-local: export FLUTTER_XCODE_ISTHMUS_LOCAL_SUFFIX = -local
build-ui-macos-local: build-ui-macos-release
build-ui-macos-release: build-isthmus-ffi-macos
build-ui-macos-release build-ui-macos-release-no-ffi:
	cd ui && flutter pub get
	cd ui && FLUTTER_XCODE_ARCHS=$(MACOS_HOST_ARCH) \
		FLUTTER_XCODE_ONLY_ACTIVE_ARCH=YES \
		flutter build macos --release

# Build the Windows FFI library and stage it for Flutter (on Windows, with
# pwsh and the MSVC toolchain).
build-isthmus-ffi-windows:
	pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/build/isthmus-ffi-windows.ps1

# Build the Windows release app with the FFI library staged.
build-ui-windows-release: build-isthmus-ffi-windows
	cd ui && flutter pub get
	cd ui && flutter build windows --release

# Lint the Windows build, tests included, from any host (rustup target add
# x86_64-pc-windows-msvc once); every crate is pure Rust, so no MSVC is needed.
check-windows:
	cargo clippy --workspace --all-targets --all-features --target x86_64-pc-windows-msvc -- -D warnings

# The `isthmus` command.
build-isthmus-cli:
	cargo build -p isthmus-cli --release

.PHONY: \
	build-isthmus-ffi-macos \
	build-isthmus-ffi-macos-debug \
	build-ui-macos-local \
	build-ui-macos-release \
	build-ui-macos-release-no-ffi \
	build-isthmus-ffi-windows \
	build-ui-windows-release \
	check-windows \
	build-isthmus-cli
